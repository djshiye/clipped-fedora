//! GlobalShortcuts portal: binds the toggle shortcut and reports activations.

use ashpd::desktop::{
    CreateSessionOptions,
    global_shortcuts::{BindShortcutsOptions, GlobalShortcuts, NewShortcut},
};
use futures_util::StreamExt;

pub const TOGGLE_ID: &str = "toggle";
/// GNOME already uses <Super>v for the notification list.
pub const DEFAULT_TRIGGER: &str = "<Super><Shift>v";

/// Binds the shortcut (GNOME shows its binding dialog the first time) and then
/// sends `()` on `tx` for every activation until the session ends.
pub async fn run(tx: async_channel::Sender<Option<String>>) -> ashpd::Result<()> {
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

    let activated = gs.receive_activated().await?;
    futures_util::pin_mut!(activated);
    while let Some(ev) = activated.next().await {
        // GNOME attaches an xdg-activation token; with it the compositor lets us take focus.
        let token = ev
            .options()
            .get("activation_token")
            .and_then(|v| v.downcast_ref::<&str>().ok())
            .map(str::to_owned);
        tracing::info!(
            id = ev.shortcut_id(),
            has_token = token.is_some(),
            "shortcut activated"
        );
        if ev.shortcut_id() == TOGGLE_ID && tx.send(token).await.is_err() {
            break;
        }
    }
    // `session` lives until here, keeping the binding alive.
    Ok(())
}
