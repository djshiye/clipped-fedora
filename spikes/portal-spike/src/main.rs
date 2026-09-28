use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

use ashpd::{
    desktop::{
        clipboard::{Clipboard, RequestClipboardOptions},
        global_shortcuts::{BindShortcutsOptions, GlobalShortcuts, NewShortcut},
        remote_desktop::{
            DeviceType, KeyState, NotifyKeyboardKeycodeOptions, NotifyKeyboardKeysymOptions, RemoteDesktop,
            SelectDevicesOptions, StartOptions,
        },
        CreateSessionOptions, PersistMode,
    },
};
use futures_util::StreamExt;

// Linux evdev keycodes (input-event-codes.h)
const KEY_LEFTCTRL: i32 = 29;
const KEY_V: i32 = 47;

fn token_path() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap()).join(".cache"));
    base.join("clipped-spike").join("restore-token")
}

fn log(tag: &str, msg: impl AsRef<str>) {
    let t = chrono_now();
    println!("[{t}] {tag:<9} {}", msg.as_ref());
}

fn chrono_now() -> String {
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap();
    let s = d.as_secs() % 86_400;
    format!("{:02}:{:02}:{:02}.{:03}", s / 3600, (s / 60) % 60, s % 60, d.subsec_millis())
}

fn pick_mime(mimes: &[String]) -> Option<&str> {
    const PREFERRED: [&str; 5] = [
        "image/png",
        "text/uri-list",
        "text/plain;charset=utf-8",
        "UTF8_STRING",
        "text/plain",
    ];
    PREFERRED
        .iter()
        .copied()
        .find(|p| mimes.iter().any(|m| m == p))
}

