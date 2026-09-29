use std::cell::RefCell;

use adw::{prelude::*, subclass::prelude::*};
use gtk::{CompositeTemplate, glib};

use crate::model::ClipItem;

mod imp {
    use super::*;

    #[derive(Default, CompositeTemplate)]
    #[template(resource = "/io/github/djshiye/Clipperino/ui/image_tile.ui")]
    pub struct ImageTile {
        #[template_child]
        pub picture: TemplateChild<gtk::Picture>,
        #[template_child]
        pub pin_icon: TemplateChild<gtk::Image>,
        #[template_child]
        pub size: TemplateChild<gtk::Label>,
        #[template_child]
        pub when: TemplateChild<gtk::Label>,
        pub bindings: RefCell<Vec<glib::Binding>>,
        pub item: RefCell<Option<ClipItem>>,
        pub context_menu: RefCell<Option<gtk::PopoverMenu>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ImageTile {
        const NAME: &'static str = "ClipperinoImageTile";
        type Type = super::ImageTile;
        type ParentType = gtk::Box;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for ImageTile {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            let popover = gtk::PopoverMenu::from_model(None::<&gtk::gio::MenuModel>);
            popover.set_parent(&*obj);
            popover.set_has_arrow(false);
            let weak = obj.downgrade();
            // Built on open, so Pin/Unpin is always current.
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
            crate::ui::attach_image_tooltip(&*obj, move || {
                weak.upgrade()?.imp().item.borrow().clone()
            });
        }

        fn dispose(&self) {
            if let Some(menu) = self.context_menu.take() {
                menu.unparent();
            }
        }
    }
    impl WidgetImpl for ImageTile {}
    impl BoxImpl for ImageTile {}
}

glib::wrapper! {
    pub struct ImageTile(ObjectSubclass<imp::ImageTile>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl Default for ImageTile {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl ImageTile {
    pub fn bind(&self, item: &ClipItem) {
        let imp = self.imp();
        imp.item.replace(Some(item.clone()));
        let mut bindings = imp.bindings.borrow_mut();
        bindings.push(
            item.bind_property("thumbnail", &*imp.picture, "paintable")
                .sync_create()
                .build(),
        );
        bindings.push(
            item.bind_property("pinned", &*imp.pin_icon, "visible")
                .sync_create()
                .build(),
        );
        // "Image · 1920 × 1080" → "1920 × 1080": the gallery says "image".
        let preview = item.preview();
        let size = preview
            .split_once(" · ")
            .map_or(preview.as_str(), |(_, s)| s);
        imp.size.set_label(size);
        let when = crate::ui::short_when(item.timestamp());
        imp.when.set_label(&when);
        self.update_property(&[gtk::accessible::Property::Label(&format!(
            "{preview}, {when}"
        ))]);
    }

    pub fn unbind(&self) {
        let imp = self.imp();
        for b in imp.bindings.borrow_mut().drain(..) {
            b.unbind();
        }
        imp.item.replace(None);
        imp.picture.set_paintable(gtk::gdk::Paintable::NONE);
    }
}
