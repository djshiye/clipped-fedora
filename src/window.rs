use std::{cell::RefCell, rc::Rc};

use adw::{prelude::*, subclass::prelude::*};
use gtk::{CompositeTemplate, gio, glib};

use crate::{
    i18n::gettext,
    model::{ClipItem, ClipKind, HistoryStore},
    platform::portal::{ClipEvent, Offer, PortalSession},
    settings::{self, settings},
    ui::{DetailDialog, GlyphPage, HistoryRow, ImageTile},
};

mod imp {
    use super::*;

    #[derive(Default, CompositeTemplate)]
    #[template(resource = "/io/github/djshiye/Clipperino/ui/window.ui")]
    pub struct ClipperinoWindow {
        #[template_child]
        pub stack: TemplateChild<adw::ViewStack>,
        #[template_child]
        pub switcher_bar: TemplateChild<adw::ViewSwitcherBar>,
        #[template_child]
        pub search_entry: TemplateChild<gtk::SearchEntry>,
        #[template_child]
        pub history_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub history_list: TemplateChild<gtk::ListView>,
        #[template_child]
        pub gallery: TemplateChild<gtk::GridView>,
        #[template_child]
        pub kind_chips: TemplateChild<adw::ToggleGroup>,
        #[template_child]
        pub no_results_page: TemplateChild<adw::StatusPage>,
        #[template_child]
        pub grant_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub toast_overlay: TemplateChild<adw::ToastOverlay>,
        #[template_child]
        pub emoji_bin: TemplateChild<adw::Bin>,
        #[template_child]
        pub symbols_bin: TemplateChild<adw::Bin>,
        #[template_child]
        pub preview_pane: TemplateChild<gtk::Box>,
        #[template_child]
        pub preview_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub preview_text: TemplateChild<gtk::Label>,
        #[template_child]
        pub preview_picture: TemplateChild<gtk::Picture>,
        #[template_child]
        pub preview_meta: TemplateChild<gtk::Label>,
        #[template_child]
        pub paused_banner: TemplateChild<adw::Banner>,

        pub history: HistoryStore,
        pub filter: RefCell<Option<gtk::StringFilter>>,
        pub filter_model: RefCell<Option<gtk::FilterListModel>>,
        pub selection: RefCell<Option<gtk::SingleSelection>>,
        pub preview_binding: RefCell<Option<glib::Binding>>,
        pub portal: Rc<RefCell<Option<Rc<PortalSession>>>>,
        pub clock: RefCell<Option<glib::SourceId>>,
        /// Which kind the chips show; `None` is All.
        pub kind: Rc<std::cell::Cell<Option<ClipKind>>>,
        pub kind_filter: RefCell<Option<gtk::CustomFilter>>,
        /// Local midnight the sections are computed against.
        pub today: Rc<std::cell::Cell<i64>>,
        pub section_sorter: RefCell<Option<gtk::CustomSorter>>,
        /// Bumped per preview; a full-size load that finishes late is dropped.
        pub preview_generation: std::cell::Cell<u64>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ClipperinoWindow {
        const NAME: &'static str = "ClipperinoWindow";
        type Type = super::ClipperinoWindow;
        type ParentType = adw::ApplicationWindow;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for ClipperinoWindow {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.setup_list();
            obj.setup_search();
            obj.setup_actions();
            obj.setup_keys();
            obj.load_history();
            obj.setup_glyph_pages();
            obj.setup_settings();
            obj.setup_clock();
            obj.setup_expiry();
            obj.update_empty_state();
        }
    }
    impl WidgetImpl for ClipperinoWindow {}
    impl WindowImpl for ClipperinoWindow {}
    impl ApplicationWindowImpl for ClipperinoWindow {}
    impl AdwApplicationWindowImpl for ClipperinoWindow {}
}

glib::wrapper! {
    pub struct ClipperinoWindow(ObjectSubclass<imp::ClipperinoWindow>)
        @extends adw::ApplicationWindow, gtk::ApplicationWindow, gtk::Window, gtk::Widget,
        @implements gio::ActionGroup, gio::ActionMap, gtk::Accessible, gtk::Buildable,
                    gtk::ConstraintTarget, gtk::Native, gtk::Root, gtk::ShortcutManager;
}

impl ClipperinoWindow {
    pub fn new(app: &impl IsA<gtk::Application>) -> Self {
        glib::Object::builder().property("application", app).build()
    }

    pub fn history(&self) -> &HistoryStore {
        &self.imp().history
    }

