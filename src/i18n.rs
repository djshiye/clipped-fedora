//! Translation helpers. Blueprint strings use `_()`; Rust strings use `gettext`.
pub use gettextrs::gettext;

pub fn init() {
    use gettextrs::{
        LocaleCategory, bind_textdomain_codeset, bindtextdomain, setlocale, textdomain,
    };
    setlocale(LocaleCategory::LcAll, "");
    bindtextdomain(crate::config::GETTEXT_DOMAIN, crate::config::LOCALEDIR).ok();
    bind_textdomain_codeset(crate::config::GETTEXT_DOMAIN, "UTF-8").ok();
    textdomain(crate::config::GETTEXT_DOMAIN).ok();
}
