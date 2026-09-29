use adw::{prelude::*, subclass::prelude::*};
use gtk::{CompositeTemplate, glib};

use crate::settings::{self, settings};

mod imp {
    use super::*;

    #[derive(Default, CompositeTemplate)]
    #[template(resource = "/io/github/djshiye/Clipperino/ui/preferences.ui")]
    pub struct ClipperinoPreferences {
        #[template_child]
        pub shortcut_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub max_history_row: TemplateChild<adw::SpinRow>,
        #[template_child]
        pub expire_row: TemplateChild<adw::ComboRow>,
        #[template_child]
        pub pause_row: TemplateChild<adw::SwitchRow>,
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
            s.bind(settings::SHOW_TRAY_ICON, &*self.tray_row, "active")
                .build();
            s.bind(settings::PAUSE_RECORDING, &*self.pause_row, "active")
                .build();
            s.bind(settings::EXPIRE_DAYS, &*self.expire_row, "selected")
                .mapping(|v, _| {
                    let days = v.get::<u32>()?;
                    let i = EXPIRE_CHOICES.iter().position(|&d| d == days).unwrap_or(0);
                    Some((i as u32).to_value())
                })
                .set_mapping(|v, _| {
                    let i = v.get::<u32>().ok()? as usize;
                    Some(EXPIRE_CHOICES.get(i).copied().unwrap_or(0).to_variant())
                })
                .build();
            if let Some(app) = super::app() {
                self.shortcut_row.set_subtitle(
                    &app.shortcut()
                        .map(|t| t.label)
                        .unwrap_or_else(|| crate::i18n::gettext("Not set")),
                );
                if app.tray_available() == Some(false) {
                    self.tray_row.set_subtitle(&crate::i18n::gettext(
                        "No tray found. On GNOME, install the AppIndicator extension",
                    ));
                }
            }
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

/// Days offered by the "Delete Unpinned Items" row, in list order; 0 = never.
const EXPIRE_CHOICES: [u32; 5] = [0, 1, 7, 30, 90];

fn app() -> Option<crate::application::ClipperinoApplication> {
    gtk::gio::Application::default().and_then(|a| a.downcast().ok())
}

impl ClipperinoPreferences {
    fn confirm_clear(&self) {
        if let Some(app) = app() {
            app.confirm_clear_history();
        }
    }
}
