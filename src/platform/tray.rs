//! Status icon (StatusNotifierItem) with a quick-access menu: the recent
//! clips are the menu, one click pastes. This is the macOS menu-bar pattern
//! and, on Wayland, the only popup an app can anchor to the panel.

use ksni::{MenuItem, Tray, TrayMethods, menu::StandardItem};
use unicode_segmentation::UnicodeSegmentation;

use crate::{config, i18n::gettext};

/// How many clips the menu lists (pinned first, then newest).
pub const MENU_ITEMS: usize = 10;
const LABEL_MAX_GRAPHEMES: usize = 44;

#[derive(Debug, Clone)]
pub enum TrayEvent {
    Toggle,
    Paste([u8; 32]),
    ClearHistory,
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

pub struct ClippedTray {
    tx: async_channel::Sender<TrayEvent>,
    pub items: Vec<TrayItem>,
}

impl ClippedTray {
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

impl Tray for ClippedTray {
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
            description: gettext("Click for recent clips"),
            ..Default::default()
        }
    }

    fn secondary_activate(&mut self, _x: i32, _y: i32) {
        self.send(TrayEvent::Toggle);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let mut menu: Vec<MenuItem<Self>> = Vec::with_capacity(self.items.len() + 6);

        if self.items.is_empty() {
            menu.push(
                StandardItem {
                    label: gettext("No clips yet"),
                    enabled: false,
                    ..Default::default()
                }
                .into(),
            );
        }
        for item in &self.items {
            let hash = item.hash;
            menu.push(
                StandardItem {
                    label: item.label.clone(),
                    icon_name: if item.pinned && item.icon_png.is_none() {
                        "view-pin-symbolic".into()
                    } else {
                        String::new()
                    },
                    icon_data: item.icon_png.clone().unwrap_or_default(),
                    activate: Box::new(move |t: &mut Self| t.send(TrayEvent::Paste(hash))),
                    ..Default::default()
                }
                .into(),
            );
        }
        menu.push(MenuItem::Separator);
        menu.push(
            StandardItem {
                label: gettext("Open Clipped"),
                shortcut: vec![vec!["Super".into(), "Shift".into(), "v".into()]],
                activate: Box::new(|t: &mut Self| t.send(TrayEvent::Toggle)),
                ..Default::default()
            }
            .into(),
        );
        menu.push(
            StandardItem {
                label: gettext("Clear History…"),
                enabled: !self.items.is_empty(),
                activate: Box::new(|t: &mut Self| t.send(TrayEvent::ClearHistory)),
                ..Default::default()
            }
            .into(),
        );
        menu.push(MenuItem::Separator);
        menu.push(
            StandardItem {
                label: gettext("Preferences"),
                activate: Box::new(|t: &mut Self| t.send(TrayEvent::Preferences)),
                ..Default::default()
            }
            .into(),
        );
        menu.push(
            StandardItem {
                label: gettext("Quit"),
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
    items: Vec<TrayItem>,
) -> Result<ksni::Handle<ClippedTray>, ksni::Error> {
    ClippedTray { tx, items }.spawn().await
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
