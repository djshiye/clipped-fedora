//! Optional status icon (StatusNotifierItem). GNOME shows it only with a tray
//! host such as the AppIndicator extension; other desktops show it natively.

use ksni::{MenuItem, Tray, TrayMethods, menu::StandardItem};

use crate::{config, i18n::gettext};

#[derive(Debug, Clone, Copy)]
pub enum TrayEvent {
    Toggle,
    Preferences,
    Quit,
}

pub struct ClippedTray {
    tx: async_channel::Sender<TrayEvent>,
}

impl ClippedTray {
    fn send(&self, ev: TrayEvent) {
        self.tx.try_send(ev).ok();
    }
}

impl Tray for ClippedTray {
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
            description: gettext("Clipboard history"),
            ..Default::default()
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(TrayEvent::Toggle);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            StandardItem {
                label: gettext("Open Clipped"),
                activate: Box::new(|t: &mut Self| t.send(TrayEvent::Toggle)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: gettext("Preferences"),
                activate: Box::new(|t: &mut Self| t.send(TrayEvent::Preferences)),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: gettext("Quit"),
                activate: Box::new(|t: &mut Self| t.send(TrayEvent::Quit)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// Register the item with the tray host. Fails (without side effects) when
/// there is no StatusNotifierWatcher on the session bus.
pub async fn spawn(
    tx: async_channel::Sender<TrayEvent>,
) -> Result<ksni::Handle<ClippedTray>, ksni::Error> {
    ClippedTray { tx }.spawn().await
}
