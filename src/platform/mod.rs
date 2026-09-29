pub mod portal;
pub mod shortcut;
pub mod tray;

/// Running inside Flatpak (the sandbox always provides this file).
pub fn is_sandboxed() -> bool {
    std::path::Path::new("/.flatpak-info").exists()
}
