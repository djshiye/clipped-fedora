use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    rc::Rc,
};

use adw::{prelude::*, subclass::prelude::*};
use gtk::{CompositeTemplate, gio, glib};

use crate::{
    i18n::gettext,
    model::{GROUP_RECENT, Glyph, GlyphGroup},
    settings::settings,
};

const RECENT_MAX: usize = 24;

/// Called with the picked glyph.
pub type PickHandler = Rc<dyn Fn(&str)>;

mod imp {
    use super::*;

    #[derive(Default, CompositeTemplate)]
    #[template(resource = "/io/github/djshiye/Clipperino/ui/glyph_page.ui")]
    pub struct GlyphPage {
        #[template_child]
        pub search_entry: TemplateChild<gtk::SearchEntry>,
        #[template_child]
        pub groups: TemplateChild<adw::ToggleGroup>,
        #[template_child]
        pub grid: TemplateChild<gtk::GridView>,

        pub active_group: Rc<Cell<u32>>,
        pub recent: Rc<RefCell<Vec<String>>>,
        pub recent_key: RefCell<String>,
        pub filter: RefCell<Option<gtk::CustomFilter>>,
        pub on_activate: RefCell<Option<PickHandler>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for GlyphPage {
        const NAME: &'static str = "ClipperinoGlyphPage";
        type Type = super::GlyphPage;
        type ParentType = gtk::Box;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for GlyphPage {}
    impl WidgetImpl for GlyphPage {}
    impl BoxImpl for GlyphPage {}
}

glib::wrapper! {
    pub struct GlyphPage(ObjectSubclass<imp::GlyphPage>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl GlyphPage {
    /// `recent_key` is the GSettings string-array key that remembers recent picks.
    pub fn new(
        model: gio::ListStore,
        groups: &'static [GlyphGroup],
        placeholder: &str,
        recent_key: &str,
    ) -> Self {
        let page: Self = glib::Object::new();
        let imp = page.imp();
        imp.search_entry.set_placeholder_text(Some(placeholder));
        imp.recent_key.replace(recent_key.to_owned());
        imp.recent.replace(
            settings()
                .strv(recent_key)
                .iter()
                .map(|s| s.to_string())
                .collect(),
        );

        // Chips: Recent first, then the groups.
        // AdwToggle tooltips are Pango markup. libadwaita 1.9.4 frees an
        // uninitialised pointer when the markup is invalid (e.g. a bare "&"),
        // so every tooltip goes through markup_escape_text().
        let recent_toggle = adw::Toggle::new();
        recent_toggle.set_name(Some("recent"));
        recent_toggle.set_icon_name(Some("document-open-recent-symbolic"));
        recent_toggle.set_tooltip(&glib::markup_escape_text(&gettext("Recently Used")));
        imp.groups.add(recent_toggle);
        for g in groups {
            let toggle = adw::Toggle::new();
            toggle.set_name(Some(&g.group.to_string()));
            toggle.set_tooltip(&glib::markup_escape_text(&gettext(g.label)));
            match g.icon {
                Some(icon) => toggle.set_icon_name(Some(icon)),
                None => toggle.set_label(Some(g.chip)),
            }
            imp.groups.add(toggle);
        }
        let initial = if imp.recent.borrow().is_empty() {
            groups[0].group
        } else {
            GROUP_RECENT
        };
        imp.active_group.set(initial);
        imp.groups
            .set_active_name(Some(&if initial == GROUP_RECENT {
                "recent".to_owned()
            } else {
                initial.to_string()
            }));

        // Filter: search text wins; otherwise the active chip.
        let active_group = imp.active_group.clone();
        let recent = imp.recent.clone();
        let search = imp.search_entry.get();
        let filter = gtk::CustomFilter::new(move |obj| {
            let glyph = obj.downcast_ref::<Glyph>().unwrap();
            let query = search.text().to_lowercase();
            if !query.trim().is_empty() {
                return glyph.search_text().contains(query.trim());
            }
            match active_group.get() {
                GROUP_RECENT => recent.borrow().iter().any(|r| *r == glyph.glyph()),
                g => glyph.group() == g,
            }
        });
        let filter_model = gtk::FilterListModel::new(Some(model), Some(filter.clone()));
        let selection = gtk::NoSelection::new(Some(filter_model));

        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let label = gtk::Label::builder().css_classes(["glyph"]).build();
            item.set_child(Some(&label));
        });
        factory.connect_bind(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let label = item.child().and_downcast::<gtk::Label>().unwrap();
            let glyph = item.item().and_downcast::<Glyph>().unwrap();
            label.set_label(&glyph.glyph());
            label.set_tooltip_text(Some(&glyph.name()));
            label.update_property(&[gtk::accessible::Property::Label(&glyph.name())]);
        });
        imp.grid.set_model(Some(&selection));
        imp.grid.set_factory(Some(&factory));
        imp.grid.connect_activate(glib::clone!(
            #[weak]
            page,
            move |grid, pos| {
                if let Some(glyph) = grid
                    .model()
                    .and_then(|m| m.item(pos))
                    .and_downcast::<Glyph>()
                {
                    page.pick(&glyph.glyph());
                }
            }
        ));

        imp.groups.connect_active_name_notify(glib::clone!(
            #[weak]
            page,
            move |g| {
                let group = match g.active_name().as_deref() {
                    Some("recent") => GROUP_RECENT,
                    Some(n) => n.parse().unwrap_or(0),
                    None => return,
                };
                page.imp().active_group.set(group);
                page.refilter();
            }
        ));
        imp.search_entry.connect_search_changed(glib::clone!(
            #[weak]
            page,
            move |_| page.refilter()
        ));
        imp.search_entry.connect_activate(glib::clone!(
            #[weak]
            page,
            move |_| {
                // Enter picks the first match.
                if let Some(glyph) = page
                    .imp()
                    .grid
                    .model()
                    .and_then(|m| m.item(0))
                    .and_downcast::<Glyph>()
                {
                    page.pick(&glyph.glyph());
                }
            }
        ));
        imp.filter.replace(Some(filter));
        page
    }

    pub fn set_on_activate(&self, f: PickHandler) {
        self.imp().on_activate.replace(Some(f));
    }

    pub fn search_entry(&self) -> gtk::SearchEntry {
        self.imp().search_entry.get()
    }

    fn refilter(&self) {
        if let Some(f) = self.imp().filter.borrow().as_ref() {
            f.changed(gtk::FilterChange::Different);
        }
    }

    fn pick(&self, glyph: &str) {
        let imp = self.imp();
        {
            let mut recent = imp.recent.borrow_mut();
            recent.retain(|r| r != glyph);
            recent.insert(0, glyph.to_owned());
            recent.truncate(RECENT_MAX);
            let set: HashSet<&String> = recent.iter().collect();
            debug_assert_eq!(set.len(), recent.len());
            let strv: Vec<&str> = recent.iter().map(String::as_str).collect();
            settings().set_strv(&imp.recent_key.borrow(), strv).ok();
        }
        if imp.active_group.get() == GROUP_RECENT {
            self.refilter();
        }
        if let Some(f) = imp.on_activate.borrow().clone() {
            f(glyph);
        }
    }
}
