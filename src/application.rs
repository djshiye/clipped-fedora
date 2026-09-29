use std::{cell::OnceCell, rc::Rc};

use adw::{prelude::*, subclass::prelude::*};
use gtk::{gio, glib};

use crate::{
    config,
    platform::portal::PortalSession,
    preferences::ClipperinoPreferences,
    settings::{self, settings},
    window::ClipperinoWindow,
};

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct ClipperinoApplication {
        pub window: OnceCell<ClipperinoWindow>,
        pub hold_guard: OnceCell<gio::ApplicationHoldGuard>,
        pub shortcut_bound: std::cell::Cell<bool>,
        pub background_setup: std::cell::Cell<bool>,
        pub tray: std::cell::RefCell<Option<ksni::Handle<crate::platform::tray::ClipperinoTray>>>,
        pub tray_refresh_pending: std::cell::Cell<bool>,
        /// PNG menu icons by content hash, so images are encoded once.
        pub tray_icon_cache: std::cell::RefCell<std::collections::HashMap<[u8; 32], Vec<u8>>>,
        /// `None` until the first attempt; `Some(false)` when no tray host exists.
        pub tray_available: std::cell::Cell<Option<bool>>,
        /// The toggle shortcut as bound in GNOME; `None` when removed.
        pub shortcut: std::cell::RefCell<Option<crate::platform::shortcut::Trigger>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ClipperinoApplication {
        const NAME: &'static str = "ClipperinoApplication";
        type Type = super::ClipperinoApplication;
        type ParentType = adw::Application;
    }

    impl ObjectImpl for ClipperinoApplication {}

    impl ApplicationImpl for ClipperinoApplication {
        fn startup(&self) {
            self.parent_startup();
            let app = self.obj();
            // Shown until the portal reports the real binding.
            self.shortcut
                .replace(Some(crate::platform::shortcut::Trigger::default_trigger()));
            app.setup_css();
            app.setup_actions();
            // Keep running without a visible window: we are a background service.
            self.hold_guard.set(app.hold()).ok();
            migrate_from_clipped();
            let window = ClipperinoWindow::new(&*app);
            self.window.set(window.clone()).ok();
            // First run: explain before triggering GNOME's remote-desktop dialog.
            // Later runs: the stored token restores the session silently.
            window.grant_button().connect_clicked(glib::clone!(
                #[weak(rename_to = app)]
                app,
                #[weak]
                window,
                move |_| app.connect_portal(window)
            ));
            // Debug builds: CLIPPERINO_DEBUG_NO_PORTAL=1 skips the portal, so the
            // list can be exercised in a nested or headless session.
            #[cfg(debug_assertions)]
            let skip_portal = std::env::var_os("CLIPPERINO_DEBUG_NO_PORTAL").is_some();
            #[cfg(not(debug_assertions))]
            let skip_portal = false;
            if skip_portal {
                tracing::warn!("CLIPPERINO_DEBUG_NO_PORTAL set: no clipboard access this run");
            } else if settings().string(settings::RESTORE_TOKEN).is_empty() {
                window.show_permission_page();
            } else {
                app.connect_portal(window.clone());
            }
            app.setup_tray();
        }

        fn activate(&self) {
            self.parent_activate();
            if let Some(window) = self.window.get() {
                window.present();
                window.focus_search();
            }
        }
    }

    impl GtkApplicationImpl for ClipperinoApplication {}
    impl AdwApplicationImpl for ClipperinoApplication {}
}

glib::wrapper! {
    pub struct ClipperinoApplication(ObjectSubclass<imp::ClipperinoApplication>)
        @extends adw::Application, gtk::Application, gio::Application,
        @implements gio::ActionGroup, gio::ActionMap;
}

impl Default for ClipperinoApplication {
    fn default() -> Self {
        Self::new()
    }
}

impl ClipperinoApplication {
    pub fn new() -> Self {
        glib::Object::builder()
            .property("application-id", config::APP_ID)
            .property("resource-base-path", config::RESOURCE_PREFIX)
            .build()
    }