    fn setup_list(&self) {
        let imp = self.imp();

        // history (ListStore)
        //   -> filter: search text AND kind chip
        //   -> sections by day: Today, Yesterday, Last 7 Days, Earlier
        //   -> SingleSelection, shared by the list and the Images gallery
        let expr = gtk::ClosureExpression::with_callback(&[] as &[gtk::Expression], |args| {
            args[0]
                .get::<ClipItem>()
                .map(|item| item.search_text())
                .unwrap_or_default()
        });
        let filter = gtk::StringFilter::builder()
            .expression(&expr)
            .ignore_case(true)
            .match_mode(gtk::StringFilterMatchMode::Substring)
            .build();
        let kind = imp.kind.clone();
        let kind_filter = gtk::CustomFilter::new(move |obj| {
            let wanted = kind.get();
            obj.downcast_ref::<ClipItem>()
                .is_some_and(|item| wanted.is_none_or(|k| item.kind() == k))
        });
        let every = gtk::EveryFilter::new();
        every.append(filter.clone());
        every.append(kind_filter.clone());
        let filter_model =
            gtk::FilterListModel::new(Some(imp.history.model().clone()), Some(every));

        imp.today.set(crate::ui::today_start());
        let today = imp.today.clone();
        let section_of = move |obj: &glib::Object| {
            let item = obj
                .downcast_ref::<ClipItem>()
                .expect("history holds ClipItems");
            crate::ui::Section::of(item.timestamp(), today.get())
        };
        // Stable: within a section, items keep their newest-first order.
        let section_sorter =
            gtk::CustomSorter::new(move |a, b| section_of(a).cmp(&section_of(b)).into());
        let sorted = gtk::SortListModel::new(Some(filter_model.clone()), None::<gtk::Sorter>);
        sorted.set_section_sorter(Some(&section_sorter));

        // Always keep a selected row so Enter in the search field has a target.
        let selection = gtk::SingleSelection::builder()
            .model(&sorted)
            .autoselect(true)
            .can_unselect(false)
            .build();

        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            item.set_child(Some(&HistoryRow::default()));
        });
        factory.connect_bind(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let row = item.child().and_downcast::<HistoryRow>().unwrap();
            let clip = item.item().and_downcast::<ClipItem>().unwrap();
            row.bind(&clip);
        });
        factory.connect_unbind(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            if let Some(row) = item.child().and_downcast::<HistoryRow>() {
                row.unbind();
            }
        });

        let headers = gtk::SignalListItemFactory::new();
        headers.connect_setup(|_, header| {
            let header = header.downcast_ref::<gtk::ListHeader>().unwrap();
            let label = gtk::Label::builder()
                .xalign(0.0)
                .css_classes(["clip-section"])
                .build();
            header.set_child(Some(&label));
        });
        let today = imp.today.clone();
        headers.connect_bind(move |_, header| {
            let header = header.downcast_ref::<gtk::ListHeader>().unwrap();
            let (Some(label), Some(item)) = (
                header.child().and_downcast::<gtk::Label>(),
                header.item().and_downcast::<ClipItem>(),
            ) else {
                return;
            };
            let section = crate::ui::Section::of(item.timestamp(), today.get());
            label.set_label(&section.title());
        });

        let tiles = gtk::SignalListItemFactory::new();
        tiles.connect_setup(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            item.set_child(Some(&ImageTile::default()));
        });
        tiles.connect_bind(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let tile = item.child().and_downcast::<ImageTile>().unwrap();
            let clip = item.item().and_downcast::<ClipItem>().unwrap();
            tile.bind(&clip);
        });
        tiles.connect_unbind(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            if let Some(tile) = item.child().and_downcast::<ImageTile>() {
                tile.unbind();
            }
        });

        imp.history_list.set_model(Some(&selection));
        imp.history_list.set_factory(Some(&factory));
        imp.history_list.set_header_factory(Some(&headers));
        imp.gallery.set_model(Some(&selection));
        imp.gallery.set_factory(Some(&tiles));
        let on_activate = glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |pos: u32| {
                if let Some(clip) = win.item_at(pos) {
                    win.activate_item(&clip);
                }
            }
        );
        let activate = on_activate.clone();
        imp.history_list
            .connect_activate(move |_, pos| activate(pos));
        imp.gallery.connect_activate(move |_, pos| on_activate(pos));

        filter_model.connect_items_changed(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |_, _, _, _| win.update_empty_state()
        ));

        // Widening the window reveals the pane: load the full-size image then.
        imp.preview_pane.connect_visible_notify(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |pane| {
                if pane.is_visible() {
                    let item = win.selected_item().map(|(_, i)| i);
                    win.update_preview(item.as_ref());
                }
            }
        ));

        selection.connect_selected_item_notify(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |sel| win.update_preview(sel.selected_item().and_downcast::<ClipItem>().as_ref())
        ));

        imp.kind_chips.connect_active_name_notify(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |chips| {
                let kind = match chips.active_name().as_deref() {
                    Some("text") => Some(ClipKind::Text),
                    Some("images") => Some(ClipKind::Image),
                    Some("files") => Some(ClipKind::Files),
                    _ => None,
                };
                let imp = win.imp();
                imp.kind.set(kind);
                if let Some(f) = imp.kind_filter.borrow().as_ref() {
                    f.changed(gtk::FilterChange::Different);
                }
                win.update_empty_state();
                // A new view starts at its newest item.
                if let Some(sel) = imp.selection.borrow().as_ref()
                    && sel.n_items() > 0
                {
                    sel.set_selected(0);
                    win.scroll_to(0);
                }
            }
        ));

        imp.filter.replace(Some(filter));
        imp.kind_filter.replace(Some(kind_filter));
        imp.filter_model.replace(Some(filter_model));
        imp.section_sorter.replace(Some(section_sorter));
        imp.selection.replace(Some(selection));
    }

    /// Re-sort into day sections at midnight, keeping `keep` selected if it was.
    fn resort(&self, keep: Option<&ClipItem>) {
        let imp = self.imp();
        let was_selected = self
            .selected_item()
            .is_some_and(|(_, sel)| keep.is_some_and(|k| *k == sel));
        if let Some(sorter) = imp.section_sorter.borrow().as_ref() {
            sorter.changed(gtk::SorterChange::Different);
        }
        let (Some(item), true) = (keep, was_selected) else {
            return;
        };
        let Some(sel) = imp.selection.borrow().clone() else {
            return;
        };
        if let Some(pos) = (0..sel.n_items()).find(|&i| {
            sel.item(i)
                .and_downcast::<ClipItem>()
                .is_some_and(|it| it == *item)
        }) {
            sel.set_selected(pos);
            self.scroll_to(pos);
        }
    }

    fn gallery_shown(&self) -> bool {
        self.imp().history_stack.visible_child_name().as_deref() == Some("gallery")
    }

    fn scroll_to(&self, pos: u32) {
        let imp = self.imp();
        // Position 0: scroll fully up, so its section header shows too.
        if pos == 0 {
            let view: gtk::Widget = if self.gallery_shown() {
                imp.gallery.get().upcast()
            } else {
                imp.history_list.get().upcast()
            };
            if let Some(sw) = view
                .ancestor(gtk::ScrolledWindow::static_type())
                .and_downcast::<gtk::ScrolledWindow>()
            {
                sw.vadjustment().set_value(0.0);
                return;
            }
        }
        if self.gallery_shown() {
            imp.gallery.scroll_to(pos, gtk::ListScrollFlags::NONE, None);
        } else {
            imp.history_list
                .scroll_to(pos, gtk::ListScrollFlags::NONE, None);
        }
    }

    /// Wide layout: the pane beside the list mirrors the selected item.
    fn update_preview(&self, item: Option<&ClipItem>) {
        let imp = self.imp();
        if let Some(b) = imp.preview_binding.take() {
            b.unbind();
        }
        let Some(item) = item else {
            imp.preview_stack.set_visible_child_name("none");
            return;
        };
        match item.kind() {
            ClipKind::Image => {
                // The thumbnail shows at once (bound: restored thumbnails arrive
                // asynchronously); the full-size image replaces it when loaded,
                // since the pane is far larger than a thumbnail.
                let b = item
                    .bind_property("thumbnail", &*imp.preview_picture, "paintable")
                    .sync_create()
                    .build();
                imp.preview_binding.replace(Some(b));
                imp.preview_stack.set_visible_child_name("image");
                self.load_full_preview(item);
            }
            _ => {
                let text = item.text().unwrap_or_default();
                let shown: String = text.chars().take(4000).collect();
                imp.preview_text.set_label(&shown);
                if crate::ui::looks_like_code(&text) {
                    imp.preview_text.add_css_class("monospace");
                } else {
                    imp.preview_text.remove_css_class("monospace");
                }
                imp.preview_stack.set_visible_child_name("text");
            }
        }
        if item.kind() != ClipKind::Image {
            imp.preview_generation.set(imp.preview_generation.get() + 1);
        }
        imp.preview_meta.set_label(&self.preview_caption(item));
    }

    fn load_full_preview(&self, item: &ClipItem) {
        let imp = self.imp();
        let generation = imp.preview_generation.get() + 1;
        imp.preview_generation.set(generation);
        // Only the wide layout shows the pane; skip the decode otherwise.
        let (Some(path), true) = (item.image_path(), imp.preview_pane.is_visible()) else {
            return;
        };
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = win)]
            self,
            async move {
                let loaded =
                    gio::spawn_blocking(move || crate::model::images::preview_from_file(&path))
                        .await;
                let imp = win.imp();
                if imp.preview_generation.get() != generation {
                    return;
                }
                if let Ok(Ok(texture)) = loaded {
                    if let Some(b) = imp.preview_binding.take() {
                        b.unbind();
                    }
                    imp.preview_picture.set_paintable(Some(&texture));
                }
            }
        ));
    }

    /// "5 min ago · 120 characters"
    fn preview_caption(&self, item: &ClipItem) -> String {
        let when = crate::ui::relative_time(item.timestamp());
        let size = match item.kind() {
            ClipKind::Image => item.preview(),
            _ => {
                let text = item.text().unwrap_or_default();
                gettext("{chars} characters").replace("{chars}", &text.chars().count().to_string())
            }
        };
        format!("{when} · {size}")
    }

    fn setup_glyph_pages(&self) {
        let imp = self.imp();
        let on_pick: std::rc::Rc<dyn Fn(&str)> = std::rc::Rc::new(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |text| win.activate_item(&ClipItem::new_text(text.to_owned()))
        ));
        let emoji = GlyphPage::new(
            crate::model::load_emoji(),
            crate::model::EMOJI_GROUPS,
            &gettext("Search emoji"),
            "recent-emoji",
        );
        emoji.set_on_activate(on_pick.clone());
        imp.emoji_bin.set_child(Some(&emoji));
        let symbols = GlyphPage::new(
            crate::model::load_symbols(),
            crate::model::SYMBOL_GROUPS,
            &gettext("Search symbols"),
            "recent-symbols",
        );
        symbols.set_on_activate(on_pick);
        imp.symbols_bin.set_child(Some(&symbols));

        // Focus the visible page's search field when switching pages.
        imp.stack.connect_visible_child_name_notify(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |_| win.focus_search()
        ));
    }

    fn setup_settings(&self) {
        let s = settings();
        let history = &self.imp().history;
        history.set_max_items(s.uint(settings::MAX_HISTORY));
        s.connect_changed(
            Some(settings::MAX_HISTORY),
            glib::clone!(
                #[weak(rename_to = win)]
                self,
                move |s, key| {
                    win.imp().history.set_max_items(s.uint(key));
                    win.update_empty_state();
                }
            ),
        );
        s.bind(
            settings::PAUSE_RECORDING,
            &*self.imp().paused_banner,
            "revealed",
        )
        .get()
        .build();
        let (w, h) = (
            s.int(settings::WINDOW_WIDTH),
            s.int(settings::WINDOW_HEIGHT),
        );
        self.set_default_size(w, h);
        tracing::debug!(w, h, applied = ?self.default_size(), "window size restored");
        self.connect_map(|win| {
            let win = win.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(300), move || {
                tracing::debug!(width = win.width(), height = win.height(), "window mapped");
                // Debug builds: CLIPPERINO_DEBUG_SIZE=WxH forces a size after map, for
                // layout checks on desktops whose extensions restore geometry.
                #[cfg(debug_assertions)]
                if let Some((w, h)) = std::env::var("CLIPPERINO_DEBUG_SIZE").ok().and_then(|v| {
                    let (a, b) = v.split_once('x')?;
                    Some((a.parse().ok()?, b.parse().ok()?))
                }) {
                    win.set_default_size(w, h);
                }
            });
        });
        self.connect_close_request(|win| {
            win.hide_window();
            glib::Propagation::Stop
        });
    }

    /// While the window is shown, keep the preview's "5 min ago" current and
    /// move clips to Yesterday at midnight; check at once on show.
    fn setup_clock(&self) {
        self.connect_map(|win| {
            win.refresh_times();
            // Open on the newest clip, scrolled fully up so its day header
            // shows; after layout, since the list sizes itself on map.
            glib::idle_add_local_once(glib::clone!(
                #[weak]
                win,
                move || {
                    let sel = win.imp().selection.borrow().clone();
                    if let Some(sel) = sel
                        && sel.n_items() > 0
                        && win.imp().search_entry.text().is_empty()
                    {
                        sel.set_selected(0);
                        win.scroll_to(0);
                    }
                }
            ));
            let id = glib::timeout_add_seconds_local(
                30,
                glib::clone!(
                    #[weak]
                    win,
                    #[upgrade_or]
                    glib::ControlFlow::Break,
                    move || {
                        win.refresh_times();
                        glib::ControlFlow::Continue
                    }
                ),
            );
            if let Some(old) = win.imp().clock.replace(Some(id)) {
                old.remove();
            }
        });
        self.connect_unmap(|win| {
            if let Some(id) = win.imp().clock.take() {
                id.remove();
            }
        });
    }

    fn refresh_times(&self) {
        let imp = self.imp();
        let today = crate::ui::today_start();
        if imp.today.replace(today) != today {
            self.resort(self.selected_item().map(|(_, i)| i).as_ref());
        }
        // Only the time in the preview caption; re-setting the text would
        // drop the user's selection in it.
        if let Some((_, item)) = self.selected_item() {
            imp.preview_meta.set_label(&self.preview_caption(&item));
        }
    }

    /// Delete unpinned clips older than the "expire-days" setting: at startup,
    /// hourly, and when the setting changes.
    fn setup_expiry(&self) {
        self.expire_old();
        settings().connect_changed(
            Some(settings::EXPIRE_DAYS),
            glib::clone!(
                #[weak(rename_to = win)]
                self,
                move |_, _| win.expire_old()
            ),
        );
        glib::timeout_add_seconds_local(
            3600,
            glib::clone!(
                #[weak(rename_to = win)]
                self,
                #[upgrade_or]
                glib::ControlFlow::Break,
                move || {
                    win.expire_old();
                    glib::ControlFlow::Continue
                }
            ),
        );
    }

    fn expire_old(&self) {
        let days = settings().uint(settings::EXPIRE_DAYS);
        if days == 0 {
            return;
        }
        let now = glib::DateTime::now_local()
            .map(|d| d.to_unix())
            .unwrap_or_default();
        let removed = self
            .imp()
            .history
            .expire_before(now - i64::from(days) * 86_400);
        if removed > 0 {
            tracing::info!(removed, days, "expired old clips");
            self.update_empty_state();
        }
    }

    pub fn clear_history(&self) {
        self.imp().history.clear();
        self.update_empty_state();
    }

    fn load_history(&self) {
        // Seeded debug runs stay in memory: a seed must never be able to trim
        // or clear the user's real history.
        #[cfg(debug_assertions)]
        if std::env::var_os("CLIPPERINO_DEBUG_SEED").is_some() {
            tracing::warn!("CLIPPERINO_DEBUG_SEED set: storage disabled for this run");
            return;
        }
        let dir = glib::user_data_dir().join("clipperino");
        match crate::storage::Storage::open(&dir) {
            Ok((storage, records)) => {
                let history = &self.imp().history;
                history.set_storage(storage);
                let mut restored = 0;
                for rec in &records {
                    if let Some(item) = ClipItem::restore(rec) {
                        history.append_restored(item);
                        restored += 1;
                    }
                }
                tracing::info!(restored, "history loaded");
                history.load_thumbnails();
                for text in crate::storage::import_legacy_history() {
                    history.add(ClipItem::new_text(text));
                }
            }
            Err(e) => tracing::error!("could not open history database: {e}"),
        }
    }

    fn selected_item(&self) -> Option<(u32, ClipItem)> {
        let sel = self.imp().selection.borrow().clone()?;
        let pos = sel.selected();
        let item = sel.selected_item().and_downcast::<ClipItem>()?;
        Some((pos, item))
    }

    /// The item at `pos` in the visible (filtered, sectioned) order.
    fn item_at(&self, pos: u32) -> Option<ClipItem> {
        self.imp()
            .selection
            .borrow()
            .as_ref()?
            .item(pos)
            .and_downcast::<ClipItem>()
    }

    /// Up/Down move a whole row of tiles in the gallery.
    fn row_step(&self) -> i32 {
        if self.gallery_shown() {
            self.imp().gallery.max_columns().max(1) as i32
        } else {
            1
        }
    }

    fn move_selection(&self, delta: i32) {
        let imp = self.imp();
        let Some(sel) = imp.selection.borrow().clone() else {
            return;
        };
        let n = sel.n_items();
        if n == 0 {
            return;
        }
        let cur = if sel.selected() == gtk::INVALID_LIST_POSITION {
            0
        } else {
            sel.selected() as i32
        };
        let next = (cur + delta).clamp(0, n as i32 - 1) as u32;
        sel.set_selected(next);
        self.scroll_to(next);
    }

    fn setup_actions(&self) {
        // Row menu actions: the target is the item's key (see `ClipItem::key`).
        let by_key = |name: &str, run: fn(&Self, &ClipItem)| {
            gio::ActionEntry::builder(name)
                .parameter_type(Some(&String::static_variant_type()))
                .activate(move |win: &Self, _, param| {
                    if let Some(item) = param
                        .and_then(|p| p.get::<String>())
                        .and_then(|k| crate::model::parse_key(&k))
                        .and_then(|h| win.imp().history.find_by_hash(&h))
                    {
                        run(win, &item);
                    }
                })
                .build()
        };
        let paste = by_key("paste-item", |win, item| win.activate_item(item));
        let copy = by_key("copy-item", |win, item| win.copy_item(item));
        let pin = by_key("pin-item", |_, item| item.set_pinned(!item.pinned()));
        let delete = by_key("delete-item", |win, item| win.delete_item(item));
        let show_item = by_key("show-item", |win, item| win.show_details(item));
        let delete_selected = gio::ActionEntry::builder("delete-selected")
            .activate(|win: &Self, _, _| {
                if let Some((_, item)) = win.selected_item() {
                    win.delete_item(&item);
                }
            })
            .build();
        let activate_selected = gio::ActionEntry::builder("activate-selected")
            .activate(|win: &Self, _, _| {
                if let Some((_, item)) = win.selected_item() {
                    win.activate_item(&item);
                }
            })
            .build();
        let activate_nth = gio::ActionEntry::builder("activate-nth")
            .parameter_type(Some(&u32::static_variant_type()))
            .activate(|win: &Self, _, param| {
                if let Some(item) = param
                    .and_then(|p| p.get::<u32>())
                    .and_then(|p| win.item_at(p))
                {
                    win.activate_item(&item);
                }
            })
            .build();
        let show_selected = gio::ActionEntry::builder("show-selected")
            .activate(|win: &Self, _, _| {
                if let Some((_, item)) = win.selected_item() {
                    win.show_details(&item);
                }
            })
            .build();
        let pin_selected = gio::ActionEntry::builder("pin-selected")
            .activate(|win: &Self, _, _| {
                if let Some((_, item)) = win.selected_item() {
                    item.set_pinned(!item.pinned());
                }
            })
            .build();
        let show_page = gio::ActionEntry::builder("show-page")
            .parameter_type(Some(&String::static_variant_type()))
            .activate(|win: &Self, _, param| {
                if let Some(name) = param.and_then(|p| p.get::<String>()) {
                    win.imp().stack.set_visible_child_name(&name);
                }
            })
            .build();
        // "all", "text", "images" or "files": the kind chips, from the keyboard.
        let show_kind = gio::ActionEntry::builder("show-kind")
            .parameter_type(Some(&String::static_variant_type()))
            .activate(|win: &Self, _, param| {
                if let Some(name) = param.and_then(|p| p.get::<String>()) {
                    win.imp().kind_chips.set_active_name(Some(&name));
                }
            })
            .build();
        let select_next = gio::ActionEntry::builder("select-next")
            .activate(|win: &Self, _, _| win.move_selection(win.row_step()))
            .build();
        let select_previous = gio::ActionEntry::builder("select-previous")
            .activate(|win: &Self, _, _| win.move_selection(-win.row_step()))
            .build();
        let focus_search = gio::ActionEntry::builder("focus-search")
            .activate(|win: &Self, _, _| win.focus_search())
            .build();
        self.add_action_entries([
            paste,
            copy,
            pin,
            delete,
            delete_selected,
            activate_selected,
            activate_nth,
            show_item,
            show_selected,
            pin_selected,
            select_next,
            select_previous,
            focus_search,
            show_page,
            show_kind,
        ]);
        self.setup_debug_actions();
    }

    /// Debug builds only: seed synthetic history (CLIPPERINO_DEBUG_SEED=N) and an
    /// animated scroll action for frame-time measurements.
    #[cfg(debug_assertions)]
    fn setup_debug_actions(&self) {
        if std::env::var("CLIPPERINO_DEBUG_SEED").as_deref() == Ok("demo") {
            self.seed_demo();
        } else if let Some(n) = std::env::var("CLIPPERINO_DEBUG_SEED")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
        {
            let history = &self.imp().history;
            let t = std::time::Instant::now();
            let now = glib::DateTime::now_local()
                .map(|d| d.to_unix())
                .unwrap_or_default();
            for i in 0..n {
                let text = match i % 7 {
                    0 => format!("Seed item {i}: the quick brown fox jumps over the lazy dog"),
                    1 => format!("fn seed_{i}() {{\n    println!(\"{i}\");\n}}"),
                    2 => format!("https://gitlab.gnome.org/GNOME/gtk/-/issues/{i}"),
                    3 => "#3584e4".to_owned(),
                    4 => format!("someone{i}@example.org"),
                    _ => format!("Seed {i} ünïcödé ✓ — multi\nline\nentry"),
                };
                // Every third item is an image, so image rows sit among text rows.
                let item = (i % 3 == 2)
                    .then(|| debug_image(i))
                    .flatten()
                    .unwrap_or_else(|| ClipItem::new_text(text));
                // Spread over the last week or two so every section shows; pin two.
                item.set_timestamp(now - i64::from(i) * 5 * 3600);
                item.set_pinned(i == 4 || i == 11);
                history.append_restored(item);
            }
            tracing::info!(n, elapsed = ?t.elapsed(), "seeded synthetic history (not persisted)");
        }
        let scroll = gio::ActionEntry::builder("debug-scroll")
            .activate(|win: &Self, _, _| win.debug_scroll())
            .build();
        let snapshot = gio::ActionEntry::builder("debug-snapshot")
            .parameter_type(Some(&String::static_variant_type()))
            .activate(|win: &Self, _, param| {
                if let Some(path) = param.and_then(|p| p.get::<String>()) {
                    win.debug_snapshot(&path);
                }
            })
            .build();
        self.add_action_entries([scroll, snapshot]);
    }

    /// Debug builds only: realistic sample history for store screenshots
    /// (CLIPPERINO_DEBUG_SEED=demo). `None` marks an image.
    #[cfg(debug_assertions)]
    fn seed_demo(&self) {
        const CLIPS: &[(Option<&str>, bool)] = &[
            (Some("Meeting moved to Thursday at 14:00, room 3B"), false),
            (
                Some("https://gitlab.gnome.org/GNOME/gtk/-/merge_requests/8421"),
                false,
            ),
            (None, false),
            (Some("#3584e4"), false),
            (Some("sudo dnf upgrade --refresh"), true),
            (Some("hello@example.org"), false),
            (
                Some("let total: u32 = items.iter().map(|i| i.price).sum();"),
                false,
            ),
            (None, false),
            (Some("Groceries: oat milk, basil, lemons, sourdough"), false),
            (Some("https://www.gnome.org/"), false),
            (Some("#2ec27e"), false),
            (Some("Tracking number: 1Z 999 AA1 01 2345 6784"), true),
            (None, false),
            (Some("The quick brown fox jumps over the lazy dog"), false),
            (Some("git commit -m \"Fix the tray menu spacing\""), false),
        ];
        let now = glib::DateTime::now_local()
            .map(|d| d.to_unix())
            .unwrap_or_default();
        let history = &self.imp().history;
        for (i, (text, pinned)) in CLIPS.iter().enumerate() {
            let i = i as u32;
            let item = match text {
                Some(t) => ClipItem::new_text((*t).to_owned()),
                None => match debug_image(i) {
                    Some(item) => item,
                    None => continue,
                },
            };
            // Minutes apart at first, then spilling into earlier days.
            let age = if i < 8 {
                i64::from(i) * 23 * 60
            } else {
                i64::from(i) * 9 * 3600
            };
            item.set_timestamp(now - age);
            item.set_pinned(*pinned);
            history.append_restored(item);
        }
        tracing::info!("seeded demo history (not persisted)");
    }

    /// Render the window at 2x into a PNG, for design reviews without the
    /// Screenshot portal (works in nested or private sessions).
    #[cfg(debug_assertions)]
    fn debug_snapshot(&self, path: &str) {
        let (w, h) = (self.width() as f64, self.height() as f64);
        let paintable = gtk::WidgetPaintable::new(Some(self));
        let snapshot = gtk::Snapshot::new();
        snapshot.scale(2.0, 2.0);
        paintable.snapshot(&snapshot, w, h);
        let Some(node) = snapshot.to_node() else {
            tracing::warn!("debug-snapshot: nothing rendered");
            return;
        };
        let Some(renderer) = self.renderer() else {
            tracing::warn!("debug-snapshot: window not realized");
            return;
        };
        match renderer.render_texture(&node, None).save_to_png(path) {
            Ok(()) => tracing::info!(path, "debug-snapshot saved"),
            Err(e) => tracing::warn!("debug-snapshot: {e}"),
        }
    }

    /// Animate the history list to the bottom over 3 s and log frame-interval
    /// statistics from the frame clock (budget: 8.3 ms at 120 Hz).
    #[cfg(debug_assertions)]
    fn debug_scroll(&self) {
        let list = self.imp().history_list.get();
        let Some(sw) = list
            .ancestor(gtk::ScrolledWindow::static_type())
            .and_downcast::<gtk::ScrolledWindow>()
        else {
            tracing::warn!("debug-scroll: no scrolled window");
            return;
        };
        let adj = sw.vadjustment();
        let target = adw::PropertyAnimationTarget::new(&adj, "value");
        let anim = adw::TimedAnimation::new(&sw, 0.0, adj.upper() - adj.page_size(), 3000, target);
        anim.set_easing(adw::Easing::EaseInOutCubic);

        let samples: std::rc::Rc<RefCell<Vec<i64>>> = Default::default();
        let last = std::rc::Rc::new(std::cell::Cell::new(0i64));
        let tick_id = sw.add_tick_callback(glib::clone!(
            #[strong]
            samples,
            #[strong]
            last,
            move |_, clock| {
                let now = clock.frame_time();
                if last.get() != 0 {
                    samples.borrow_mut().push(now - last.get());
                }
                last.set(now);
                glib::ControlFlow::Continue
            }
        ));
        let tick_id = RefCell::new(Some(tick_id));
        let t = std::time::Instant::now();
        anim.connect_done(move |_| {
            if let Some(id) = tick_id.borrow_mut().take() {
                id.remove();
            }
            let mut d = samples.borrow().clone();
            d.sort_unstable();
            let n = d.len();
            let ms = |v: i64| v as f64 / 1000.0;
            let pick = |q: f64| {
                if n == 0 {
                    0
                } else {
                    d[((n as f64 * q) as usize).min(n - 1)]
                }
            };
            let slow = d.iter().filter(|&&v| v > 9_000).count();
            tracing::info!(
                elapsed = ?t.elapsed(),
                frames = n,
                median_ms = ms(pick(0.5)),
                p95_ms = ms(pick(0.95)),
                max_ms = ms(*d.last().unwrap_or(&0)),
                over_9ms = slow,
                "debug-scroll done"
            );
        });
        anim.play();
    }

    #[cfg(not(debug_assertions))]
    fn setup_debug_actions(&self) {}

    /// Window-level shortcuts. They fire while the search entry has focus, so
    /// "type to filter, arrows to move, Enter to paste" needs no focus changes.
    fn setup_keys(&self) {
        let controller = gtk::ShortcutController::new();
        controller.set_scope(gtk::ShortcutScope::Managed);
        let add = |trigger: &str, action: &str| {
            controller.add_shortcut(gtk::Shortcut::new(
                gtk::ShortcutTrigger::parse_string(trigger),
                Some(gtk::NamedAction::new(action)),
            ));
        };
        add("Down", "win.select-next");
        add("Up", "win.select-previous");
        add("Return|KP_Enter", "win.activate-selected");
        add("Delete", "win.delete-selected");
        add("<Control>f", "win.focus-search");
        add("<Control>p", "win.pin-selected");
        add("<Control>d", "win.show-selected");
        // Left/Right step through gallery tiles, but only while the search
        // field is empty: otherwise they move its text cursor.
        for (trigger, delta) in [("Left", -1), ("Right", 1)] {
            controller.add_shortcut(gtk::Shortcut::new(
                gtk::ShortcutTrigger::parse_string(trigger),
                Some(gtk::CallbackAction::new(move |w, _| {
                    let Some(win) = w.downcast_ref::<Self>() else {
                        return glib::Propagation::Proceed;
                    };
                    if win.gallery_shown() && win.imp().search_entry.text().is_empty() {
                        win.move_selection(delta);
                        glib::Propagation::Stop
                    } else {
                        glib::Propagation::Proceed
                    }
                })),
            ));
        }
        for (n, kind) in ["all", "text", "images", "files"].iter().enumerate() {
            controller.add_shortcut(gtk::Shortcut::with_arguments(
                gtk::ShortcutTrigger::parse_string(&format!("<Alt>{}", n + 1)),
                Some(gtk::NamedAction::new("win.show-kind")),
                &kind.to_variant(),
            ));
        }
        for n in 1..=9u32 {
            controller.add_shortcut(gtk::Shortcut::with_arguments(
                gtk::ShortcutTrigger::parse_string(&format!("<Control>{n}")),
                Some(gtk::NamedAction::new("win.activate-nth")),
                &(n - 1).to_variant(),
            ));
        }
        self.add_controller(controller);
    }

    fn show_details(&self, item: &ClipItem) {
        let dialog = DetailDialog::new(item);
        dialog.copy_button().connect_clicked(glib::clone!(
            #[weak(rename_to = win)]
            self,
            #[weak]
            dialog,
            #[strong]
            item,
            move |_| {
                dialog.close();
                win.copy_item(&item);
            }
        ));
        dialog.paste_button().connect_clicked(glib::clone!(
            #[weak(rename_to = win)]
            self,
            #[weak]
            dialog,
            #[strong]
            item,
            move |_| {
                dialog.close();
                win.activate_item(&item);
            }
        ));
        dialog.present(Some(self));
    }

    /// Paste (or copy) an item chosen from the tray menu; the window stays hidden.
    pub fn paste_from_tray(&self, hash: [u8; 32]) {
        let Some(item) = self.imp().history.find_by_hash(&hash) else {
            return;
        };
        let then = if settings().boolean(settings::PASTE_ON_SELECT) {
            // Let the panel menu close and focus settle first.
            After::Paste(TRAY_PASTE_DELAY)
        } else {
            After::Nothing
        };
        self.deliver(&item, then);
    }

    /// Hide, remembering the size (Escape and paste paths do not emit close-request).
    pub fn hide_window(&self) {
        let s = settings();
        let (w, h) = self.default_size();
        s.set_int(settings::WINDOW_WIDTH, w).ok();
        s.set_int(settings::WINDOW_HEIGHT, h).ok();
        self.set_visible(false);
    }

    fn copy_item(&self, item: &ClipItem) {
        self.deliver(item, After::Toast);
    }

    /// Move `item` to the top of the history, put it on the clipboard, then
    /// do `then`. The single path for copy, paste and tray actions.
    fn deliver(&self, item: &ClipItem, then: After) {
        let Some(portal) = self.imp().portal.borrow().clone() else {
            tracing::warn!("no portal session; cannot set the clipboard");
            return;
        };
        let item = self.imp().history.add(item.clone());
        let overlay = self.imp().toast_overlay.get();
        glib::spawn_future_local(async move {
            let Some(offer) = offer_for(&item).await else {
                return;
            };
            if let Err(e) = portal.offer(offer).await {
                tracing::warn!("could not set clipboard: {e}");
                return;
            }
            match then {
                After::Nothing => {}
                After::Toast => overlay.add_toast(
                    adw::Toast::builder()
                        .title(gettext("Copied"))
                        .timeout(2)
                        .build(),
                ),
                After::Paste(delay) => {
                    glib::timeout_future(delay).await;
                    match portal.inject_paste().await {
                        Ok(()) => tracing::info!("paste injected"),
                        Err(e) => tracing::warn!("paste injection failed: {e}"),
                    }
                }
            }
        });
    }

    fn delete_item(&self, item: &ClipItem) {
        let imp = self.imp();
        let Some(pos) = imp.history.remove(item) else {
            return;
        };
        self.update_empty_state();
        let toast = adw::Toast::builder()
            .title(gettext("Item deleted"))
            .button_label(gettext("Undo"))
            .timeout(5)
            .build();
        toast.connect_button_clicked(glib::clone!(
            #[weak(rename_to = win)]
            self,
            #[strong]
            item,
            move |_| {
                win.imp().history.insert_at(pos, &item);
                win.update_empty_state();
            }
        ));
        imp.toast_overlay.add_toast(toast);
    }

    fn setup_search(&self) {
        let imp = self.imp();
        imp.search_entry.set_key_capture_widget(Some(self));
        imp.search_entry.connect_search_changed(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |entry| {
                let text = entry.text();
                if let Some(filter) = win.imp().filter.borrow().as_ref() {
                    filter.set_search(if text.is_empty() {
                        None
                    } else {
                        Some(text.as_str())
                    });
                }
            }
        ));
        imp.search_entry.connect_stop_search(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |entry| {
                if entry.text().is_empty() {
                    win.hide_window();
                } else {
                    entry.set_text("");
                }
            }
        ));
    }

    fn update_empty_state(&self) {
        let imp = self.imp();
        if imp.history_stack.visible_child_name().as_deref() == Some("permission") {
            return;
        }
        let filtered = imp
            .filter_model
            .borrow()
            .as_ref()
            .map(|m| m.n_items())
            .unwrap_or(0);
        let kind = imp.kind.get();
        let page = if imp.history.is_empty() {
            "empty"
        } else if filtered == 0 {
            let searching = !imp.search_entry.text().is_empty();
            let title = match kind {
                _ if searching => gettext("No Results"),
                Some(ClipKind::Image) => gettext("No Images"),
                Some(ClipKind::Files) => gettext("No Files"),
                Some(ClipKind::Text) => gettext("No Text"),
                None => gettext("No Results"),
            };
            imp.no_results_page.set_title(&title);
            imp.no_results_page.set_description(
                (!searching && kind.is_some())
                    .then(|| gettext("Copied items of this kind will appear here."))
                    .as_deref(),
            );
            "no-results"
        } else if kind == Some(ClipKind::Image) {
            "gallery"
        } else {
            "list"
        };
        imp.history_stack.set_visible_child_name(page);
    }

    pub fn show_permission_page(&self) {
        self.imp()
            .history_stack
            .set_visible_child_name("permission");
    }

    pub fn grant_button(&self) -> gtk::Button {
        self.imp().grant_button.get()
    }

    /// The session ended: stop offering copy and paste, ask for access again.
    pub fn clear_portal(&self) {
        self.imp().portal.replace(None);
        self.show_permission_page();
    }

    pub fn set_portal(&self, portal: Rc<PortalSession>) {
        self.imp().portal.replace(Some(portal));
        self.imp().history_stack.set_visible_child_name("empty");
        self.update_empty_state();
    }

    pub fn handle_clip_event(&self, event: ClipEvent) {
        if settings().boolean(settings::PAUSE_RECORDING) {
            tracing::debug!("recording paused; clip ignored");
            return;
        }
        let item = match event {
            ClipEvent::Text(text) => ClipItem::new_text(text),
            ClipEvent::Image { bytes, .. } => {
                // Decode off the main thread; insert the row when done.
                glib::spawn_future_local(glib::clone!(
                    #[weak(rename_to = win)]
                    self,
                    async move {
                        let decoded = gio::spawn_blocking({
                            let bytes = bytes.clone();
                            move || crate::model::images::decode(&bytes)
                        })
                        .await;
                        match decoded {
                            Ok(Ok(d)) => {
                                let item =
                                    ClipItem::new_image(bytes, d.pixel_hash, d.width, d.height);
                                item.set_thumbnail(Some(d.thumbnail));
                                let item = win.imp().history.add(item);
                                tracing::info!(kind = ?item.kind(), total = win.imp().history.len(), "clip added");
                                win.update_empty_state();
                            }
                            Ok(Err(e)) => tracing::warn!("could not decode image: {e}"),
                            Err(_) => {}
                        }
                    }
                ));
                return;
            }
            ClipEvent::Uris(uris) => ClipItem::new_files(&uris),
        };
        let item = self.imp().history.add(item);
        // Never log clipboard content: it may be a password or private data.
        tracing::info!(kind = ?item.kind(), total = self.imp().history.len(), "clip added");
        self.update_empty_state();
    }

    /// Select an item: own the clipboard via the portal, hide, paste into the
    /// previously focused app.
    fn activate_item(&self, item: &ClipItem) {
        tracing::info!(kind = ?item.kind(), "item activated");
        if self.imp().portal.borrow().is_none() {
            // Stay visible: the permission page explains what is missing.
            tracing::warn!("no portal session; cannot paste");
            return;
        }
        self.hide_window();
        let then = if settings().boolean(settings::PASTE_ON_SELECT) {
            // Give the compositor time to hand focus back to the previous window.
            After::Paste(WINDOW_PASTE_DELAY)
        } else {
            After::Nothing
        };
        self.deliver(item, then);
    }

    pub fn focus_search(&self) {
        let imp = self.imp();
        match imp.stack.visible_child_name().as_deref() {
            Some("emoji") => imp
                .emoji_bin
                .child()
                .and_downcast::<GlyphPage>()
                .map(|p| p.search_entry().grab_focus()),
            Some("symbols") => imp
                .symbols_bin
                .child()
                .and_downcast::<GlyphPage>()
                .map(|p| p.search_entry().grab_focus()),
            _ => Some(imp.search_entry.grab_focus()),
        };
    }
}

