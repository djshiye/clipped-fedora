//! RemoteDesktop + Clipboard portal session: background clipboard monitoring,
//! own-selection writes and Ctrl+V injection. Verified in docs/SPIKES.md.

use std::{cell::RefCell, os::fd::OwnedFd, time::Duration};

use ashpd::{
    desktop::{
        CreateSessionOptions, PersistMode, Session,
        clipboard::{Clipboard, RequestClipboardOptions, SetSelectionOptions},
        remote_desktop::{
            DeviceType, KeyState, NotifyKeyboardKeycodeOptions, RemoteDesktop,
            SelectDevicesOptions, StartOptions,
        },
    },
    enumflags2::BitFlags,
};
use futures_util::{AsyncReadExt, AsyncWriteExt, StreamExt};
use gtk::glib;

use crate::config;

const PASSWORD_HINT: &str = "x-kde-passwordManagerHint";
/// Preference order when several MIME types are offered.
const PREFERRED_MIMES: [&str; 5] = [
    "image/png",
    "text/uri-list",
    "text/plain;charset=utf-8",
    "UTF8_STRING",
    "text/plain",
];
/// Skip payloads above this (clipboard, not a file manager).
const MAX_BYTES: usize = 20 * 1024 * 1024;

// evdev keycodes
const KEY_LEFTCTRL: i32 = 29;
const KEY_V: i32 = 47;
/// Spike 3: events fired back-to-back lose the modifier; 30 ms works.
const KEY_GAP: Duration = Duration::from_millis(30);

#[derive(Debug)]
pub enum ClipEvent {
    Text(String),
    Image { mime: String, bytes: Vec<u8> },
    Uris(Vec<String>),
}

const TEXT_MIMES: [&str; 5] = [
    "text/plain;charset=utf-8",
    "text/plain",
    "UTF8_STRING",
    "STRING",
    "TEXT",
];
const IMAGE_MIMES: [&str; 1] = ["image/png"];
/// Files paste as files in Nautilus (GNOME's own format first) and as their
/// URIs in text fields.
const FILE_MIMES: [&str; 7] = [
    "x-special/gnome-copied-files",
    "text/uri-list",
    "text/plain;charset=utf-8",
    "text/plain",
    "UTF8_STRING",
    "STRING",
    "TEXT",
];

/// What we currently offer on the clipboard (served on demand).
#[derive(Debug, Clone)]
pub enum Offer {
    Text(String),
    Png(Vec<u8>),
    /// File URIs, as captured from `text/uri-list`.
    Files(Vec<String>),
}

impl Offer {
    fn mimes(&self) -> &'static [&'static str] {
        match self {
            Offer::Text(_) => &TEXT_MIMES,
            Offer::Png(_) => &IMAGE_MIMES,
            Offer::Files(_) => &FILE_MIMES,
        }
    }

    /// The bytes to send for a transfer request in `mime`, if we offered it.
    fn bytes_for(&self, mime: &str) -> Option<Vec<u8>> {
        match self {
            Offer::Text(t) if TEXT_MIMES.contains(&mime) => Some(t.as_bytes().to_vec()),
            Offer::Png(b) if mime == "image/png" => Some(b.clone()),
            Offer::Files(uris) => match mime {
                "x-special/gnome-copied-files" => {
                    Some(format!("copy\n{}", uris.join("\n")).into_bytes())
                }
                // RFC 2483: CRLF-terminated lines.
                "text/uri-list" => Some(
                    uris.iter()
                        .flat_map(|u| [u.as_str(), "\r\n"])
                        .collect::<String>()
                        .into_bytes(),
                ),
                m if TEXT_MIMES.contains(&m) => Some(uris.join("\n").into_bytes()),
                _ => None,
            },
            _ => None,
        }
    }
}

pub struct PortalSession {
    rd: RemoteDesktop,
    clip: Clipboard,
    session: Session<RemoteDesktop>,
    restore_token: Option<String>,
    offer: RefCell<Option<Offer>>,
}