    fn setup_css(&self) {
        let provider = gtk::CssProvider::new();
        provider.load_from_resource(&format!("{}/style.css", config::RESOURCE_PREFIX));
        // One notch above the user stylesheet (~/.config/gtk-4.0/gtk.css): a
        // third-party theme's generic `row` rules otherwise erase the card
        // design. These rules only target Clipperino's own widgets and classes.
        gtk::style_context_add_provider_for_display(
            &gtk::gdk::Display::default().expect("no display"),
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_USER + 1,
        );
    }

    fn setup_actions(&self) {
        let quit = gio::ActionEntry::builder("quit")
            .activate(|app: &Self, _, _| app.quit())
            .build();
        let about = gio::ActionEntry::builder("about")
            .activate(|app: &Self, _, _| app.show_about())
            .build();
        let shortcuts = gio::ActionEntry::builder("shortcuts")
            .activate(|app: &Self, _, _| {
                let builder = gtk::Builder::from_resource(&format!(
                    "{}/ui/shortcuts.ui",
                    config::RESOURCE_PREFIX
                ));
                // Show the key actually bound in GNOME, not the default.
                if let Some(item) = builder.object::<adw::ShortcutsItem>("toggle_item") {
                    item.set_accelerator(
                        &app.shortcut().map(|t| t.accelerator()).unwrap_or_default(),
                    );
                }
                if let Some(dialog) = builder.object::<adw::ShortcutsDialog>("shortcuts_dialog") {
                    dialog.present(app.active_window().as_ref());
                }
            })
            .build();
        // Used by scripted checks and the tray; hides without quitting.
        let hide = gio::ActionEntry::builder("hide")
            .activate(|app: &Self, _, _| {
                if let Some(w) = app.imp().window.get() {
                    w.hide_window();
                }
            })
            .build();
        let preferences = gio::ActionEntry::builder("preferences")
            .activate(|app: &Self, _, _| {
                ClipperinoPreferences::default().present(app.active_window().as_ref());
            })
            .build();
        self.add_action_entries([quit, about, preferences, shortcuts, hide]);
        // Stateful toggle backed by the setting: menu, banner and tray share it.
        self.add_action(&settings().create_action(settings::PAUSE_RECORDING));
        self.set_accels_for_action("app.shortcuts", &["<Control>question"]);

        self.set_accels_for_action("app.quit", &["<Control>q"]);
        self.set_accels_for_action("app.preferences", &["<Control>comma"]);
        self.set_accels_for_action("window.close", &["Escape"]);
        self.set_accels_for_action("win.show-selected", &["<Control>d"]);
    }

    /// Open the portal session (permission dialog on first run) and start
    /// monitoring. On failure the window shows the permission page with retry.
    fn connect_portal(&self, window: ClipperinoWindow) {
        let token =
            Some(settings().string(settings::RESTORE_TOKEN).to_string()).filter(|t| !t.is_empty());
        glib::spawn_future_local(glib::clone!(
            #[weak]
            window,
            #[weak(rename_to = app)]
            self,
            async move {
                match PortalSession::connect(token).await {
                    Ok(session) => {
                        if let Some(tok) = session.restore_token() {
                            settings().set_string(settings::RESTORE_TOKEN, tok).ok();
                        }
                        let session = Rc::new(session);
                        window.set_portal(session.clone());
                        app.bind_shortcut_once();
                        app.setup_background_once();
                        glib::spawn_future_local(glib::clone!(
                            #[strong]
                            session,
                            async move {
                                if let Err(e) = session.serve_transfers().await {
                                    tracing::error!("clipboard transfer server stopped: {e}");
                                }
                            }
                        ));
                        let (tx, rx) = async_channel::unbounded();
                        glib::spawn_future_local(glib::clone!(
                            #[weak]
                            window,
                            async move {
                                while let Ok(event) = rx.recv().await {
                                    window.handle_clip_event(event);
                                }
                            }
                        ));
                        let started = std::time::Instant::now();
                        let monitor = std::pin::pin!(session.monitor(tx));
                        let closed = std::pin::pin!(session.closed());
                        match futures_util::future::select(monitor, closed).await {
                            futures_util::future::Either::Left((Err(e), _)) => {
                                tracing::error!("clipboard monitor stopped: {e}")
                            }
                            futures_util::future::Either::Left((Ok(()), _)) => {
                                tracing::warn!("clipboard monitor ended")
                            }
                            futures_util::future::Either::Right(_) => {
                                tracing::warn!("portal session closed")
                            }
                        }
                        // Recording has stopped: say so instead of failing silently.
                        window.clear_portal();
                        // A session that ran a while was likely ended by a shell or
                        // portal restart; the stored token reconnects silently. One
                        // that died at once would only loop, so wait for the user.
                        if started.elapsed() > RECONNECT_AFTER {
                            glib::timeout_future_seconds(3).await;
                            app.connect_portal(window);
                        }
                    }
                    Err(e) => {
                        tracing::warn!("portal session failed: {e}");
                        window.show_permission_page();
                    }
                }
            }
        ));
    }

