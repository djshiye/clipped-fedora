//! SQLite-backed history. All writes happen on one worker thread through a
//! channel, so the UI thread never blocks on disk. Images live as PNG files
//! next to the database; the row stores the path.

use std::{
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
};

use rusqlite::{Connection, params};

use crate::model::ClipKind;

#[derive(Debug, Clone)]
pub struct Record {
    pub hash: [u8; 32],
    pub kind: ClipKind,
    pub text: Option<String>,
    pub preview: String,
    pub image_path: Option<PathBuf>,
    pub created: i64,
    pub pinned: bool,
}

enum Cmd {
    Upsert {
        record: Record,
        png: Option<Vec<u8>>,
    },
    Touch {
        hash: [u8; 32],
        created: i64,
    },
    SetPinned {
        hash: [u8; 32],
        pinned: bool,
    },
    Remove {
        hash: [u8; 32],
        image_path: Option<PathBuf>,
    },
    Clear,
    #[cfg(test)]
    Flush(mpsc::Sender<()>),
}

/// Cheap to clone; every clone talks to the same worker thread.
#[derive(Clone)]
pub struct Storage {
    tx: mpsc::Sender<Cmd>,
    images_dir: PathBuf,
}

impl Storage {
    /// Opens (or creates) the database under `dir`, loads every row newest
    /// first, and starts the writer thread.
    pub fn open(dir: &Path) -> rusqlite::Result<(Self, Vec<Record>)> {
        std::fs::create_dir_all(dir).ok();
        let images_dir = dir.join("images");
        std::fs::create_dir_all(&images_dir).ok();
        let db_path = dir.join("history.db");

        let mut conn = Connection::open(&db_path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS items (
                id         INTEGER PRIMARY KEY,
                hash       BLOB    NOT NULL UNIQUE,
                kind       INTEGER NOT NULL,
                text       TEXT,
                preview    TEXT    NOT NULL,
                image_path TEXT,
                created    INTEGER NOT NULL,
                pinned     INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS items_created ON items(created DESC, id DESC);",
        )?;
        let records = load_all(&mut conn)?;
        prune_orphans(&images_dir, &records);

        let (tx, rx) = mpsc::channel::<Cmd>();
        let worker_images = images_dir.clone();
        thread::Builder::new()
            .name("clipperino-storage".into())
            .spawn(move || worker(conn, rx, &worker_images))
            .expect("spawn storage thread");

        Ok((Self { tx, images_dir }, records))
    }

    pub fn image_path_for(&self, hash: &[u8; 32]) -> PathBuf {
        self.images_dir.join(format!("{}.png", hex(hash)))
    }

    pub fn upsert(&self, record: Record, png: Option<Vec<u8>>) {
        self.send(Cmd::Upsert { record, png });
    }

    pub fn touch(&self, hash: [u8; 32], created: i64) {
        self.send(Cmd::Touch { hash, created });
    }

    pub fn set_pinned(&self, hash: [u8; 32], pinned: bool) {
        self.send(Cmd::SetPinned { hash, pinned });
    }

    pub fn remove(&self, hash: [u8; 32], image_path: Option<PathBuf>) {
        self.send(Cmd::Remove { hash, image_path });
    }

    pub fn clear(&self) {
        self.send(Cmd::Clear);
    }

    #[cfg(test)]
    pub fn flush(&self) {
        let (tx, rx) = mpsc::channel();
        self.send(Cmd::Flush(tx));
        rx.recv().ok();
    }

    fn send(&self, cmd: Cmd) {
        if self.tx.send(cmd).is_err() {
            tracing::error!("storage thread is gone; change not persisted");
        }
    }
}

fn worker(conn: Connection, rx: mpsc::Receiver<Cmd>, images_dir: &Path) {
    for cmd in rx {
        let result = match cmd {
            Cmd::Upsert { record, png } => {
                let image_path = match (&record.kind, png) {
                    (ClipKind::Image, Some(bytes)) => {
                        let p = images_dir.join(format!("{}.png", hex(&record.hash)));
                        if let Err(e) = std::fs::write(&p, &bytes) {
                            tracing::warn!("could not write image {}: {e}", p.display());
                        }
                        Some(p)
                    }
                    _ => record.image_path.clone(),
                };
                conn.execute(
                    "INSERT INTO items (hash, kind, text, preview, image_path, created, pinned)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                     ON CONFLICT(hash) DO UPDATE SET created = excluded.created",
                    params![
                        record.hash.as_slice(),
                        record.kind as i64,
                        record.text,
                        record.preview,
                        image_path
                            .as_ref()
                            .map(|p| p.to_string_lossy().into_owned()),
                        record.created,
                        record.pinned as i64,
                    ],
                )
                .map(|_| ())
            }
            Cmd::Touch { hash, created } => conn
                .execute(
                    "UPDATE items SET created = ?2 WHERE hash = ?1",
                    params![hash.as_slice(), created],
                )
                .map(|_| ()),
            Cmd::SetPinned { hash, pinned } => conn
                .execute(
                    "UPDATE items SET pinned = ?2 WHERE hash = ?1",
                    params![hash.as_slice(), pinned as i64],
                )
                .map(|_| ()),
            Cmd::Remove { hash, image_path } => {
                if let Some(p) = image_path {
                    std::fs::remove_file(p).ok();
                }
                conn.execute(
                    "DELETE FROM items WHERE hash = ?1",
                    params![hash.as_slice()],
                )
                .map(|_| ())
            }
            Cmd::Clear => {
                if let Ok(rd) = std::fs::read_dir(images_dir) {
                    for e in rd.flatten() {
                        std::fs::remove_file(e.path()).ok();
                    }
                }
                conn.execute("DELETE FROM items", []).map(|_| ())
            }
            #[cfg(test)]
            Cmd::Flush(done) => {
                done.send(()).ok();
                Ok(())
            }
        };
        if let Err(e) = result {
            tracing::error!("storage write failed: {e}");
        }
    }
}

fn load_all(conn: &mut Connection) -> rusqlite::Result<Vec<Record>> {
    let mut stmt = conn.prepare(
        "SELECT hash, kind, text, preview, image_path, created, pinned
         FROM items ORDER BY created DESC, id DESC",
    )?;
    let rows = stmt.query_map([], |r| {
        let hash: Vec<u8> = r.get(0)?;
        let mut h = [0u8; 32];
        if hash.len() == 32 {
            h.copy_from_slice(&hash);
        }
        Ok(Record {
            hash: h,
            kind: match r.get::<_, i64>(1)? {
                1 => ClipKind::Image,
                2 => ClipKind::Files,
                _ => ClipKind::Text,
            },
            text: r.get(2)?,
            preview: r.get(3)?,
            image_path: r.get::<_, Option<String>>(4)?.map(PathBuf::from),
            created: r.get(5)?,
            pinned: r.get::<_, i64>(6)? != 0,
        })
    })?;
    rows.collect()
}

/// Delete image files no row references (left behind by deletes whose Undo expired).
fn prune_orphans(images_dir: &Path, records: &[Record]) {
    let referenced: std::collections::HashSet<PathBuf> = records
        .iter()
        .filter_map(|r| r.image_path.clone())
        .collect();
    let Ok(rd) = std::fs::read_dir(images_dir) else {
        return;
    };
    let mut removed = 0;
    for entry in rd.flatten() {
        let p = entry.path();
        if p.extension().is_some_and(|e| e == "png")
            && !referenced.contains(&p)
            && std::fs::remove_file(&p).is_ok()
        {
            removed += 1;
        }
    }
    if removed > 0 {
        tracing::info!(removed, "pruned orphan image files");
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// One-time import of the legacy `clipman` text history (`---ENTRY---`
/// separated). Returns the entries oldest first. The file is renamed after.
pub fn import_legacy_history() -> Vec<String> {
    let path = gtk::glib::home_dir().join(".local/share/clipman/history.txt");
    let Ok(content) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let mut entries: Vec<String> = content
        .split("---ENTRY---\n")
        .map(|e| e.trim_end_matches('\n').to_owned())
        .filter(|e| !e.trim().is_empty())
        .collect();
    // Legacy file is newest first; we add oldest first so newest ends on top.
    entries.reverse();
    std::fs::rename(&path, path.with_extension("txt.imported")).ok();
    tracing::info!(count = entries.len(), "imported legacy clipman history");
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_remove() {
        let dir = std::env::temp_dir().join(format!("clipperino-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (s, initial) = Storage::open(&dir).unwrap();
        assert!(initial.is_empty());
        let h1 = [1u8; 32];
        let h2 = [2u8; 32];
        s.upsert(
            Record {
                hash: h1,
                kind: ClipKind::Text,
                text: Some("one".into()),
                preview: "one".into(),
                image_path: None,
                created: 10,
                pinned: false,
            },
            None,
        );
        s.upsert(
            Record {
                hash: h2,
                kind: ClipKind::Image,
                text: None,
                preview: "img".into(),
                image_path: None,
                created: 20,
                pinned: true,
            },
            Some(vec![0x89, 0x50, 0x4e, 0x47]),
        );
        s.touch(h1, 30);
        s.flush();
        let (s2, rows) = Storage::open(&dir).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].hash, h1, "touched row is newest");
        assert!(rows[1].pinned);
        assert!(rows[1].image_path.as_ref().unwrap().exists());
        s2.remove(h2, None);
        s2.flush();
        assert!(s2.image_path_for(&h2).exists(), "file kept for Undo");
        let (_, rows) = Storage::open(&dir).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(!s2.image_path_for(&h2).exists(), "orphan pruned on open");
        std::fs::remove_dir_all(&dir).ok();
    }
}
