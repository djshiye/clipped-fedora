mod application;
mod config;
mod i18n;
mod model;
mod platform;
mod preferences;
mod settings;
mod storage;
mod ui;
mod window;

use gtk::{gio, prelude::*};

fn main() -> gtk::glib::ExitCode {
    // Development convenience: use the schema compiled by build.rs when the
    // app is not installed. Release builds rely on the system schema dir.
    #[cfg(debug_assertions)]
    if std::env::var_os("GSETTINGS_SCHEMA_DIR").is_none() {
        // SAFETY: called before any other thread exists.
        unsafe { std::env::set_var("GSETTINGS_SCHEMA_DIR", concat!(env!("OUT_DIR"), "/schemas")) };
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"))
                // ashpd's request/session proxies always log a harmless cache warning
                .add_directive("zbus::proxy=error".parse().unwrap()),
        )
        .init();

    i18n::init();
    gio::resources_register_include!("clipped.gresource").expect("failed to register resources");

    let app = application::ClippedApplication::new();
    app.run()
}
