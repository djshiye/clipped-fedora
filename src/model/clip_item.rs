use std::cell::{Cell, RefCell};

use gtk::{gdk, glib, prelude::*, subclass::prelude::*};
use unicode_segmentation::UnicodeSegmentation;

pub const PREVIEW_MAX_GRAPHEMES: usize = 120;

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq, glib::Enum)]
#[enum_type(name = "ClippedClipKind")]
pub enum ClipKind {
    #[default]
    Text,
    Image,
    Files,
}

mod imp {
    use super::*;

    #[derive(Default, glib::Properties)]
    #[properties(wrapper_type = super::ClipItem)]
    pub struct ClipItem {
        #[property(get, set)]
        pub id: Cell<i64>,
        #[property(get, set, builder(ClipKind::default()))]
        pub kind: Cell<ClipKind>,
        #[property(get, set, nullable)]
        pub text: RefCell<Option<String>>,
        #[property(get, set)]
        pub preview: RefCell<String>,
        #[property(get, set, nullable)]
        pub thumbnail: RefCell<Option<gdk::Texture>>,
        #[property(get, set, nullable)]
        pub image_path: RefCell<Option<String>>,
        /// Unix seconds.
        #[property(get, set)]
        pub timestamp: Cell<i64>,
        #[property(get, set)]
        pub pinned: Cell<bool>,
        /// blake3 of the raw content; not a GObject property.
        pub hash: RefCell<[u8; 32]>,
        /// Raw PNG bytes of a freshly captured image, handed to storage once
        /// and then dropped; afterwards the file at `image_path` is the source.
        pub png: RefCell<Option<Vec<u8>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ClipItem {
        const NAME: &'static str = "ClippedClipItem";
        type Type = super::ClipItem;
    }

    #[glib::derived_properties]
    impl ObjectImpl for ClipItem {}
}

glib::wrapper! {
    pub struct ClipItem(ObjectSubclass<imp::ClipItem>);
}

impl ClipItem {
    pub fn new_text(text: String) -> Self {
        let hash = *blake3::hash(text.as_bytes()).as_bytes();
        let preview = make_preview(&text);
        let item: Self = glib::Object::builder()
            .property("kind", ClipKind::Text)
            .property("preview", preview)
            .property("timestamp", now())
            .build();
        item.set_text(Some(text));
        item.imp().hash.replace(hash);
        item
    }

    /// A copied file list (text/uri-list). Stored as text, one URI per line.
    pub fn new_files(uris: &[String]) -> Self {
        let item = Self::new_text(uris.join("\n"));
        item.set_kind(ClipKind::Files);
        let names: Vec<String> = uris
            .iter()
            .map(|u| {
                glib::filename_from_uri(u)
                    .map(|(p, _)| {
                        p.file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default()
                    })
                    .unwrap_or_else(|_| u.clone())
            })
            .collect();
        item.set_preview(make_preview(&names.join(", ")));
        item
    }

    /// An image item. `width`/`height` are the full image size; the thumbnail
    /// is set later via `set_thumbnail` once decoded off the main thread.
    pub fn new_image(png: Vec<u8>, width: i32, height: i32) -> Self {
        let hash = *blake3::hash(&png).as_bytes();
        let item: Self = glib::Object::builder()
            .property("kind", ClipKind::Image)
            .property(
                "preview",
                crate::i18n::gettext("Image · {w} × {h}")
                    .replace("{w}", &width.to_string())
                    .replace("{h}", &height.to_string()),
            )
            .property("timestamp", now())
            .build();
        item.imp().hash.replace(hash);
        item.imp().png.replace(Some(png));
        item
    }

    /// Hand the captured PNG to whoever persists it; the item keeps none.
    pub fn take_png(&self) -> Option<Vec<u8>> {
        self.imp().png.take()
    }

    /// Rebuild an item from storage. Image thumbnails are loaded afterwards
    /// (see `HistoryStore::load_thumbnails`).
    pub fn restore(rec: &crate::storage::Record) -> Option<Self> {
        if rec.kind == ClipKind::Image && rec.image_path.as_ref().is_none_or(|p| !p.exists()) {
            return None;
        }
        let item: Self = glib::Object::builder()
            .property("kind", rec.kind)
            .property("preview", &rec.preview)
            .property("timestamp", rec.created)
            .property("pinned", rec.pinned)
            .build();
        item.set_text(rec.text.clone());
        item.set_image_path(
            rec.image_path
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
        );
        item.imp().hash.replace(rec.hash);
        Some(item)
    }

    pub fn to_record(&self) -> crate::storage::Record {
        crate::storage::Record {
            hash: self.hash(),
            kind: self.kind(),
            text: self.text(),
            preview: self.preview(),
            image_path: self.image_path().map(std::path::PathBuf::from),
            created: self.timestamp(),
            pinned: self.pinned(),
        }
    }

    pub fn hash(&self) -> [u8; 32] {
        *self.imp().hash.borrow()
    }
}

fn now() -> i64 {
    glib::DateTime::now_local()
        .map(|d| d.to_unix())
        .unwrap_or_default()
}

/// One line, at most `PREVIEW_MAX_GRAPHEMES` graphemes, whitespace collapsed.
/// Grapheme-aware, so a multi-byte character is never cut in half.
pub fn make_preview(text: &str) -> String {
    let collapsed: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut graphemes = collapsed.graphemes(true);
    let mut out: String = graphemes.by_ref().take(PREVIEW_MAX_GRAPHEMES).collect();
    if graphemes.next().is_some() {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_collapses_whitespace_and_truncates_by_grapheme() {
        let p = make_preview("a\n\n  b\tc");
        assert_eq!(p, "a b c");
        let long = "é".repeat(200);
        let p = make_preview(&long);
        assert_eq!(p.graphemes(true).count(), PREVIEW_MAX_GRAPHEMES + 1);
        assert!(p.ends_with('…'));
        assert!(std::str::from_utf8(p.as_bytes()).is_ok());
    }
}