async fn run() -> ashpd::Result<()> {
    log("spike", "starting. Press Ctrl+C to stop.");
    // Declare our identity to the portal (non-sandboxed apps). Must precede every other portal call.
    let app_id = ashpd::AppID::try_from("io.github.dino.ClippedSpike").expect("valid app id");
    ashpd::register_host_app(app_id).await?;
    log("portal", "registered host app id io.github.dino.ClippedSpike");

    // ── 1. RemoteDesktop + Clipboard session ─────────────────────────────
    let saved_token = fs::read_to_string(token_path()).ok().map(|s| s.trim().to_string());
    log("portal", format!("saved restore token: {}", saved_token.as_deref().unwrap_or("<none>")));

    let rd = RemoteDesktop::new().await?;
    log("portal", format!("RemoteDesktop portal version {}", rd.version()));
    let session = rd.create_session(CreateSessionOptions::default()).await?;

    let clip = Clipboard::new().await?;
    log("portal", format!("Clipboard portal version {}", clip.version()));
    // Must be called before Start.
    clip.request(&session, RequestClipboardOptions::default()).await?;

    rd.select_devices(
        &session,
        SelectDevicesOptions::default()
            .set_devices(ashpd::enumflags2::BitFlags::from(DeviceType::Keyboard))
            .set_persist_mode(PersistMode::ExplicitlyRevoked)
            .set_restore_token(saved_token.as_deref()),
    )
    .await?
    .response()?;

    log("portal", "calling Start (a permission dialog may appear now)...");
    let t0 = Instant::now();
    let selected = rd.start(&session, None, StartOptions::default()).await?.response()?;
    log(
        "portal",
        format!(
            "started after {:?}: devices={:?} clipboard_enabled={} restore_token={}",
            t0.elapsed(),
            selected.devices(),
            selected.is_clipboard_enabled(),
            selected.restore_token().unwrap_or("<none>")
        ),
    );
    if let Some(tok) = selected.restore_token() {
        let p = token_path();
        fs::create_dir_all(p.parent().unwrap()).ok();
        fs::write(&p, tok).ok();
        log("portal", format!("restore token saved to {}", p.display()));
    }

    // ── 2. GlobalShortcuts session ───────────────────────────────────────
    let gs = GlobalShortcuts::new().await?;
    log("shortcut", format!("GlobalShortcuts portal version {}", gs.version()));
    let gs_session = gs.create_session(CreateSessionOptions::default()).await?;
    let bound = gs
        .bind_shortcuts(
            &gs_session,
            &[NewShortcut::new("toggle", "Show clipboard history").preferred_trigger("<Super><Shift>v")],
            None,
            BindShortcutsOptions::default(),
        )
        .await?
        .response()?;
    for s in bound.shortcuts() {
        log("shortcut", format!("bound '{}' -> {} ({})", s.id(), s.trigger_description(), s.description()));
    }

    // ── 3. Event loops ───────────────────────────────────────────────────
    let owner_changed = clip.receive_selection_owner_changed::<RemoteDesktop>().await?;
    futures_util::pin_mut!(owner_changed);
    let activated = gs.receive_activated().await?;
    futures_util::pin_mut!(activated);

    let clip_loop = async {
        log("clip", "watching. Copy text/images in other apps now.");
        while let Some((_s, ev)) = owner_changed.next().await {
            let t = Instant::now();
            let mimes = ev.mime_types();
            if ev.session_is_owner().unwrap_or(false) {
                log("clip", "owner-change from OUR session (ignored)");
                continue;
            }
            if mimes.iter().any(|m| m == "x-kde-passwordManagerHint") {
                log("clip", "password-manager hint present -> skipped");
                continue;
            }
            log("clip", format!("owner changed, {} mime types: {}", mimes.len(), mimes.join(", ")));
            let Some(mime) = pick_mime(mimes) else {
                log("clip", "no usable mime type");
                continue;
            };
            match clip.selection_read(&session, mime).await {
                Ok(fd) => {
                    let f = fs::File::from(std::os::fd::OwnedFd::from(fd));
                    let mut f = async_io::Async::new(f).expect("async fd");
                    let mut buf = Vec::new();
                    use futures_util::AsyncReadExt as _;
                    match f.read_to_end(&mut buf).await {
                        Ok(n) => {
                            let preview = if mime.starts_with("text") || mime == "UTF8_STRING" {
                                let s = String::from_utf8_lossy(&buf);
                                let one: String = s.chars().take(80).collect();
                                one.replace('\n', "⏎")
                            } else {
                                format!("<{n} bytes binary>")
                            };
                            log("clip", format!("read {mime} ({n} bytes) in {:?}: {preview}", t.elapsed()));
                        }
                        Err(e) => log("clip", format!("read error: {e}")),
                    }
                }
                Err(e) => log("clip", format!("SelectionRead failed: {e}")),
            }
        }
        log("clip", "owner-changed stream ended");
    };

    let deactivated = gs.receive_deactivated().await?;
    futures_util::pin_mut!(deactivated);

    let shortcut_loop = async {
        log("shortcut", "waiting. Focus GNOME Text Editor and press the shortcut ONCE.");
        while let Some(ev) = activated.next().await {
            log("shortcut", format!("ACTIVATED id={}", ev.shortcut_id()));
            let timeout = async_io::Timer::after(Duration::from_secs(3));
            futures_util::pin_mut!(timeout);
            match futures_util::future::select(deactivated.next(), timeout).await {
                futures_util::future::Either::Left(_) => log("shortcut", "DEACTIVATED (keys released)"),
                futures_util::future::Either::Right(_) => log("shortcut", "no Deactivated within 3 s"),
            }
            async_io::Timer::after(Duration::from_millis(300)).await;

            let gap = || async_io::Timer::after(Duration::from_millis(30));
            let kc = |code: i32, st: KeyState| rd.notify_keyboard_keycode(&session, code, st, NotifyKeyboardKeycodeOptions::default());
            let ks = |sym: i32, st: KeyState| rd.notify_keyboard_keysym(&session, sym, st, NotifyKeyboardKeysymOptions::default());

            let r: Result<(), ashpd::Error> = async {
                // Step A: plain letter via keysym ('a' = 0x61). Expect: "a" appears.
                ks(0x61, KeyState::Pressed).await?; gap().await;
                ks(0x61, KeyState::Released).await?; gap().await;
                log("paste", "A: typed 'a' via keysym");
                async_io::Timer::after(Duration::from_millis(400)).await;

                // Step B: plain letter via keycode (KEY_B = 48). Expect: "b" appears.
                kc(48, KeyState::Pressed).await?; gap().await;
                kc(48, KeyState::Released).await?; gap().await;
                log("paste", "B: typed 'b' via keycode");
                async_io::Timer::after(Duration::from_millis(400)).await;

                // Step C: Ctrl+V via keycodes with gaps. Expect: paste.
                kc(KEY_LEFTCTRL, KeyState::Pressed).await?; gap().await;
                kc(KEY_V, KeyState::Pressed).await?; gap().await;
                kc(KEY_V, KeyState::Released).await?; gap().await;
                kc(KEY_LEFTCTRL, KeyState::Released).await?; gap().await;
                log("paste", "C: Ctrl+V via keycodes (30 ms gaps)");
                async_io::Timer::after(Duration::from_millis(600)).await;

                // Step D: Ctrl+V via keysyms with gaps. Expect: paste.
                ks(0xffe3, KeyState::Pressed).await?; gap().await;
                ks(0x76, KeyState::Pressed).await?; gap().await;
                ks(0x76, KeyState::Released).await?; gap().await;
                ks(0xffe3, KeyState::Released).await?; gap().await;
                log("paste", "D: Ctrl+V via keysyms (30 ms gaps)");
                async_io::Timer::after(Duration::from_millis(600)).await;

                // Step E: Shift+Insert via keycodes (KEY_LEFTSHIFT=42, KEY_INSERT=110). Expect: paste (primary/clipboard).
                kc(42, KeyState::Pressed).await?; gap().await;
                kc(110, KeyState::Pressed).await?; gap().await;
                kc(110, KeyState::Released).await?; gap().await;
                kc(42, KeyState::Released).await?; gap().await;
                log("paste", "E: Shift+Insert via keycodes");
                Ok(())
            }.await;
            if let Err(e) = r { log("paste", format!("injection failed: {e}")); }
            log("paste", "sequence done. Expected editor text: a b <paste> <paste> <paste>");
        }
    };

    futures_util::join!(clip_loop, shortcut_loop);
    Ok(())
}

fn main() {
    if let Err(e) = async_io::block_on(run()) {
        eprintln!("spike failed: {e}");
        std::process::exit(1);
    }
}
