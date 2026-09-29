//! Human times for rows, section headers, tiles and the tray.

use gtk::glib;

use crate::i18n::gettext;

fn now() -> i64 {
    glib::DateTime::now_local()
        .map(|d| d.to_unix())
        .unwrap_or_default()
}

/// "Just now", "5 min ago", "3 h ago", then `short_when`. Goes stale: refresh it.
pub(crate) fn relative_time(unix: i64) -> String {
    let secs = (now() - unix).max(0);
    match secs {
        0..=59 => gettext("Just now"),
        60..=3599 => gettext("{} min ago").replace("{}", &(secs / 60).to_string()),
        3600..=86_399 => gettext("{} h ago").replace("{}", &(secs / 3600).to_string()),
        _ => short_when(unix),
    }
}

/// Unix time of the most recent local midnight.
pub(crate) fn today_start() -> i64 {
    glib::DateTime::now_local()
        .and_then(|d| glib::DateTime::from_local(d.year(), d.month(), d.day_of_month(), 0, 0, 0.0))
        .map(|d| d.to_unix())
        .unwrap_or_default()
}

/// History sections by day, in display order. Pinned items stay in place
/// (marked by their pin), so Enter still pastes the newest clip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Section {
    Today,
    Yesterday,
    ThisWeek,
    Earlier,
}

impl Section {
    pub fn of(unix: i64, today_start: i64) -> Self {
        const DAY: i64 = 86_400;
        match () {
            _ if unix >= today_start => Section::Today,
            _ if unix >= today_start - DAY => Section::Yesterday,
            _ if unix >= today_start - 6 * DAY => Section::ThisWeek,
            _ => Section::Earlier,
        }
    }

    pub fn title(self) -> String {
        match self {
            Section::Today => gettext("Today"),
            Section::Yesterday => gettext("Yesterday"),
            Section::ThisWeek => gettext("Last 7 Days"),
            Section::Earlier => gettext("Earlier"),
        }
    }
}

/// Compact absolute time that never goes stale: "14:32" today,
/// "Yesterday 14:32", "Mon 14:32" this week, else the date.
pub(crate) fn short_when(unix: i64) -> String {
    let start = today_start();
    let clock = format_local(unix, "%R");
    match Section::of(unix, start) {
        Section::Today => clock,
        Section::Yesterday => format!("{} {clock}", gettext("Yesterday")),
        Section::ThisWeek => format_local(unix, "%a %R"),
        _ => format_local(unix, "%x"),
    }
}

fn format_local(unix: i64, fmt: &str) -> String {
    glib::DateTime::from_unix_local(unix)
        .and_then(|d| d.format(fmt))
        .map(|s| s.to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections_follow_the_calendar() {
        let t = 1_000_000;
        assert_eq!(Section::of(t + 5, t), Section::Today);
        assert_eq!(Section::of(t - 1, t), Section::Yesterday);
        assert_eq!(Section::of(t - 86_401, t), Section::ThisWeek);
        assert_eq!(Section::of(t - 7 * 86_400, t), Section::Earlier);
        assert!(Section::Today < Section::Yesterday && Section::ThisWeek < Section::Earlier);
    }
}
