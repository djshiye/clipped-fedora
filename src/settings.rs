use gtk::gio;

use crate::config;

pub fn settings() -> gio::Settings {
    thread_local! {
        static SETTINGS: gio::Settings = gio::Settings::new(config::APP_ID);
    }
    SETTINGS.with(|s| s.clone())
}

pub const MAX_HISTORY: &str = "max-history";
pub const PASTE_ON_SELECT: &str = "paste-on-select";
pub const RUN_IN_BACKGROUND: &str = "run-in-background";
pub const SHOW_TRAY_ICON: &str = "show-tray-icon";
pub const RESTORE_TOKEN: &str = "restore-token";
pub const WINDOW_WIDTH: &str = "window-width";
pub const WINDOW_HEIGHT: &str = "window-height";
