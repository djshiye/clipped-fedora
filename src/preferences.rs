use adw::{prelude::*, subclass::prelude::*};
use gtk::{CompositeTemplate, glib};

use crate::{
    i18n::gettext,
    settings::{self, settings},
};

mod imp {
    use super::*;

    #[derive(Default, CompositeTemplate)]
    #[template(resource = "/io/github/djshiye/Clipped/ui/preferences.ui")]
    pub struct ClippedPreferences {
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
    impl ObjectSubclass for ClippedPreferences {
        const NAME: &'static str = "ClippedPreferences";
        type Type = super::ClippedPreferences;
        type ParentType = adw::PreferencesDialog;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for ClippedPreferences {
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
    impl WidgetImpl for ClippedPreferences {}
    impl AdwDialogImpl for ClippedPreferences {}
    impl PreferencesDialogImpl for ClippedPreferences {}
}

glib::wrapper! {
    pub struct ClippedPreferences(ObjectSubclass<imp::ClippedPreferences>)
        @extends adw::PreferencesDialog, adw::Dialog, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for ClippedPreferences {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl ClippedPreferences {
    /// HIG: warn before irreversible loss that is not the obvious result of
    /// the action; clearing everything qualifies.
    fn confirm_clear(&self) {
        let dialog = adw::AlertDialog::builder()
            .heading(gettext("Clear Clipboard History?"))
            .body(gettext(
                "All entries, including pinned items, will be permanently deleted.",
            ))
            .default_response("cancel")
            .close_response("cancel")
            .build();
        dialog.add_responses(&[
            ("cancel", &gettext("_Cancel")),
            ("clear", &gettext("_Clear")),
        ]);
        dialog.set_response_appearance("clear", adw::ResponseAppearance::Destructive);
        dialog.connect_response(None, |_, response| {
            let app = gtk::gio::Application::default()
                .and_then(|a| a.downcast::<crate::application::ClippedApplication>().ok());
            if let (true, Some(app)) = (response == "clear", app) {
                app.clear_history();
            }
        });
        dialog.present(Some(self));
    }
}