impl PortalSession {
    /// Registers our identity, opens the session and starts it. Shows GNOME's
    /// permission dialog the first time; silent afterwards thanks to the token.
    pub async fn connect(restore_token: Option<String>) -> ashpd::Result<Self> {
        let app_id = ashpd::AppID::try_from(config::APP_ID).expect("valid app id");
        ashpd::register_host_app(app_id).await?;

        let rd = RemoteDesktop::new().await?;
        let session = rd.create_session(CreateSessionOptions::default()).await?;
        let clip = Clipboard::new().await?;
        clip.request(&session, RequestClipboardOptions::default())
            .await?;
        rd.select_devices(
            &session,
            SelectDevicesOptions::default()
                .set_devices(BitFlags::from(DeviceType::Keyboard))
                .set_persist_mode(PersistMode::ExplicitlyRevoked)
                .set_restore_token(restore_token.as_deref()),
        )
        .await?
        .response()?;
        let selected = rd
            .start(&session, None, StartOptions::default())
            .await?
            .response()?;
        tracing::info!(
            clipboard = selected.is_clipboard_enabled(),
            devices = ?selected.devices(),
            "portal session started"
        );
        Ok(Self {
            rd,
            clip,
            session,
            restore_token: selected.restore_token().map(str::to_owned),
            offer: RefCell::new(None),
        })
    }

    pub fn restore_token(&self) -> Option<&str> {
        self.restore_token.as_deref()
    }

    /// Runs until the session closes. Every clipboard change from another app
    /// is read and sent on `tx`.
    pub async fn monitor(&self, tx: async_channel::Sender<ClipEvent>) -> ashpd::Result<()> {
        let stream = self
            .clip
            .receive_selection_owner_changed::<RemoteDesktop>()
            .await?;
        futures_util::pin_mut!(stream);
        while let Some((_, ev)) = stream.next().await {
            if ev.session_is_owner().unwrap_or(false) {
                continue;
            }
            let mimes = ev.mime_types();
            if mimes.iter().any(|m| m == PASSWORD_HINT) {
                tracing::debug!("skipping password-manager clipboard content");
                continue;
            }
            let Some(mime) = PREFERRED_MIMES
                .iter()
                .copied()
                .find(|p| mimes.iter().any(|m| m == p))
            else {
                continue;
            };
            match self.read(mime).await {
                Ok(Some(event)) => {
                    if tx.send(event).await.is_err() {
                        break;
                    }
                }
                Ok(None) => {}
                Err(e) => tracing::warn!("clipboard read failed: {e}"),
            }
        }
        Ok(())
    }

