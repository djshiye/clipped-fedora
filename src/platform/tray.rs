//! Status icon (StatusNotifierItem) with a quick-access menu: the recent
//! clips are the menu, one click pastes. This is the macOS menu-bar pattern
//! and, on Wayland, the only popup an app can anchor to the panel.

use ksni::{
    MenuItem, Tray, TrayMethods,
    menu::{CheckmarkItem, StandardItem},
};
use unicode_segmentation::UnicodeSegmentation;

use crate::{config, i18n::gettext};

/// How many clips the menu lists (pinned first, then newest).
pub const MENU_ITEMS: usize = 8;
const LABEL_MAX_GRAPHEMES: usize = 36;

#[derive(Debug, Clone)]
pub enum TrayEvent {
    Toggle,
    Paste([u8; 32]),
    TogglePause,
    Preferences,
    Quit,
}

/// A snapshot of one history entry, safe to hand to the tray thread.
#[derive(Debug, Clone)]
pub struct TrayItem {
    pub hash: [u8; 32],
    pub label: String,
    pub pinned: bool,
    /// Small PNG for image entries.
    pub icon_png: Option<Vec<u8>>,
}

/// Everything the menu shows, snapshotted on the main thread.
#[derive(Debug, Clone, Default)]
pub struct TrayState {
    pub items: Vec<TrayItem>,
    pub paused: bool,
    /// dbusmenu keys of the bound shortcut, e.g. ["Super", "Shift", "v"].
    pub shortcut: Option<Vec<String>>,
}

pub struct ClipperinoTray {
    tx: async_channel::Sender<TrayEvent>,
    pub state: TrayState,
}

impl ClipperinoTray {
    fn send(&self, ev: TrayEvent) {
        self.tx.try_send(ev).ok();
    }
}

/// One line, bounded, with menu mnemonics escaped.
pub fn menu_label(preview: &str) -> String {
    let mut g = preview.graphemes(true);
    let mut out: String = g.by_ref().take(LABEL_MAX_GRAPHEMES).collect();
    if g.next().is_some() {
        out.push('…');
    }
    out.replace('_', "__")
}

impl Tray for ClipperinoTray {
    // Left click opens the menu (the quick-access panel); middle click opens the window.
    const MENU_ON_ACTIVATE: bool = true;

    fn id(&self) -> String {
        config::APP_ID.into()
    }

    fn title(&self) -> String {
        config::APP_NAME.into()
    }

    fn icon_name(&self) -> String {
        format!("{}-symbolic", config::APP_ID)
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: config::APP_NAME.into(),
            description: if self.state.paused {
                gettext("Recording paused")
            } else {
                gettext("Click for recent clips")
            },
            ..Default::default()
        }
    }

    fn secondary_activate(&mut self, _x: i32, _y: i32) {
        self.send(TrayEvent::Toggle);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let items = &self.state.items;
        let mut menu: Vec<MenuItem<Self>> = Vec::with_capacity(items.len() + 10);

        let header = |label: String| -> MenuItem<Self> {
            StandardItem {
                label,
                enabled: false,
                ..Default::default()
            }
            .into()
        };
        let clip = |item: &TrayItem| -> MenuItem<Self> {
            let hash = item.hash;
            // The Shell draws item icons on the right edge, so a type icon on
            // every row reads as clutter: only pins and thumbnails get one.
            let icon_name = if item.pinned && item.icon_png.is_none() {
                "view-pin-symbolic".into()
            } else {
                String::new()
            };
            StandardItem {
                label: item.label.clone(),
                icon_name,
                icon_data: item.icon_png.clone().unwrap_or_default(),
                activate: Box::new(move |t: &mut Self| t.send(TrayEvent::Paste(hash))),
                ..Default::default()
            }
            .into()
        };

        let (pinned, recent): (Vec<&TrayItem>, Vec<&TrayItem>) =
            items.iter().partition(|i| i.pinned);
        if items.is_empty() {
            menu.push(header(gettext("No clips yet. Copy something.")));
        }
        menu.extend(pinned.iter().map(|i| clip(i)));
        if !pinned.is_empty() && !recent.is_empty() {
            menu.push(MenuItem::Separator);
        }
        menu.extend(recent.iter().map(|i| clip(i)));

        menu.push(MenuItem::Separator);
        menu.push(
            StandardItem {
                label: gettext("Open Clipperino"),
                icon_name: "edit-paste-symbolic".into(),
                shortcut: self.state.shortcut.clone().into_iter().collect(),
                activate: Box::new(|t: &mut Self| t.send(TrayEvent::Toggle)),
                ..Default::default()
            }
            .into(),
        );
        menu.push(
            CheckmarkItem {
                label: gettext("Pause Recording"),
                checked: self.state.paused,
                activate: Box::new(|t: &mut Self| t.send(TrayEvent::TogglePause)),
                ..Default::default()
            }
            .into(),
        );
        menu.push(MenuItem::Separator);
        menu.push(
            StandardItem {
                label: gettext("Preferences"),
                icon_name: "emblem-system-symbolic".into(),
                activate: Box::new(|t: &mut Self| t.send(TrayEvent::Preferences)),
                ..Default::default()
            }
            .into(),
        );
        menu.push(
            StandardItem {
                label: gettext("Quit"),
                icon_name: "application-exit-symbolic".into(),
                activate: Box::new(|t: &mut Self| t.send(TrayEvent::Quit)),
                ..Default::default()
            }
            .into(),
        );
        menu
    }
}

/// Register the item with the tray host. Fails (without side effects) when
/// there is no StatusNotifierWatcher on the session bus.
pub async fn spawn(
    tx: async_channel::Sender<TrayEvent>,
    state: TrayState,
) -> Result<ksni::Handle<ClipperinoTray>, ksni::Error> {
    ClipperinoTray { tx, state }.spawn().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_is_bounded_and_escaped() {
        assert_eq!(menu_label("snake_case"), "snake__case");
        let long = "x".repeat(100);
        let l = menu_label(&long);
        assert_eq!(l.graphemes(true).count(), LABEL_MAX_GRAPHEMES + 1);
        assert!(l.ends_with('…'));
    }
}
