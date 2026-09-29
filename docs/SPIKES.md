# Spike results

Recorded 2026-09-28 on Fedora 44, GNOME Shell 50.5, Wayland, xdg-desktop-portal 1.22.1, xdg-desktop-portal-gnome 50.0.
Code: `spikes/portal-spike/` (ashpd 0.13.13, async-io transport).

## Spike 1: RemoteDesktop + Clipboard portal for background clipboard monitoring — PASS

| Check | Result |
|---|---|
| `CreateSession` → `RequestClipboard` → `SelectDevices(keyboard, persist=ExplicitlyRevoked)` → `Start` | Works. `clipboard_enabled=true`, keyboard device granted. |
| Permission dialog | GNOME shows its **remote desktop / "allow remote interaction"** dialog. The user cancelled it on first sight. The app must show an explanation page before triggering it. |
| Restore token persistence | Second start with the saved token: **no dialog, 2.5 ms**. Token is stored per app ID, so the ID must be stable from the first run. |
| Text copy from a browser | Seen and read in < 1 ms (`text/plain;charset=utf-8`). |
| Unicode text (`wl-copy`) | Read correctly (35 bytes, `ünïcödé`). |
| Image (`wl-copy --type image/png`, 58 KB) | Read in < 1 ms once the pipe is read asynchronously. The fd is **non-blocking**; a blocking `read` fails with `EAGAIN`. |
| `x-kde-passwordManagerHint` | Present in the MIME list; skipping it works. |
| Event pattern | GNOME emits **three** `SelectionOwnerChanged` signals per copy: one with zero MIME types, then one or two with data. Dedupe by content hash. |
| Own writes | `session_is_owner` flag is provided; not yet exercised (spike never wrote). |

Image copy verified with `wl-copy --type image/png` (58 KB PNG read in < 1 ms). Not yet tested: copies from an XWayland app and from a Flatpak app; `text/uri-list` from Files.

## Spike 2: GlobalShortcuts portal — PASS (with a requirement)

| Check | Result |
|---|---|
| Bare binary from a terminal | `org.freedesktop.portal.Error.NotAllowed: An app id is required`. |
| Launched inside `app-gnome-<appid>-<n>.scope` | Works. This is how GNOME launches desktop apps; the RPM'd app gets it automatically. |
| Clean solution | `ashpd::register_host_app(app_id)` on the `org.freedesktop.host.portal.Registry` interface (present on this system), called before any other portal call. Use this in Phase 2 so launching from a terminal also works. |
| Binding `<Super><Shift>v` | Bound as "Press <Shift><Super>v". GNOME's binding dialog appeared once; re-bind on later starts took 96 ms with no dialog. |
| Activation | `Activated` signals arrive with a timestamp on every press. |

Focus after `Activated`: GNOME puts an `activation_token` in the signal's options. Passing it to `gtk::Window::set_startup_id()` before `present()` gives the window keyboard focus; without it Mutter refuses (focus-stealing prevention). **Verified in the app.**

## Spike 3: Ctrl+V injection via `NotifyKeyboardKeycode` — PASS (with two requirements)

| Attempt | Result in GNOME Text Editor |
|---|---|
| Four key events back-to-back (< 1 ms apart), 150 ms after `Activated` | Typed a capital "V" or nothing. The Ctrl modifier was lost and the user's physical Super+Shift were still held. |
| Same, but after waiting for `Deactivated` (key release, arrives ~160 ms after `Activated`) | Nothing. |
| **30 ms gap between each key event**, 300 ms after `Deactivated` | **Works** for all three: Ctrl+V via keycodes, Ctrl+V via keysyms, Shift+Insert via keycodes. Plain letters via keysym and keycode also arrive. |

Requirements for the real app:
1. Wait for the GlobalShortcuts `Deactivated` signal (or the window's own focus-out) before injecting, so the user's modifiers are up.
2. Space the four key events by ~30 ms (total paste latency about 120 ms). Do not fire them back-to-back.

## Spike 4: decision on auto-paste — ON by default

Monitoring already requires the remote-desktop permission, so auto-paste adds no extra prompt. It is the app's core interaction (select an item, it lands in the previous app). Default `paste-on-select = true`, with a preference to turn it off and get copy-and-close plus a "Copied" toast instead.

## Portal identity — REQUIREMENT

Portals refuse GlobalShortcuts, and store permissions, per app ID. A host (non-Flatpak) app gets an ID only if (a) it runs in a systemd scope named `app-…-<appid>-….scope` **or** calls `org.freedesktop.host.portal.Registry.Register` (`ashpd::register_host_app`) before any other portal call, **and** (b) a desktop file `<appid>.desktop` is installed. Deleting the desktop file silently downgraded the app to anonymous, which triggered a fresh permission dialog and broke shortcuts. The RPM installs the desktop file; the app registers itself at startup.

## Spike 5: list scrolling at 120 Hz

Deferred to the first Phase 4 build (needs the real window).

## libadwaita 1.9.4 bug: AdwToggle tooltip with invalid markup crashes

`adw_toggle_group_add()` → `update_button()` runs the toggle's tooltip through `pango_parse_markup()` and `g_free()`s the output without checking the return value. A tooltip that is not valid markup (a bare `&`, as in "Smileys & People") frees an uninitialised pointer: `double free or corruption (out)`. Reproduced in Python with `Adw.Toggle().set_tooltip("Smileys & People")`. Workaround in `src/ui/glyph_page.rs`: escape every tooltip with `glib::markup_escape_text()`. Worth reporting upstream against `src/adw-toggle-group.c`.

## Window size on this machine: the Déjà Window extension wins

The app restores its last window size from GSettings and Blueprint sets a default of 380×560, but on this machine the window always maps at the size the **Déjà Window** GNOME extension has stored for `io.github.djshiye.Clipperino`. A plain probe window under a different app ID honours its default size. This is the extension doing its job, not an app bug; debug builds accept `CLIPPERINO_DEBUG_SIZE=WxH` to force a size after mapping for layout checks.

## GtkImage baseline warnings from the Preferences dialog

Opening Preferences logs "GtkImage … reported baselines of minimum -2147483648 …" for four images (the spin row's +/− buttons and switch rows). It does not reproduce for the About, Details or Shortcuts dialogs. Cosmetic, comes from libadwaita/GTK internals (possibly interacting with the user's Orchis stylesheet); not from Clipperino's own widgets.

## User GTK themes override app CSS at APPLICATION priority

`~/.config/gtk-4.0/gtk.css` loads at `GTK_STYLE_PROVIDER_PRIORITY_USER` (800), above `APPLICATION` (600). The Orchis theme's generic `listview > row` rules therefore erased Clipperino's card design. Clipperino loads its stylesheet at `USER + 1`; the rules are scoped to Clipperino's own widgets and classes, so the theme still styles standard controls.
