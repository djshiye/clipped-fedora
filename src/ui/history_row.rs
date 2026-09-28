use std::cell::RefCell;

use adw::{prelude::*, subclass::prelude::*};
use gtk::{CompositeTemplate, gio, glib};

use crate::{
    i18n::gettext,
    model::{ClipItem, ClipKind},
};

mod imp {
    use super::*;

    #[derive(Default, CompositeTemplate)]
    #[template(resource = "/io/github/djshiye/Clipped/ui/history_row.ui")]
    pub struct HistoryRow {
        #[template_child]
        pub icon: TemplateChild<gtk::Image>,
        #[template_child]
        pub picture: TemplateChild<gtk::Picture>,
        #[template_child]
        pub preview: TemplateChild<gtk::Label>,
        #[template_child]
        pub time: TemplateChild<gtk::Label>,
        #[template_child]
        pub pin_icon: TemplateChild<gtk::Image>,
        #[template_child]
        pub more_button: TemplateChild<gtk::MenuButton>,
        pub bindings: RefCell<Vec<glib::Binding>>,
        pub menu: RefCell<Option<gio::Menu>>,
        pub context_menu: RefCell<Option<gtk::PopoverMenu>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for HistoryRow {
        const NAME: &'static str = "ClippedHistoryRow";
        type Type = super::HistoryRow;
        type ParentType = gtk::Box;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for HistoryRow {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            // Right-click opens the same menu as the ⋮ button.
            let gesture = gtk::GestureClick::builder().button(3).build();
            gesture.connect_pressed(glib::clone!(
                #[weak]
                obj,
                move |gesture, _, x, y| {
                    gesture.set_state(gtk::EventSequenceState::Claimed);
                    obj.show_context_menu(x, y);
                }
            ));
            obj.add_controller(gesture);
        }

        fn dispose(&self) {
            if let Some(menu) = self.context_menu.take() {
                menu.unparent();
            }
        }
    }
    impl WidgetImpl for HistoryRow {}
    impl BoxImpl for HistoryRow {}
}

glib::wrapper! {
    pub struct HistoryRow(ObjectSubclass<imp::HistoryRow>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl Default for HistoryRow {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl HistoryRow {
    /// `position` is the row's index in the visible (filtered) list; the row
    /// menu targets window actions with it.
    pub fn bind(&self, item: &ClipItem, position: u32) {
        let imp = self.imp();
        imp.preview.set_label(&item.preview());
        let code = item.kind() == ClipKind::Text
            && item.text().is_some_and(|t| crate::ui::looks_like_code(&t));
        if code {
            imp.preview.add_css_class("monospace");
        } else {
            imp.preview.remove_css_class("monospace");
        }
        imp.time.set_label(&relative_time(item.timestamp()));
        match item.kind() {
            ClipKind::Image => {
                imp.icon.set_visible(false);
                let b = item
                    .bind_property("thumbnail", &*imp.picture, "paintable")
                    .sync_create()
                    .build();
                imp.bindings.borrow_mut().push(b);
                imp.picture.set_visible(true);
            }
            kind => {
                imp.picture.set_visible(false);
                imp.picture.set_paintable(gtk::gdk::Paintable::NONE);
                imp.icon.set_icon_name(Some(match kind {
                    ClipKind::Files => "folder-symbolic",
                    _ => "text-x-generic-symbolic",
                }));
                imp.icon.set_visible(true);
            }
        }
        let pin_binding = item
            .bind_property("pinned", &*imp.pin_icon, "visible")
            .sync_create()
            .build();
        imp.bindings.borrow_mut().push(pin_binding);

        let menu = gio::Menu::new();
        let add = |section: &gio::Menu, label: &str, action: &str| {
            let mi = gio::MenuItem::new(Some(label), None);
            mi.set_action_and_target_value(Some(action), Some(&position.to_variant()));
            section.append_item(&mi);
        };
        let section = gio::Menu::new();
        add(&section, &gettext("Paste"), "win.activate-nth");
        add(&section, &gettext("Copy"), "win.copy-item");
        add(
            &section,
            &if item.pinned() {
                gettext("Unpin")
            } else {
                gettext("Pin")
            },
            "win.pin-item",
        );
        add(&section, &gettext("Details…"), "win.show-item");
        menu.append_section(None, &section);
        let danger = gio::Menu::new();
        add(&danger, &gettext("Delete"), "win.delete-item");
        menu.append_section(None, &danger);
        imp.more_button.set_menu_model(Some(&menu));
        imp.menu.replace(Some(menu));

        self.update_property(&[gtk::accessible::Property::Label(&item.preview())]);
    }

    pub fn unbind(&self) {
        let imp = self.imp();
        for b in imp.bindings.borrow_mut().drain(..) {
            b.unbind();
        }
        imp.picture.set_paintable(gtk::gdk::Paintable::NONE);
        imp.more_button.set_menu_model(None::<&gio::MenuModel>);
        imp.menu.replace(None);
    }

    fn show_context_menu(&self, x: f64, y: f64) {
        let imp = self.imp();
        let Some(model) = imp.menu.borrow().clone() else {
            return;
        };
        let popover = imp.context_menu.borrow().clone().unwrap_or_else(|| {
            let p = gtk::PopoverMenu::from_model(None::<&gio::MenuModel>);
            p.set_parent(self);
            p.set_has_arrow(false);
            imp.context_menu.replace(Some(p.clone()));
            p
        });
        popover.set_menu_model(Some(&model));
        popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        popover.popup();
    }
}

pub(crate) fn relative_time(unix: i64) -> String {
    let now = glib::DateTime::now_local()
        .map(|d| d.to_unix())
        .unwrap_or(unix);
    let secs = (now - unix).max(0);
    match secs {
        0..=59 => gettext("Just now"),
        60..=3599 => gettext("{} min ago").replace("{}", &(secs / 60).to_string()),
        3600..=86_399 => gettext("{} h ago").replace("{}", &(secs / 3600).to_string()),
        _ => glib::DateTime::from_unix_local(unix)
            .and_then(|d| d.format("%x"))
            .map(|s| s.to_string())
            .unwrap_or_default(),
    }
}
