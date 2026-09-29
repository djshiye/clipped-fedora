use std::{cell::RefCell, collections::HashMap, rc::Rc};

use gtk::{gio, prelude::*};

use super::ClipItem;
use crate::storage::Storage;

/// The history's change listener, shared with the per-item pin watchers.
type OnChange = Rc<RefCell<Option<Rc<dyn Fn()>>>>;

/// A `gio::ListStore` of `ClipItem`s (newest first) with O(1) content dedupe.
pub struct HistoryStore {
    model: gio::ListStore,
    by_hash: RefCell<HashMap<[u8; 32], ClipItem>>,
    max_items: RefCell<u32>,
    // Shared with the per-item pin watchers, which look both up when they
    // fire: items restored at startup are watched before the tray registers.
    storage: Rc<RefCell<Option<Storage>>>,
    on_change: OnChange,
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
            storage: Rc::default(),
            on_change: Rc::default(),
        }
    }

    /// Called after any change (add, move, remove, clear, trim, pin).
    pub fn set_on_change(&self, f: Rc<dyn Fn()>) {
        self.on_change.replace(Some(f));
    }

    fn changed(&self) {
        if let Some(f) = self.on_change.borrow().clone() {
            f();
        }
    }

    pub fn find_by_hash(&self, hash: &[u8; 32]) -> Option<ClipItem> {
        self.by_hash.borrow().get(hash).cloned()
    }

    /// Pinned items first (newest first within each group), at most `n`.
    pub fn recent(&self, n: usize) -> Vec<ClipItem> {
        let all: Vec<ClipItem> = (0..self.model.n_items())
            .filter_map(|i| self.model.item(i).and_downcast::<ClipItem>())
            .collect();
        let mut out: Vec<ClipItem> = all.iter().filter(|i| i.pinned()).cloned().collect();
        out.extend(all.iter().filter(|i| !i.pinned()).cloned());
        out.truncate(n);
        out
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
        let storage = self.storage.clone();
        let on_change = self.on_change.clone();
        item.connect_pinned_notify(move |it| {
            if let Some(s) = storage.borrow().as_ref() {
                s.set_pinned(it.hash(), it.pinned());
            }
            let f = on_change.borrow().clone();
            if let Some(f) = f {
                f();
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
            self.changed();
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
        self.changed();
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
        self.changed();
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
        self.changed();
    }

    pub fn clear(&self) {
        self.model.remove_all();
        self.by_hash.borrow_mut().clear();
        if let Some(s) = self.storage.borrow().as_ref() {
            s.clear();
        }
        self.changed();
    }

    pub fn set_max_items(&self, max: u32) {
        *self.max_items.borrow_mut() = max.max(1);
        self.trim();
    }

    /// Drop unpinned items last used before `cutoff` (Unix seconds).
    /// Returns how many were removed.
    pub fn expire_before(&self, cutoff: i64) -> u32 {
        let mut removed = 0;
        let mut i = self.model.n_items();
        while i > 0 {
            i -= 1;
            let item = self.model.item(i).and_downcast::<ClipItem>().unwrap();
            if !item.pinned() && item.timestamp() < cutoff {
                self.model.remove(i);
                self.forget(&item);
                removed += 1;
            }
        }
        if removed > 0 {
            self.changed();
        }
        removed
    }

    /// Drop an item that is already out of the model, image file included.
    fn forget(&self, item: &ClipItem) {
        self.by_hash.borrow_mut().remove(&item.hash());
        if let Some(s) = self.storage.borrow().as_ref() {
            s.remove(item.hash(), item.image_path().map(Into::into));
        }
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
                self.forget(&item);
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

    #[test]
    fn expire_keeps_pinned_and_recent() {
        let s = HistoryStore::new(10);
        let old = s.add(ClipItem::new_text("old".into()));
        old.set_timestamp(100);
        let old_pinned = s.add(ClipItem::new_text("old pinned".into()));
        old_pinned.set_timestamp(100);
        old_pinned.set_pinned(true);
        let fresh = s.add(ClipItem::new_text("fresh".into()));
        fresh.set_timestamp(1000);
        assert_eq!(s.expire_before(500), 1);
        assert!(!s.contains_hash(&old.hash()));
        assert!(s.contains_hash(&old_pinned.hash()));
        assert!(s.contains_hash(&fresh.hash()));
    }

    #[test]
    fn pin_notifies_items_added_before_the_callback() {
        let s = store();
        let item = ClipItem::new_text("restored".into());
        s.append_restored(item.clone());
        let hits = Rc::new(std::cell::Cell::new(0));
        let h = hits.clone();
        s.set_on_change(Rc::new(move || h.set(h.get() + 1)));
        item.set_pinned(true);
        assert_eq!(hits.get(), 1);
    }
}