    async fn read(&self, mime: &str) -> ashpd::Result<Option<ClipEvent>> {
        let fd: OwnedFd = self.clip.selection_read(&self.session, mime).await?.into();
        // The portal hands us a non-blocking pipe; read it asynchronously.
        let io_err = |e: std::io::Error| {
            ashpd::Error::from(ashpd::zbus::Error::InputOutput(std::sync::Arc::new(e)))
        };
        let mut reader = async_io::Async::new(std::fs::File::from(fd)).map_err(io_err)?;
        // Stop one byte past the limit instead of buffering an unbounded payload.
        let mut bytes = Vec::new();
        (&mut reader)
            .take(MAX_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(io_err)?;
        if bytes.is_empty() || bytes.len() > MAX_BYTES {
            return Ok(None);
        }
        Ok(Some(match mime {
            "image/png" => ClipEvent::Image {
                mime: mime.to_owned(),
                bytes,
            },
            "text/uri-list" => ClipEvent::Uris(
                String::from_utf8_lossy(&bytes)
                    .lines()
                    .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
                    .map(str::to_owned)
                    .collect(),
            ),
            _ => {
                let text = String::from_utf8_lossy(&bytes).into_owned();
                if text.trim().is_empty() {
                    return Ok(None);
                }
                ClipEvent::Text(text)
            }
        }))
    }

    /// Press Ctrl+V in whatever window is focused. Call only after our own
    /// window is hidden and the user's modifiers are released.
    pub async fn inject_paste(&self) -> ashpd::Result<()> {
        let key = |code, state| {
            self.rd.notify_keyboard_keycode(
                &self.session,
                code,
                state,
                NotifyKeyboardKeycodeOptions::default(),
            )
        };
        key(KEY_LEFTCTRL, KeyState::Pressed).await?;
        glib::timeout_future(KEY_GAP).await;
        key(KEY_V, KeyState::Pressed).await?;
        glib::timeout_future(KEY_GAP).await;
        key(KEY_V, KeyState::Released).await?;
        glib::timeout_future(KEY_GAP).await;
        key(KEY_LEFTCTRL, KeyState::Released).await?;
        Ok(())
    }

    /// Take ownership of the clipboard with `offer`. Works while our window is
    /// hidden, because the portal session (not a surface) is the owner.
    pub async fn offer(&self, offer: Offer) -> ashpd::Result<()> {
        let mimes = offer.mimes();
        self.offer.replace(Some(offer));
        self.clip
            .set_selection(
                &self.session,
                SetSelectionOptions::default().set_mime_types(mimes),
            )
            .await
    }

    /// Runs forever: answers every `SelectionTransfer` request with the bytes of
    /// the current offer. Must be running before `offer()` is used.
    pub async fn serve_transfers(&self) -> ashpd::Result<()> {
        let stream = self
            .clip
            .receive_selection_transfer::<RemoteDesktop>()
            .await?;
        futures_util::pin_mut!(stream);
        while let Some((_, mime, serial)) = stream.next().await {
            let bytes = self
                .offer
                .borrow()
                .as_ref()
                .and_then(|o| o.bytes_for(&mime));
            let ok = match (
                bytes,
                self.clip.selection_write(&self.session, serial).await,
            ) {
                (Some(bytes), Ok(fd)) => {
                    let fd: OwnedFd = fd.into();
                    match async_io::Async::new(std::fs::File::from(fd)) {
                        Ok(mut w) => w.write_all(&bytes).await.is_ok() && w.flush().await.is_ok(),
                        Err(_) => false,
                    }
                }
                (_, Err(e)) => {
                    tracing::warn!("SelectionWrite failed: {e}");
                    false
                }
                (None, Ok(_)) => false,
            };
            if let Err(e) = self
                .clip
                .selection_write_done(&self.session, serial, ok)
                .await
            {
                tracing::warn!("SelectionWriteDone failed: {e}");
            }
        }
        Ok(())
    }

    /// Resolves when GNOME closes the session (permission revoked, portal or
    /// shell restarted). Clipboard signals simply stop arriving then.
    pub async fn closed(&self) {
        match self.session.receive_closed().await {
            Ok(stream) => {
                futures_util::pin_mut!(stream);
                stream.next().await;
            }
            Err(e) => {
                tracing::warn!("cannot watch the portal session: {e}");
                std::future::pending::<()>().await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_are_offered_as_files_and_text() {
        let offer = Offer::Files(vec!["file:///a%20b".into(), "file:///c".into()]);
        assert_eq!(
            offer.bytes_for("x-special/gnome-copied-files").unwrap(),
            b"copy\nfile:///a%20b\nfile:///c"
        );
        assert_eq!(
            offer.bytes_for("text/uri-list").unwrap(),
            b"file:///a%20b\r\nfile:///c\r\n"
        );
        assert_eq!(
            offer.bytes_for("text/plain").unwrap(),
            b"file:///a%20b\nfile:///c"
        );
        assert!(offer.bytes_for("image/png").is_none());
        assert!(Offer::Text("x".into()).bytes_for("text/uri-list").is_none());
    }
}
