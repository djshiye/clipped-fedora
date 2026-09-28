pub const APP_ID: &str = "io.github.djshiye.Clipped";
pub const APP_NAME: &str = "Clipped";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const RESOURCE_PREFIX: &str = "/io/github/djshiye/Clipped";
pub const GETTEXT_DOMAIN: &str = "clipped";
/// Set by build-aux/cargo.sh from Meson's localedir; defaults to the system path.
pub const LOCALEDIR: &str = match option_env!("CLIPPED_LOCALEDIR") {
    Some(dir) => dir,
    None => "/usr/share/locale",
};