    fn setup_background_once(&self) {
        if self.imp().background_setup.replace(true) {
            return;
        }
        self.setup_background();
    }

    fn bind_shortcut_once(&self) {
        if self.imp().shortcut_bound.replace(true) {
            return;
        }
        self.bind_shortcut();
    }

    /// Bind the global shortcut and toggle the window on every activation.
    fn bind_shortcut(&self) {
        use crate::platform::shortcut::ShortcutEvent;
        let (tx, rx) = async_channel::unbounded::<ShortcutEvent>();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = app)]
            self,
            async move {
                while let Ok(event) = rx.recv().await {
                    match event {
                        ShortcutEvent::Activated(token) => app.toggle_window(token.as_deref()),
                        ShortcutEvent::Trigger(trigger) => {
                            app.imp().shortcut.replace(trigger);
                            app.schedule_tray_refresh();
                        }
                    }
                }
            }
        ));
        glib::spawn_future_local(async move {
            if let Err(e) = crate::platform::shortcut::run(tx).await {
                tracing::warn!("global shortcut unavailable: {e}");
            }
        });
    }

    pub fn clear_history(&self) {
        if let Some(window) = self.imp().window.get() {
            window.clear_history();
        }
    }

    /// Ask the Background portal for autostart according to the setting, now
    /// and whenever the setting changes. GNOME lists us under Settings › Apps.
    fn setup_background(&self) {
        let apply = |enabled: bool| {
            glib::spawn_future_local(async move {
                let reason =
                    crate::i18n::gettext("Keep clipboard history while the window is closed");
                let request = ashpd::desktop::background::Background::request()
                    .reason(reason.as_str())
                    .auto_start(enabled)
                    .command(["clipperino", "--gapplication-service"])
                    .dbus_activatable(false);
                match request.send().await.and_then(|r| r.response()) {
                    Ok(r) => tracing::info!(
                        background = r.run_in_background(),
                        autostart = r.auto_start(),
                        "background portal"
                    ),
                    Err(e) => tracing::warn!("background portal: {e}"),
                }
            });
        };
        let s = settings();
        apply(s.boolean(settings::RUN_IN_BACKGROUND));
        s.connect_changed(Some(settings::RUN_IN_BACKGROUND), move |s, key| {
            apply(s.boolean(key))
        });
    }

    /// Show or hide the status icon according to the setting, now and on change.
    fn setup_tray(&self) {
        let (tx, rx) = async_channel::unbounded::<crate::platform::tray::TrayEvent>();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = app)]
            self,
            async move {
                use crate::platform::tray::TrayEvent;
                while let Ok(ev) = rx.recv().await {
                    match ev {
                        TrayEvent::Toggle => app.toggle_window(None),
                        TrayEvent::Paste(hash) => {
                            if let Some(w) = app.imp().window.get() {
                                w.paste_from_tray(hash);
                            }
                        }
                        TrayEvent::TogglePause => {
                            let s = settings();
                            s.set_boolean(
                                settings::PAUSE_RECORDING,
                                !s.boolean(settings::PAUSE_RECORDING),
                            )
                            .ok();
                        }
                        TrayEvent::ClearHistory => app.confirm_clear_history(),
                        TrayEvent::Preferences => {
                            if let Some(w) = app.imp().window.get() {
                                w.present();
                            }
                            app.activate_action("preferences", None);
                        }
                        TrayEvent::Quit => app.quit(),
                    }
                }
            }
        ));
        let apply = glib::clone!(
            #[weak(rename_to = app)]
            self,
            #[strong]
            tx,
            move |enabled: bool| {
                if !enabled {
                    if let Some(handle) = app.imp().tray.take() {
                        glib::spawn_future_local(async move {
                            handle.shutdown().await;
                        });
                    }
                    return;
                }
                if app.imp().tray.borrow().is_some() {
                    return;
                }
                let tx = tx.clone();
                let state = app.tray_state();
                glib::spawn_future_local(async move {
                    match crate::platform::tray::spawn(tx, state).await {
                        Ok(handle) => {
                            app.imp().tray.replace(Some(handle));
                            app.imp().tray_available.set(Some(true));
                            tracing::info!("status icon registered");
                        }
                        Err(e) => {
                            app.imp().tray_available.set(Some(false));
                            tracing::info!("no status icon: {e}");
                        }
                    }
                });
            }
        );
        let s = settings();
        apply(s.boolean(settings::SHOW_TRAY_ICON));
        s.connect_changed(Some(settings::SHOW_TRAY_ICON), move |s, key| {
            apply(s.boolean(key))
        });
        s.connect_changed(
            Some(settings::PAUSE_RECORDING),
            glib::clone!(
                #[weak(rename_to = app)]
                self,
                move |_, _| app.schedule_tray_refresh()
            ),
        );

        // Keep the menu in sync with the history, coalescing bursts of changes.
        if let Some(window) = self.imp().window.get() {
            window
                .history()
                .set_on_change(std::rc::Rc::new(glib::clone!(
                    #[weak(rename_to = app)]
                    self,
                    move || app.schedule_tray_refresh()
                )));
        }
    }

    fn schedule_tray_refresh(&self) {
        if self.imp().tray_refresh_pending.replace(true) {
            return;
        }
        glib::timeout_add_local_once(
            std::time::Duration::from_millis(200),
            glib::clone!(
                #[weak(rename_to = app)]
                self,
                move || {
                    app.imp().tray_refresh_pending.set(false);
                    let Some(handle) = app.imp().tray.borrow().clone() else {
                        return;
                    };
                    let state = app.tray_state();
                    glib::spawn_future_local(async move {
                        handle.update(|t| t.state = state).await;
                    });
                }
            ),
        );
    }

    /// Everything the tray menu shows, as plain data for its thread.
    fn tray_state(&self) -> crate::platform::tray::TrayState {
        crate::platform::tray::TrayState {
            items: self.tray_items(),
            paused: settings().boolean(settings::PAUSE_RECORDING),
            shortcut: self.shortcut().map(|t| t.keys),
        }
    }

    /// The recent clips as plain data for the tray menu.
    fn tray_items(&self) -> Vec<crate::platform::tray::TrayItem> {
        use crate::{model::ClipKind, platform::tray};
        let Some(window) = self.imp().window.get() else {
            return Vec::new();
        };
        let mut cache = self.imp().tray_icon_cache.borrow_mut();
        let items: Vec<tray::TrayItem> = window
            .history()
            .recent(tray::MENU_ITEMS)
            .iter()
            .map(|item| {
                let hash = item.hash();
                let icon_png = if item.kind() == ClipKind::Image {
                    item.image_path().and_then(|path| {
                        if let Some(png) = cache.get(&hash) {
                            return Some(png.clone());
                        }
                        let png = crate::model::images::menu_icon_png(&path, 48)?;
                        cache.insert(hash, png.clone());
                        Some(png)
                    })
                } else {
                    None
                };
                let flavor = crate::model::Flavor::of(item.kind(), item.text().as_deref());
                // Images all read "Image · W × H"; the time tells them apart.
                // Absolute, because the menu is not refreshed as time passes.
                let label = if item.kind() == ClipKind::Image {
                    format!(
                        "{} · {}",
                        item.preview(),
                        crate::ui::short_when(item.timestamp())
                    )
                } else {
                    item.preview()
                };
                tray::TrayItem {
                    hash,
                    icon_name: match flavor {
                        crate::model::Flavor::Plain => String::new(),
                        f => f.icon_name().to_owned(),
                    },
                    label: tray::menu_label(&label),
                    pinned: item.pinned(),
                    icon_png,
                }
            })
            .collect();
        let keep: std::collections::HashSet<[u8; 32]> = items.iter().map(|i| i.hash).collect();
        cache.retain(|h, _| keep.contains(h));
        items
    }

    /// Ask before clearing everything (HIG: warn on irreversible loss).
    pub fn confirm_clear_history(&self) {
        let Some(window) = self.imp().window.get() else {
            return;
        };
        window.present();
        let dialog = adw::AlertDialog::builder()
            .heading(crate::i18n::gettext("Clear Clipboard History?"))
            .body(crate::i18n::gettext(
                "All entries, including pinned items, will be permanently deleted.",
            ))
            .default_response("cancel")
            .close_response("cancel")
            .build();
        dialog.add_responses(&[
            ("cancel", &crate::i18n::gettext("_Cancel")),
            ("clear", &crate::i18n::gettext("_Clear")),
        ]);
        dialog.set_response_appearance("clear", adw::ResponseAppearance::Destructive);
        dialog.connect_response(
            None,
            glib::clone!(
                #[weak(rename_to = app)]
                self,
                move |_, response| {
                    if response == "clear" {
                        app.clear_history();
                    }
                }
            ),
        );
        dialog.present(Some(window));
    }

    /// The toggle shortcut as currently bound, if any.
    pub fn shortcut(&self) -> Option<crate::platform::shortcut::Trigger> {
        self.imp().shortcut.borrow().clone()
    }

    /// `Some(false)` means the desktop has no tray host.
    pub fn tray_available(&self) -> Option<bool> {
        self.imp().tray_available.get()
    }

    pub fn toggle_window(&self, activation_token: Option<&str>) {
        let Some(window) = self.imp().window.get() else {
            return;
        };
        if window.is_visible() && window.is_active() {
            window.set_visible(false);
        } else {
            if let Some(token) = activation_token {
                window.set_startup_id(token);
            }
            window.present();
            window.focus_search();
        }
    }

    fn show_about(&self) {
        let dialog = adw::AboutDialog::builder()
            .application_name(config::APP_NAME)
            .application_icon(config::APP_ID)
            .version(config::VERSION)
            .developer_name("djshiye")
            .license_type(gtk::License::MitX11)
            .website("https://github.com/djshiye/Clipperino")
            .issue_url("https://github.com/djshiye/Clipperino/issues")
            .build();
        dialog.present(self.active_window().as_ref());
    }
}

/// Reconnect automatically only when the lost session had run at least this long.
const RECONNECT_AFTER: std::time::Duration = std::time::Duration::from_secs(30);

/// Clipperino was called Clipped before 1.1. Carry the history over once and
/// drop the old login item, whose binary the package upgrade removed. Settings
/// stay behind: the portal grants they held are tied to the old app ID.
fn migrate_from_clipped() {
    let data = glib::user_data_dir();
    let (old, new) = (data.join("clipped"), data.join("clipperino"));
    if old.is_dir() && !new.exists() {
        match std::fs::rename(&old, &new) {
            Ok(()) => tracing::info!("moved history from {}", old.display()),
            Err(e) => tracing::warn!("could not move history from {}: {e}", old.display()),
        }
    }
    let autostart = glib::user_config_dir()
        .join("autostart")
        .join("io.github.djshiye.Clipped.desktop");
    std::fs::remove_file(autostart).ok();
}
