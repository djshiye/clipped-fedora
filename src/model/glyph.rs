use std::cell::{Cell, RefCell};

use gtk::{gio, glib, prelude::*, subclass::prelude::*};

/// Group index used for the "Recent" chip.
pub const GROUP_RECENT: u32 = u32::MAX;

mod imp {
    use super::*;

    #[derive(Default, glib::Properties)]
    #[properties(wrapper_type = super::Glyph)]
    pub struct Glyph {
        #[property(get, set)]
        pub glyph: RefCell<String>,
        #[property(get, set)]
        pub name: RefCell<String>,
        /// Lower-cased "name keyword keyword…" used by the search filter.
        #[property(get, set)]
        pub search_text: RefCell<String>,
        #[property(get, set)]
        pub group: Cell<u32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Glyph {
        const NAME: &'static str = "ClipperinoGlyph";
        type Type = super::Glyph;
    }

    #[glib::derived_properties]
    impl ObjectImpl for Glyph {}
}

glib::wrapper! {
    pub struct Glyph(ObjectSubclass<imp::Glyph>);
}

impl Glyph {
    pub fn new(glyph: &str, name: &str, keywords: &[String], group: u32) -> Self {
        let mut search = name.to_lowercase();
        for k in keywords {
            search.push(' ');
            search.push_str(&k.to_lowercase());
        }
        glib::Object::builder()
            .property("glyph", glyph)
            .property("name", name)
            .property("search-text", search)
            .property("group", group)
            .build()
    }
}

/// A category chip: the group index it selects, its full name (tooltip), and
/// either a symbolic icon or a single representative glyph shown on the chip.
pub struct GlyphGroup {
    pub group: u32,
    pub label: &'static str,
    pub icon: Option<&'static str>,
    pub chip: &'static str,
}

/// GTK's emoji sections (gtkemojichooser.c). Group 2 is skin-tone components.
pub const EMOJI_GROUPS: &[GlyphGroup] = &[
    GlyphGroup {
        group: 0,
        label: "Smileys & People",
        icon: Some("emoji-people-symbolic"),
        chip: "",
    },
    GlyphGroup {
        group: 1,
        label: "Body & Hands",
        icon: Some("emoji-body-symbolic"),
        chip: "",
    },
    GlyphGroup {
        group: 3,
        label: "Animals & Nature",
        icon: Some("emoji-nature-symbolic"),
        chip: "",
    },
    GlyphGroup {
        group: 4,
        label: "Food & Drink",
        icon: Some("emoji-food-symbolic"),
        chip: "",
    },
    GlyphGroup {
        group: 5,
        label: "Travel & Places",
        icon: Some("emoji-travel-symbolic"),
        chip: "",
    },
    GlyphGroup {
        group: 6,
        label: "Activities",
        icon: Some("emoji-activities-symbolic"),
        chip: "",
    },
    GlyphGroup {
        group: 7,
        label: "Objects",
        icon: Some("emoji-objects-symbolic"),
        chip: "",
    },
    GlyphGroup {
        group: 8,
        label: "Symbols",
        icon: Some("emoji-symbols-symbolic"),
        chip: "",
    },
    GlyphGroup {
        group: 9,
        label: "Flags",
        icon: Some("emoji-flags-symbolic"),
        chip: "",
    },
];

pub const SYMBOL_GROUPS: &[GlyphGroup] = &[
    GlyphGroup {
        group: 0,
        label: "Math",
        icon: None,
        chip: "∑",
    },
    GlyphGroup {
        group: 1,
        label: "Arrows",
        icon: None,
        chip: "→",
    },
    GlyphGroup {
        group: 2,
        label: "Currency",
        icon: None,
        chip: "€",
    },
    GlyphGroup {
        group: 3,
        label: "Punctuation",
        icon: None,
        chip: "¶",
    },
    GlyphGroup {
        group: 4,
        label: "Keyboard",
        icon: None,
        chip: "⌘",
    },
    GlyphGroup {
        group: 5,
        label: "Geometric",
        icon: None,
        chip: "◆",
    },
    GlyphGroup {
        group: 6,
        label: "Greek",
        icon: None,
        chip: "Ω",
    },
    GlyphGroup {
        group: 7,
        label: "Misc",
        icon: None,
        chip: "☯",
    },
];

/// Load GTK's emoji database, localized when GTK ships data for the current
/// language (/usr/share/gtk-4.0/emoji/<lang>.gresource), else English.
pub fn load_emoji() -> gio::ListStore {
    let store = gio::ListStore::new::<Glyph>();
    let bytes = localized_emoji_bytes().or_else(|| {
        gio::resources_lookup_data(
            "/org/gtk/libgtk/emoji/en.data",
            gio::ResourceLookupFlags::NONE,
        )
        .ok()
    });
    let Some(bytes) = bytes else {
        tracing::warn!("no emoji data found in GTK resources");
        return store;
    };
    let ty = glib::VariantTy::new("a(aussasasu)").unwrap();
    let variant = glib::Variant::from_bytes_with_type(&bytes, ty);
    let mut count = 0;
    for entry in variant.iter() {
        let group = entry.child_value(5).get::<u32>().unwrap_or(0);
        if group == 2 {
            continue; // skin-tone components
        }
        let codepoints = entry.child_value(0).get::<Vec<u32>>().unwrap_or_default();
        let text: String = codepoints
            .iter()
            .filter(|&&c| c != 0)
            .filter_map(|&c| char::from_u32(c))
            .collect();
        let name = entry.child_value(1).get::<String>().unwrap_or_default();
        let keywords = entry
            .child_value(3)
            .get::<Vec<String>>()
            .unwrap_or_default();
        store.append(&Glyph::new(&text, &name, &keywords, group));
        count += 1;
    }
    tracing::info!(count, "emoji loaded");
    store
}

fn localized_emoji_bytes() -> Option<glib::Bytes> {
    for lang in glib::language_names() {
        let lang = lang.split(['.', '@']).next().unwrap_or("");
        if lang.is_empty() || lang == "C" || lang == "en" {
            continue;
        }
        let path = format!("/usr/share/gtk-4.0/emoji/{lang}.gresource");
        if !std::path::Path::new(&path).exists() {
            continue;
        }
        if let Ok(res) = gio::Resource::load(&path) {
            gio::resources_register(&res);
            let data = format!("/org/gtk/libgtk/emoji/{lang}.data");
            if let Ok(bytes) = gio::resources_lookup_data(&data, gio::ResourceLookupFlags::NONE) {
                tracing::info!(lang, "using localized emoji names");
                return Some(bytes);
            }
        }
    }
    None
}

#[derive(serde::Deserialize)]
struct SymbolEntry {
    glyph: String,
    name: String,
    group: String,
}

/// Curated symbols from data/symbols.json (embedded).
pub fn load_symbols() -> gio::ListStore {
    let store = gio::ListStore::new::<Glyph>();
    let Ok(bytes) = gio::resources_lookup_data(
        &format!("{}/symbols.json", crate::config::RESOURCE_PREFIX),
        gio::ResourceLookupFlags::NONE,
    ) else {
        return store;
    };
    let entries: Vec<SymbolEntry> = serde_json::from_slice(&bytes).unwrap_or_default();
    for e in entries {
        let group = SYMBOL_GROUPS
            .iter()
            .find(|g| g.label == e.group)
            .map(|g| g.group)
            .unwrap_or(7);
        store.append(&Glyph::new(&e.glyph, &e.name, &[], group));
    }
    tracing::info!(count = store.n_items(), "symbols loaded");
    store
}