/// Build the clipboard offer; image bytes come from disk.
async fn offer_for(item: &ClipItem) -> Option<Offer> {
    match item.kind() {
        ClipKind::Image => {
            let path = item.image_path()?;
            let file = gio::File::for_path(path);
            match file.load_contents_future().await {
                Ok((bytes, _)) => Some(Offer::Png(bytes.to_vec())),
                Err(e) => {
                    tracing::warn!("could not read image: {e}");
                    None
                }
            }
        }
        ClipKind::Files => item
            .text()
            .map(|t| Offer::Files(t.lines().map(str::to_owned).collect())),
        ClipKind::Text => item.text().map(Offer::Text),
    }
}

/// Debug seed: synthetic images of assorted shapes (screenshot, photo, logo
/// with transparency, tall receipt, wide banner), so image rows can be judged
/// without real clipboard content.
#[cfg(debug_assertions)]
fn debug_image(i: u32) -> Option<ClipItem> {
    use gtk::gdk_pixbuf::{Colorspace, Pixbuf};
    let rgba = |c: u32| (c << 8) | 0xff;
    let hues = [0x3584e4, 0x2ec27e, 0xe66100, 0x9141ac, 0xe01b24, 0xf6d32d];
    let hue = hues[(i as usize / 3) % hues.len()];
    let (w, h, bg) = match (i / 3) % 5 {
        0 => (1920, 1080, 0xf6f5f4),
        1 => (1600, 1200, hue),
        2 => (256, 256, 0),
        3 => (500, 1400, 0xffffff),
        _ => (1500, 320, 0x241f31),
    };
    let pb = Pixbuf::new(Colorspace::Rgb, true, 8, w, h)?;
    pb.fill(if bg == 0 { 0 } else { rgba(bg) });
    let rect = |x: i32, y: i32, rw: i32, rh: i32, c: u32| {
        pb.new_subpixbuf(x, y, rw.min(w - x), rh.min(h - y))
            .fill(rgba(c));
    };
    match (i / 3) % 5 {
        0 => {
            rect(0, 0, w, 60, 0x303030);
            rect(0, 60, 320, h - 60, 0xdeddda);
            for k in 0..6 {
                rect(
                    380,
                    120 + k * 150,
                    1400,
                    100,
                    if k == 1 { hue } else { 0xc0bfbc },
                );
            }
        }
        1 => {
            for k in 0..12 {
                rect(0, k * 100, w, 50, hue ^ (k as u32 * 0x0a0a0a));
            }
            rect(500, 350, 600, 500, 0xffffff);
        }
        2 => {
            rect(48, 48, 160, 160, hue);
            rect(96, 96, 64, 64, 0xffffff);
        }
        3 => {
            for k in 0..20 {
                rect(40, 60 + k * 64, 300 + (k % 3) * 40, 20, 0x9a9996);
            }
            rect(40, 1330, 420, 30, hue);
        }
        _ => {
            rect(60, 60, 200, 200, hue);
            rect(320, 110, 900, 40, 0xffffff);
            rect(320, 180, 600, 30, 0x9a9996);
        }
    }
    let png = pb.save_to_bufferv("png", &[]).ok()?;
    let d = crate::model::images::decode(&png).ok()?;
    let item = ClipItem::new_image(png, d.pixel_hash, d.width, d.height);
    item.set_thumbnail(Some(d.thumbnail));
    Some(item)
}

/// What `deliver` does once the clipboard is set.
enum After {
    Nothing,
    Toast,
    Paste(std::time::Duration),
}

/// Delay before pressing Ctrl+V, so focus is back in the target app. The
/// tray needs longer: the panel menu has to close first.
const WINDOW_PASTE_DELAY: std::time::Duration = std::time::Duration::from_millis(250);
const TRAY_PASTE_DELAY: std::time::Duration = std::time::Duration::from_millis(350);
