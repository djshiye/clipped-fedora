use adw::{prelude::*, subclass::prelude::*};
use gtk::{CompositeTemplate, glib};

use crate::{
    i18n::gettext,
    model::{ClipItem, ClipKind},
};

mod imp {
    use super::*;

    #[derive(Default, CompositeTemplate)]
    #[template(resource = "/io/github/djshiye/Clipperino/ui/detail_dialog.ui")]
    pub struct DetailDialog {
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub text_view: TemplateChild<gtk::TextView>,
        #[template_child]
        pub picture: TemplateChild<gtk::Picture>,
        #[template_child]
        pub meta_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub copy_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub paste_button: TemplateChild<gtk::Button>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DetailDialog {
        const NAME: &'static str = "ClipperinoDetailDialog";
        type Type = super::DetailDialog;
        type ParentType = adw::Dialog;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for DetailDialog {}
    impl WidgetImpl for DetailDialog {}
    impl AdwDialogImpl for DetailDialog {}
}

glib::wrapper! {
    pub struct DetailDialog(ObjectSubclass<imp::DetailDialog>)
        @extends adw::Dialog, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl DetailDialog {
    pub fn new(item: &ClipItem) -> Self {
        let dialog: Self = glib::Object::new();
        let imp = dialog.imp();
        match item.kind() {
            ClipKind::Image => {
                let texture = item
                    .image_path()
                    .and_then(|p| gtk::gdk::Texture::from_filename(p).ok())
                    .or_else(|| item.thumbnail());
                if let Some(t) = &texture {
                    imp.meta_label.set_label(
                        &gettext("{w} × {h} px")
                            .replace("{w}", &t.width().to_string())
                            .replace("{h}", &t.height().to_string()),
                    );
                }
                imp.picture.set_paintable(texture.as_ref());
                imp.stack.set_visible_child_name("image");
            }
            _ => {
                let text = item.text().unwrap_or_default();
                let chars = text.chars().count();
                let lines = text.lines().count().max(1);
                imp.meta_label.set_label(
                    &gettext("{chars} characters, {lines} lines")
                        .replace("{chars}", &chars.to_string())
                        .replace("{lines}", &lines.to_string()),
                );
                imp.text_view.buffer().set_text(&text);
                if crate::ui::looks_like_code(&text) {
                    imp.text_view.add_css_class("monospace");
                }
                imp.stack.set_visible_child_name("text");
            }
        }
        dialog
    }

    pub fn copy_button(&self) -> gtk::Button {
        self.imp().copy_button.get()
    }

    pub fn paste_button(&self) -> gtk::Button {
        self.imp().paste_button.get()
    }
}
