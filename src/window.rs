use std::{cell::RefCell, rc::Rc};

use adw::{prelude::*, subclass::prelude::*};
use gtk::{CompositeTemplate, gio, glib};

use crate::{
    i18n::gettext,
    model::{ClipItem, ClipKind, HistoryStore},
    platform::portal::{ClipEvent, Offer, PortalSession},
    settings::{self, settings},
    ui::{DetailDialog, GlyphPage, HistoryRow},
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

        pub history: HistoryStore,
        pub filter: RefCell<Option<gtk::StringFilter>>,
        pub filter_model: RefCell<Option<gtk::FilterListModel>>,
        pub selection: RefCell<Option<gtk::SingleSelection>>,
        pub preview_binding: RefCell<Option<glib::Binding>>,
        pub portal: Rc<RefCell<Option<Rc<PortalSession>>>>,
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

        // history (ListStore) -> StringFilter on "preview" -> SingleSelection -> ListView
        let expr =
            gtk::PropertyExpression::new(ClipItem::static_type(), gtk::Expression::NONE, "preview");
        let filter = gtk::StringFilter::builder()
            .expression(&expr)
            .ignore_case(true)
            .match_mode(gtk::StringFilterMatchMode::Substring)
            .build();
        let filter_model =
            gtk::FilterListModel::new(Some(imp.history.model().clone()), Some(filter.clone()));
        // Always keep a selected row so Enter in the search field has a target.
        let selection = gtk::SingleSelection::builder()
            .model(&filter_model)
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
            row.bind(&clip, item.position());
        });
        factory.connect_unbind(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            if let Some(row) = item.child().and_downcast::<HistoryRow>() {
                row.unbind();
            }
        });

        imp.history_list.set_model(Some(&selection));
        imp.history_list.set_factory(Some(&factory));
        imp.history_list.connect_activate(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |list, pos| {
                if let Some(clip) = list
                    .model()
                    .and_then(|m| m.item(pos))
                    .and_downcast::<ClipItem>()
                {
                    win.activate_item(&clip);
                }
            }
        ));

        filter_model.connect_items_changed(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |_, _, _, _| win.update_empty_state()
        ));

        selection.connect_selected_item_notify(glib::clone!(
            #[weak(rename_to = win)]
            self,
            move |sel| win.update_preview(sel.selected_item().and_downcast::<ClipItem>().as_ref())
        ));

        imp.filter.replace(Some(filter));
        imp.filter_model.replace(Some(filter_model));
        imp.selection.replace(Some(selection));
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
                // Bind rather than copy: restored thumbnails arrive asynchronously.
                let b = item
                    .bind_property("thumbnail", &*imp.preview_picture, "paintable")
                    .sync_create()
                    .build();
                imp.preview_binding.replace(Some(b));
                imp.preview_stack.set_visible_child_name("image");
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
        let when = crate::ui::relative_time(item.timestamp());
        let size = match item.kind() {
            ClipKind::Image => item.preview(),
            _ => {
                let text = item.text().unwrap_or_default();
                gettext("{chars} characters").replace("{chars}", &text.chars().count().to_string())
            }
        };
        imp.preview_meta.set_label(&format!("{when} · {size}"));
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

    fn item_at(&self, pos: u32) -> Option<ClipItem> {
        self.imp()
            .filter_model
            .borrow()
            .as_ref()?
            .item(pos)
            .and_downcast::<ClipItem>()
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
        imp.history_list
            .scroll_to(next, gtk::ListScrollFlags::NONE, None);
    }

    fn setup_actions(&self) {
        let copy = gio::ActionEntry::builder("copy-item")
            .parameter_type(Some(&u32::static_variant_type()))
            .activate(|win: &Self, _, param| {
                if let Some(item) = param
                    .and_then(|p| p.get::<u32>())
                    .and_then(|p| win.item_at(p))
                {
                    win.copy_item(&item);
                }
            })
            .build();
        let pin = gio::ActionEntry::builder("pin-item")
            .parameter_type(Some(&u32::static_variant_type()))
            .activate(|win: &Self, _, param| {
                if let Some(item) = param
                    .and_then(|p| p.get::<u32>())
                    .and_then(|p| win.item_at(p))
                {
                    item.set_pinned(!item.pinned());
                }
            })
            .build();
        let delete = gio::ActionEntry::builder("delete-item")
            .parameter_type(Some(&u32::static_variant_type()))
            .activate(|win: &Self, _, param| {
                if let Some(item) = param
                    .and_then(|p| p.get::<u32>())
                    .and_then(|p| win.item_at(p))
                {
                    win.delete_item(&item);
                }
            })
            .build();
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
        let show_item = gio::ActionEntry::builder("show-item")
            .parameter_type(Some(&u32::static_variant_type()))
            .activate(|win: &Self, _, param| {
                if let Some(item) = param
                    .and_then(|p| p.get::<u32>())
                    .and_then(|p| win.item_at(p))
                {
                    win.show_details(&item);
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
        let select_next = gio::ActionEntry::builder("select-next")
            .activate(|win: &Self, _, _| win.move_selection(1))
            .build();
        let select_previous = gio::ActionEntry::builder("select-previous")
            .activate(|win: &Self, _, _| win.move_selection(-1))
            .build();
        let focus_search = gio::ActionEntry::builder("focus-search")
            .activate(|win: &Self, _, _| win.focus_search())
            .build();
        self.add_action_entries([
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
        ]);
        self.setup_debug_actions();
    }

    /// Debug builds only: seed synthetic history (CLIPPERINO_DEBUG_SEED=N) and an
    /// animated scroll action for frame-time measurements.
    #[cfg(debug_assertions)]
    fn setup_debug_actions(&self) {
        if let Some(n) = std::env::var("CLIPPERINO_DEBUG_SEED")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
        {
            let history = &self.imp().history;
            let t = std::time::Instant::now();
            for i in 0..n {
                let text = match i % 4 {
                    0 => format!("Seed item {i}: the quick brown fox jumps over the lazy dog"),
                    1 => format!("fn seed_{i}() {{\n    println!(\"{i}\");\n}}"),
                    2 => format!("https://example.com/path/{i}?query=seed&n={i}"),
                    _ => format!("Seed {i} ünïcödé ✓ — multi\nline\nentry"),
                };
                history.append_restored(ClipItem::new_text(text));
            }
            tracing::info!(n, elapsed = ?t.elapsed(), "seeded synthetic history (not persisted)");
        }
        let scroll = gio::ActionEntry::builder("debug-scroll")
            .activate(|win: &Self, _, _| win.debug_scroll())
            .build();
        self.add_action_entries([scroll]);
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
        let Some(portal) = self.imp().portal.borrow().clone() else {
            return;
        };
        let item = self.imp().history.add(item);
        let paste = settings().boolean(settings::PASTE_ON_SELECT);
        glib::spawn_future_local(async move {
            let Some(offer) = offer_for(&item).await else {
                return;
            };
            if let Err(e) = portal.offer(offer).await {
                tracing::warn!("could not set clipboard: {e}");
                return;
            }
            if paste {
                // Let the panel menu close and focus settle first.
                glib::timeout_future(std::time::Duration::from_millis(350)).await;
                match portal.inject_paste().await {
                    Ok(()) => tracing::info!("paste injected (tray)"),
                    Err(e) => tracing::warn!("paste injection failed: {e}"),
                }
            }
        });
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
        let Some(portal) = self.imp().portal.borrow().clone() else {
            return;
        };
        self.imp().history.add(item.clone());
        let overlay = self.imp().toast_overlay.get();
        let item = item.clone();
        glib::spawn_future_local(async move {
            let Some(offer) = offer_for(&item).await else {
                return;
            };
            match portal.offer(offer).await {
                Ok(()) => overlay.add_toast(
                    adw::Toast::builder()
                        .title(gettext("Copied"))
                        .timeout(2)
                        .build(),
                ),
                Err(e) => tracing::warn!("could not set clipboard: {e}"),
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
        let page = if imp.history.is_empty() {
            "empty"
        } else if filtered == 0 {
            "no-results"
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

    pub fn set_portal(&self, portal: Rc<PortalSession>) {
        self.imp().portal.replace(Some(portal));
        self.imp().history_stack.set_visible_child_name("empty");
        self.update_empty_state();
    }

    pub fn handle_clip_event(&self, event: ClipEvent) {
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
                                let item = ClipItem::new_image(bytes, d.width, d.height);
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
        let Some(portal) = self.imp().portal.borrow().clone() else {
            tracing::warn!("no portal session; cannot paste");
            return;
        };
        self.imp().history.add(item.clone());
        let paste = settings().boolean(settings::PASTE_ON_SELECT);
        if !paste {
            // Copy-and-close: the toast would be invisible after hiding, so hide only.
            self.copy_item(item);
            self.hide_window();
            return;
        }
        self.hide_window();
        let item = item.clone();
        glib::spawn_future_local(async move {
            let Some(offer) = offer_for(&item).await else {
                return;
            };
            if let Err(e) = portal.offer(offer).await {
                tracing::warn!("could not set clipboard: {e}");
                return;
            }
            // Give the compositor time to hand focus back to the previous window.
            glib::timeout_future(std::time::Duration::from_millis(250)).await;
            match portal.inject_paste().await {
                Ok(()) => tracing::info!("paste injected"),
                Err(e) => tracing::warn!("paste injection failed: {e}"),
            }
        });
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
        _ => item.text().map(Offer::Text),
    }
}
