use std::{cell::RefCell, collections::HashMap};

use gtk::{gio, prelude::*};

use super::ClipItem;
use crate::storage::Storage;

/// A `gio::ListStore` of `ClipItem`s (newest first) with O(1) content dedupe.
pub struct HistoryStore {
    model: gio::ListStore,
    by_hash: RefCell<HashMap<[u8; 32], ClipItem>>,
    max_items: RefCell<u32>,
    storage: RefCell<Option<Storage>>,
}

impl Default for HistoryStore {
    fn default() -> Self {
        Self::new(100)
    }
}

impl HistoryStore {
    pub fn new(max_items: u32) -> Self {
        Self {
            model: gio::ListStore::new::<ClipItem>(),
            by_hash: RefCell::new(HashMap::new()),
            max_items: RefCell::new(max_items),
            storage: RefCell::new(None),
        }
    }

    /// Attach persistence. Items added afterwards are written through.
    pub fn set_storage(&self, storage: Storage) {
        self.storage.replace(Some(storage));
    }

    /// Put an already-persisted item at the end (used when loading).
    pub fn append_restored(&self, item: ClipItem) {
        self.watch_pinned(&item);
        self.by_hash.borrow_mut().insert(item.hash(), item.clone());
        self.model.append(&item);
    }

    fn watch_pinned(&self, item: &ClipItem) {
        let storage = self.storage.borrow().clone();
        item.connect_pinned_notify(move |it| {
            if let Some(s) = &storage {
                s.set_pinned(it.hash(), it.pinned());
            }
        });
    }

    pub fn model(&self) -> &gio::ListStore {
        &self.model
    }

    pub fn len(&self) -> u32 {
        self.model.n_items()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns `true` if the content was already the newest item (no change).
    pub fn contains_hash(&self, hash: &[u8; 32]) -> bool {
        self.by_hash.borrow().contains_key(hash)
    }

    /// Insert at the top. If an item with the same content exists it is moved
    /// to the top instead (its pin state and id are kept).
    pub fn add(&self, item: ClipItem) -> ClipItem {
        let hash = item.hash();
        if let Some(existing) = self.by_hash.borrow().get(&hash).cloned() {
            if let Some(pos) = self.position_of(&existing) {
                if pos == 0 {
                    return existing;
                }
                self.model.remove(pos);
            }
            existing.set_timestamp(item.timestamp());
            self.model.insert(0, &existing);
            if let Some(s) = self.storage.borrow().as_ref() {
                s.touch(hash, existing.timestamp());
            }
            return existing;
        }
        self.watch_pinned(&item);
        self.by_hash.borrow_mut().insert(hash, item.clone());
        self.model.insert(0, &item);
        if let Some(s) = self.storage.borrow().as_ref() {
            let png = item.take_png();
            if png.is_some() {
                item.set_image_path(Some(s.image_path_for(&hash).to_string_lossy().into_owned()));
            }
            s.upsert(item.to_record(), png);
        }
        self.trim();
        item
    }

    /// Decode thumbnails for restored image items on a worker thread and
    /// attach them as they arrive; rows update through the property binding.
    pub fn load_thumbnails(&self) {
        let pending: Vec<ClipItem> = (0..self.model.n_items())
            .filter_map(|i| self.model.item(i).and_downcast::<ClipItem>())
            .filter(|it| it.kind() == super::ClipKind::Image && it.thumbnail().is_none())
            .collect();
        for item in pending {
            let Some(path) = item.image_path() else {
                continue;
            };
            gtk::glib::spawn_future_local(async move {
                let result =
                    gtk::gio::spawn_blocking(move || super::images::thumbnail_from_file(&path))
                        .await;
                match result {
                    Ok(Ok(texture)) => item.set_thumbnail(Some(texture)),
                    Ok(Err(e)) => tracing::warn!("thumbnail failed: {e}"),
                    Err(_) => {}
                }
            });
        }
    }

    pub fn remove(&self, item: &ClipItem) -> Option<u32> {
        let pos = self.position_of(item)?;
        self.model.remove(pos);
        self.by_hash.borrow_mut().remove(&item.hash());
        if let Some(s) = self.storage.borrow().as_ref() {
            // Keep the image file: Undo may bring the row back. Orphans are
            // pruned when the database is next opened.
            s.remove(item.hash(), None);
        }
        Some(pos)
    }

    /// Re-insert a previously removed item at `pos` (for Undo).
    pub fn insert_at(&self, pos: u32, item: &ClipItem) {
        let pos = pos.min(self.model.n_items());
        self.by_hash.borrow_mut().insert(item.hash(), item.clone());
        self.model.insert(pos, item);
        if let Some(s) = self.storage.borrow().as_ref() {
            s.upsert(item.to_record(), None);
        }
    }

    pub fn clear(&self) {
        self.model.remove_all();
        self.by_hash.borrow_mut().clear();
        if let Some(s) = self.storage.borrow().as_ref() {
            s.clear();
        }
    }

    pub fn set_max_items(&self, max: u32) {
        *self.max_items.borrow_mut() = max.max(1);
        self.trim();
    }

    fn position_of(&self, item: &ClipItem) -> Option<u32> {
        self.model.find(item)
    }

    /// Drop the oldest unpinned items beyond the limit.
    fn trim(&self) {
        let max = *self.max_items.borrow();
        let mut i = self.model.n_items();
        while self.model.n_items() > max && i > 0 {
            i -= 1;
            let item = self.model.item(i).and_downcast::<ClipItem>().unwrap();
            if !item.pinned() {
                self.model.remove(i);
                self.by_hash.borrow_mut().remove(&item.hash());
                if let Some(s) = self.storage.borrow().as_ref() {
                    s.remove(item.hash(), item.image_path().map(Into::into));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> HistoryStore {
        HistoryStore::new(3)
    }

    #[test]
    fn dedupe_moves_to_top() {
        let s = store();
        s.add(ClipItem::new_text("a".into()));
        s.add(ClipItem::new_text("b".into()));
        s.add(ClipItem::new_text("a".into()));
        assert_eq!(s.len(), 2);
        let top = s.model().item(0).and_downcast::<ClipItem>().unwrap();
        assert_eq!(top.text().as_deref(), Some("a"));
    }

    #[test]
    fn trim_keeps_pinned() {
        let s = store();
        let p = s.add(ClipItem::new_text("pinned".into()));
        p.set_pinned(true);
        for t in ["1", "2", "3", "4"] {
            s.add(ClipItem::new_text(t.into()));
        }
        assert_eq!(s.len(), 3);
        assert!(s.contains_hash(&p.hash()));
    }
}
