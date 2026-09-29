pub const APP_ID: &str = "io.github.djshiye.Clipperino";
pub const APP_NAME: &str = "Clipperino";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const RESOURCE_PREFIX: &str = "/io/github/djshiye/Clipperino";
pub const GETTEXT_DOMAIN: &str = "clipperino";
/// Set by build-aux/cargo.sh from Meson's localedir; defaults to the system path.
pub const LOCALEDIR: &str = match option_env!("CLIPPERINO_LOCALEDIR") {
    Some(dir) => dir,
    None => "/usr/share/locale",
};
