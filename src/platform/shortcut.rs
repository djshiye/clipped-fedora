//! GlobalShortcuts portal: binds the toggle shortcut and reports activations
//! and the key the user actually assigned.

use ashpd::desktop::{
    CreateSessionOptions,
    global_shortcuts::{BindShortcutsOptions, GlobalShortcuts, NewShortcut, Shortcut},
};
use futures_util::StreamExt;

pub const TOGGLE_ID: &str = "toggle";
/// GNOME already uses <Super>v for the notification list.
pub const DEFAULT_TRIGGER: &str = "<Super><Shift>v";

#[derive(Debug, Clone)]
pub enum ShortcutEvent {
    /// Carries GNOME's xdg-activation token, if any.
    Activated(Option<String>),
    /// The trigger bound now (`None`: the user removed it).
    Trigger(Option<Trigger>),
}

/// A key combination, for display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trigger {
    /// "Super+Shift+V"
    pub label: String,
    /// dbusmenu form for the tray: ["Super", "Shift", "v"]
    pub keys: Vec<String>,
}

impl Trigger {
    pub fn default_trigger() -> Self {
        parse_trigger(DEFAULT_TRIGGER).expect("valid default trigger")
    }

    /// GTK accelerator syntax, e.g. `<Super><Shift>v`.
    pub fn accelerator(&self) -> String {
        let (key, mods) = self.keys.split_last().expect("a trigger has a key");
        mods.iter().map(|m| format!("<{m}>")).collect::<String>() + key
    }
}

/// Parses a portal trigger description. GNOME reports it as
/// `Press <Shift><Super>v`; bare accelerators are accepted too.
pub fn parse_trigger(description: &str) -> Option<Trigger> {
    // Several bindings are joined with " or "; show the first.
    let first = description.split(" or ").next()?.trim();
    // Skip the (possibly translated) verb: start at the first modifier, or
    // take the last word when there is none.
    let mut rest = match first.find('<') {
        Some(i) => &first[i..],
        None => first.rsplit(' ').next()?,
    };
    let mut mods: Vec<&str> = Vec::new();
    while let Some(after) = rest.strip_prefix('<') {
        let (name, tail) = after.split_once('>')?;
        let canonical = match name.to_ascii_lowercase().as_str() {
            "super" | "meta" | "mod4" => "Super",
            "control" | "ctrl" | "primary" => "Control",
            "alt" | "mod1" => "Alt",
            "shift" => "Shift",
            _ => return None,
        };
        if !mods.contains(&canonical) {
            mods.push(canonical);
        }
        rest = tail;
    }
    let key = rest.trim();
    if key.is_empty() {
        return None;
    }
    // Stable order regardless of how the portal spells it.
    const ORDER: [&str; 4] = ["Super", "Control", "Alt", "Shift"];
    mods.sort_by_key(|m| ORDER.iter().position(|o| o == m));
    let key_label = if key.chars().count() == 1 {
        key.to_uppercase()
    } else {
        key.to_owned()
    };
    let label = mods
        .iter()
        .map(|m| if *m == "Control" { "Ctrl" } else { m })
        .chain([key_label.as_str()])
        .collect::<Vec<_>>()
        .join("+");
    let keys = mods
        .iter()
        .map(|m| m.to_string())
        .chain([key.to_owned()])
        .collect();
    Some(Trigger { label, keys })
}

fn trigger_of(shortcuts: &[Shortcut]) -> Option<Trigger> {
    shortcuts
        .iter()
        .find(|s| s.id() == TOGGLE_ID)
        .and_then(|s| parse_trigger(s.trigger_description()))
}

/// Binds the shortcut (GNOME shows its binding dialog the first time), then
/// reports the bound trigger, every change to it, and every activation.
pub async fn run(tx: async_channel::Sender<ShortcutEvent>) -> ashpd::Result<()> {
    let gs = GlobalShortcuts::new().await?;
    let session = gs.create_session(CreateSessionOptions::default()).await?;
    let bound = gs
        .bind_shortcuts(
            &session,
            &[
                NewShortcut::new(TOGGLE_ID, crate::i18n::gettext("Show clipboard history"))
                    .preferred_trigger(DEFAULT_TRIGGER),
            ],
            None,
            BindShortcutsOptions::default(),
        )
        .await?
        .response()?;
    for s in bound.shortcuts() {
        tracing::info!(
            id = s.id(),
            trigger = s.trigger_description(),
            "shortcut bound"
        );
    }
    tx.send(ShortcutEvent::Trigger(trigger_of(bound.shortcuts())))
        .await
        .ok();

    let activated = gs.receive_activated().await?.fuse();
    let changed = gs.receive_shortcuts_changed().await?.fuse();
    futures_util::pin_mut!(activated);
    futures_util::pin_mut!(changed);
    loop {
        let event = futures_util::select! {
            ev = activated.next() => {
                let Some(ev) = ev else { break };
                if ev.shortcut_id() != TOGGLE_ID {
                    continue;
                }
                // GNOME attaches an xdg-activation token; with it the compositor lets us take focus.
                let token = ev
                    .options()
                    .get("activation_token")
                    .and_then(|v| v.downcast_ref::<&str>().ok())
                    .map(str::to_owned);
                tracing::info!(has_token = token.is_some(), "shortcut activated");
                ShortcutEvent::Activated(token)
            }
            ev = changed.next() => {
                let Some(ev) = ev else { break };
                let trigger = trigger_of(ev.shortcuts());
                tracing::info!(trigger = ?trigger.as_ref().map(|t| &t.label), "shortcut changed");
                ShortcutEvent::Trigger(trigger)
            }
        };
        if tx.send(event).await.is_err() {
            break;
        }
    }
    // `session` lives until here, keeping the binding alive.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_gnome_descriptions() {
        let t = parse_trigger("Press <Shift><Super>v").unwrap();
        assert_eq!(t.label, "Super+Shift+V");
        assert_eq!(t.keys, ["Super", "Shift", "v"]);
        let t = parse_trigger("<Control><Alt>space or <Super>c").unwrap();
        assert_eq!(t.label, "Ctrl+Alt+space");
        assert_eq!(t.keys, ["Control", "Alt", "space"]);
        assert_eq!(Trigger::default_trigger().label, "Super+Shift+V");
        assert_eq!(Trigger::default_trigger().accelerator(), "<Super><Shift>v");
        assert!(parse_trigger("").is_none());
        assert!(parse_trigger("Press <Shift>").is_none());
        assert_eq!(
            parse_trigger("Drücken Sie <Super>v").unwrap().label,
            "Super+V"
        );
        assert_eq!(parse_trigger("Press F12").unwrap().label, "F12");
    }
}
