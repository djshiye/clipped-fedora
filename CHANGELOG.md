# Changelog

## 1.1.0 (2026-09-29)

- Renamed from Clipped to Clipperino: app ID `io.github.djshiye.Clipperino`,
  binary and package `clipperino`. The RPM replaces `clipped` on upgrade, and
  the clipboard history moves to `~/.local/share/clipperino` on first launch.
  GNOME asks for the clipboard permission and the shortcut again, since both
  are granted per app ID.
- The tray menu opens as soon as the icon is clicked. GNOME's AppIndicator
  extension used to wait out the double-click time first, because the status
  item exported an `Activate` method; a vendored ksni (`third_party/ksni`) no
  longer exports it.

## 1.0.0 (2026-09-28)

Complete rewrite in Rust with GTK 4 and libadwaita, targeting Fedora GNOME on Wayland.

- Clipboard monitoring, paste injection, the global shortcut and autostart now go
  through GNOME's desktop portals (RemoteDesktop + Clipboard, GlobalShortcuts,
  Background); no X11, no shell extension required.
- Default shortcut is `Super+Shift+V` (`Super+V` is GNOME's notification list).
- History is stored in SQLite and survives restarts, images included.
- Search, keyboard navigation, `Ctrl+1`–`Ctrl+9`, pin, delete with undo, details view.
- Emoji browser backed by GNOME's localized emoji database; curated symbol browser;
  recently used chips.
- Preferences apply immediately: history size, paste on select, run in background.
- Password-manager clipboard entries are skipped; clipboard content is never logged.
- Adaptive layout: bottom tab bar on narrow windows, list plus preview pane above 700 px.
- Card-based visual design on Adwaita tokens: rounded rows, tinted selection, pill search, hover-revealed controls.
- Status icon (StatusNotifierItem) whose menu lists the ten most recent clips for one-click paste, grouped into Pinned and Recent sections, with image thumbnails and file icons, plus Open, Clear History…, Preferences and Quit; shown when a tray host exists.
- Packaged as an RPM with a spec, man page and AppStream metadata.

### Removed

- Windows and macOS builds, X11 hotkey grabbing, `GtkStatusIcon` tray, the
  remote "announcements" fetch and the `curl` dependency.
