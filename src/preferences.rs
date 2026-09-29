use adw::{prelude::*, subclass::prelude::*};
use gtk::{CompositeTemplate, glib};

use crate::settings::{self, settings};

mod imp {
    use super::*;

    #[derive(Default, CompositeTemplate)]
    #[template(resource = "/io/github/djshiye/Clipperino/ui/preferences.ui")]
    pub struct ClipperinoPreferences {
        #[template_child]
        pub max_history_row: TemplateChild<adw::SpinRow>,
        #[template_child]
        pub paste_on_select_row: TemplateChild<adw::SwitchRow>,
        #[template_child]
        pub clear_row: TemplateChild<adw::ButtonRow>,
        #[template_child]
        pub background_row: TemplateChild<adw::SwitchRow>,
        #[template_child]
        pub tray_row: TemplateChild<adw::SwitchRow>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ClipperinoPreferences {
        const NAME: &'static str = "ClipperinoPreferences";
        type Type = super::ClipperinoPreferences;
        type ParentType = adw::PreferencesDialog;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for ClipperinoPreferences {
        fn constructed(&self) {
            self.parent_constructed();
            let s = settings();
            // Every control applies immediately (HIG: no Save button).
            s.bind(settings::MAX_HISTORY, &*self.max_history_row, "value")
                .build();
            s.bind(
                settings::PASTE_ON_SELECT,
                &*self.paste_on_select_row,
                "active",
            )
            .build();
            s.bind(settings::RUN_IN_BACKGROUND, &*self.background_row, "active")
                .build();
            self.clear_row.connect_activated(glib::clone!(
                #[weak(rename_to = dialog)]
                self.obj(),
                move |_| dialog.confirm_clear()
            ));
        }
    }
    impl WidgetImpl for ClipperinoPreferences {}
    impl AdwDialogImpl for ClipperinoPreferences {}
    impl PreferencesDialogImpl for ClipperinoPreferences {}
}

glib::wrapper! {
    pub struct ClipperinoPreferences(ObjectSubclass<imp::ClipperinoPreferences>)
        @extends adw::PreferencesDialog, adw::Dialog, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for ClipperinoPreferences {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl ClipperinoPreferences {
    fn confirm_clear(&self) {
        if let Some(app) = gtk::gio::Application::default().and_then(|a| {
            a.downcast::<crate::application::ClipperinoApplication>()
                .ok()
        }) {
            app.confirm_clear_history();
        }
    }
}
