use std::cell::RefCell;

use adw::{prelude::*, subclass::prelude::*};
use gtk::{CompositeTemplate, glib};

use crate::{
    i18n::gettext,
    model::{ClipItem, ClipKind, Flavor},
};

mod imp {
    use super::*;

    #[derive(Default, CompositeTemplate)]
    #[template(resource = "/io/github/djshiye/Clipperino/ui/history_row.ui")]
    pub struct HistoryRow {
        #[template_child]
        pub media: TemplateChild<gtk::Overlay>,
        #[template_child]
        pub media_picture: TemplateChild<gtk::Picture>,
        #[template_child]
        pub icon: TemplateChild<gtk::Image>,
        #[template_child]
        pub swatch: TemplateChild<gtk::Picture>,
        #[template_child]
        pub preview: TemplateChild<gtk::Label>,
        #[template_child]
        pub caption: TemplateChild<gtk::Label>,
        #[template_child]
        pub pin_icon: TemplateChild<gtk::Image>,
        #[template_child]
        pub more_button: TemplateChild<gtk::MenuButton>,
        pub bindings: RefCell<Vec<glib::Binding>>,
        pub item: RefCell<Option<ClipItem>>,
        pub pinned_handler: RefCell<Option<glib::SignalHandlerId>>,
        pub context_menu: RefCell<Option<gtk::PopoverMenu>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for HistoryRow {
        const NAME: &'static str = "ClipperinoHistoryRow";
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
            let popover = gtk::PopoverMenu::from_model(None::<&gtk::gio::MenuModel>);
            popover.set_parent(&*obj);
            popover.set_has_arrow(false);
            let weak = obj.downgrade();
            crate::ui::attach_context_menu(&*obj, &popover.downgrade(), move || {
                weak.upgrade()?
                    .imp()
                    .item
                    .borrow()
                    .as_ref()
                    .map(crate::ui::item_menu)
            });
            self.context_menu.replace(Some(popover));
            let weak = obj.downgrade();
            crate::ui::attach_image_tooltip(&*self.media, move || {
                weak.upgrade()?.imp().item.borrow().clone()
            });
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
    pub fn bind(&self, item: &ClipItem) {
        let imp = self.imp();
        imp.item.replace(Some(item.clone()));
        imp.preview.set_label(&item.preview());
        let text = item.text();
        let flavor = Flavor::of(item.kind(), text.as_deref());
        if flavor == Flavor::Code {
            imp.preview.add_css_class("monospace");
        } else {
            imp.preview.remove_css_class("monospace");
        }

        let is_image = item.kind() == ClipKind::Image;
        imp.media.set_visible(is_image);
        if is_image {
            let b = item
                .bind_property("thumbnail", &*imp.media_picture, "paintable")
                .sync_create()
                .build();
            imp.bindings.borrow_mut().push(b);
        } else {
            imp.media_picture.set_paintable(gtk::gdk::Paintable::NONE);
        }

        // Leading slot: a colour swatch for colour codes, else the type icon.
        // Image cards need neither: the picture says it.
        match &flavor {
            Flavor::Color(rgba) => {
                imp.swatch
                    .set_paintable(Some(&crate::model::images::swatch(*rgba)));
                imp.swatch.set_visible(true);
                imp.icon.set_visible(false);
            }
            Flavor::Image => {
                imp.swatch.set_visible(false);
                imp.icon.set_visible(false);
            }
            f => {
                imp.swatch.set_visible(false);
                imp.icon.set_icon_name(Some(f.icon_name()));
                imp.icon.set_visible(true);
            }
        }

        let caption = match &flavor {
            Flavor::Link { domain } => domain.clone(),
            Flavor::Email => gettext("Email address"),
            // Cards carry their time (hovering one shows the picture instead).
            Flavor::Image => crate::ui::short_when(item.timestamp()),
            Flavor::Files { count } if *count > 1 => {
                gettext("{n} files").replace("{n}", &count.to_string())
            }
            _ => String::new(),
        };
        imp.caption.set_visible(!caption.is_empty());
        imp.caption.set_label(&caption);

        let pin_binding = item
            .bind_property("pinned", &*imp.pin_icon, "visible")
            .sync_create()
            .build();
        imp.bindings.borrow_mut().push(pin_binding);

        // The Pin/Unpin label follows the item, whichever way it is toggled.
        let handler = item.connect_pinned_notify(glib::clone!(
            #[weak(rename_to = row)]
            self,
            move |item| row
                .imp()
                .more_button
                .set_menu_model(Some(&crate::ui::item_menu(item)))
        ));
        imp.pinned_handler.replace(Some(handler));
        imp.more_button
            .set_menu_model(Some(&crate::ui::item_menu(item)));

        // Rows carry no time (the section header has the day); say it here.
        let when = crate::ui::short_when(item.timestamp());
        self.set_tooltip_text((!is_image).then_some(when.as_str()));
        self.update_property(&[gtk::accessible::Property::Label(&format!(
            "{}, {when}",
            item.preview()
        ))]);
    }

    pub fn unbind(&self) {
        let imp = self.imp();
        for b in imp.bindings.borrow_mut().drain(..) {
            b.unbind();
        }
        if let (Some(item), Some(handler)) = (imp.item.take(), imp.pinned_handler.take()) {
            item.disconnect(handler);
        }
        imp.media_picture.set_paintable(gtk::gdk::Paintable::NONE);
        imp.more_button.set_menu_model(None::<&gtk::gio::MenuModel>);
    }
}
