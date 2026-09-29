use std::cell::{Cell, RefCell};

use gtk::{gdk, glib, prelude::*, subclass::prelude::*};
use unicode_segmentation::UnicodeSegmentation;

pub const PREVIEW_MAX_GRAPHEMES: usize = 120;

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq, glib::Enum)]
#[enum_type(name = "ClipperinoClipKind")]
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
        const NAME: &'static str = "ClipperinoClipItem";
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

    /// An image item. `hash` identifies the pixels (see `images::decode`), so
    /// the same picture copied from two apps is one entry. `width`/`height`
    /// are the full image size; the thumbnail is set by the caller.
    pub fn new_image(png: Vec<u8>, hash: [u8; 32], width: i32, height: i32) -> Self {
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

    /// The hash as hex: a stable key for menu actions, unlike a list position.
    pub fn key(&self) -> String {
        hex(&self.hash())
    }

    /// What search matches against: the preview (file names, image size)
    /// plus the start of the full text. Borrowed, so huge clips are not
    /// copied on every keystroke.
    pub fn search_text(&self) -> String {
        let text = self.imp().text.borrow();
        let mut out = self.preview();
        if let Some(t) = text.as_deref() {
            out.push('\n');
            out.push_str(prefix(t, SEARCH_MAX_BYTES));
        }
        out
    }
}

/// How much of a clip's text search looks at.
const SEARCH_MAX_BYTES: usize = 64 * 1024;

/// The longest prefix of `s` within `max` bytes, cut on a character boundary.
fn prefix(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Inverse of `ClipItem::key`.
pub fn parse_key(key: &str) -> Option<[u8; 32]> {
    if key.len() != 64 || !key.is_ascii() {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&key[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
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

    #[test]
    fn key_roundtrips() {
        let item = ClipItem::new_text("hello".into());
        assert_eq!(parse_key(&item.key()), Some(item.hash()));
        assert_eq!(parse_key("zz"), None);
        assert_eq!(parse_key(&"g".repeat(64)), None);
    }

    #[test]
    fn search_covers_text_beyond_the_preview() {
        let text = format!("{}needle", "word ".repeat(200));
        let item = ClipItem::new_text(text);
        assert!(!item.preview().contains("needle"));
        assert!(item.search_text().contains("needle"));
        let big = "é".repeat(SEARCH_MAX_BYTES);
        assert!(prefix(&big, SEARCH_MAX_BYTES - 1).len() < SEARCH_MAX_BYTES);
    }
}
