# Clipperino (formerly Clipped) 1.0: Rust rebuild for Fedora

Plan written 2026-09-28 against commit e7f1ddf. Target: Fedora 44 Workstation, GNOME 50, Wayland. Delivery: RPM.

**Status (2026-09-28, end of day):** Phases 0–7 are implemented and verified on this machine; see `docs/SPIKES.md` for platform findings and `docs/PERF.md` for measurements. Not done: COPR publishing (needs the owner's API token), a Flatpak, and translations beyond the English source strings. The status icon was added on request (on by default, only registers when a tray host exists); its menu is the quick-access popup, listing the ten most recent clips with thumbnails, updated live, one click to paste. On Wayland an app cannot position a window next to the panel, so the tray host's own menu is the only anchored popup available, and it matches the macOS menu-bar pattern (Maccy, Paste).

---

## 0. Ground truth (verified on this machine, not assumed)

| Component | Version on Fedora 44 | Notes |
|---|---|---|
| GTK | 4.22.5 | GNOME 50 stack. GTK 4.24.0 (2026-09-11) exists upstream but is not in Fedora 44; it arrives with Fedora 45. |
| libadwaita | 1.9.4 | libadwaita 1.10.0 requires GTK 4.23+ and GLib 2.89+, so it cannot ship on Fedora 44. |
| GLib | 2.88.3 | |
| Rust toolchain | rust 1.98.1, cargo 1.98.1 (in repos, **not yet installed**) | |
| gtk4-rs | 0.11.5 on crates.io, 0.11.4 packaged as `rust-gtk4-devel` | Feature gates up to `v4_24`; use `v4_22`. Needs rust ≥ 1.92. |
| libadwaita-rs | 0.9.2 (crates.io and Fedora) | Feature gates up to `v1_10`; use `v1_9`. |
| ashpd (portals) | 0.13.13 (crates.io and Fedora) | Has `global_shortcuts`, `remote_desktop`, `clipboard`, `background` modules. |
| zbus | 5.18 (Fedora) | Transport for ashpd and the optional tray. |
| rusqlite | 0.40.2 crates.io, 0.38.0 Fedora | Pin to `0.38` so the RPM builds from Fedora crates. |
| ksni (StatusNotifierItem) | 0.3.6 | Optional tray. Not packaged in Fedora; vendor if used. |
| blueprint-compiler | 0.20.4 | UI files. |
| Portals present | GlobalShortcuts v1, RemoteDesktop v2, Clipboard, Background v2 | From `xdg-desktop-portal-gnome` 50.0. |
| Wayland protocols | `wl_data_device_manager` only; **no** `ext_data_control_v1` | Mutter does not allow background clipboard reading; the portal path is mandatory. |
| Displays | 2 × 2048×1152 at **120 Hz**, fractional scale **1.25** | Frame budget is 8.3 ms. Fractional scaling must stay crisp. |
| GPU / renderer | Radeon RX 9070, Mesa 26.2 Vulkan | GTK's Vulkan renderer has been the default since 4.16. |
| GNOME shortcuts | `<Super>v` and `<Super>m` are taken by the notification list | Default must not be Super+V. |
| User stylesheet | `~/.config/gtk-4.0/gtk.css` → Orchis-Dark theme (has parse errors at lines 230 and 243) | Overrides Adwaita in every GTK4 app. Test with and without it. |

"Latest" in this plan means: the newest versions the Fedora 44 RPM can depend on. Everything is written so that bumping to GTK 4.24 / libadwaita 1.10 on Fedora 45 is a one-line feature-gate change.

## 1. What is wrong with the current app (short form)

The C/GTK3 code compiles on Fedora but is built around X11 assumptions that fail on GNOME Wayland: clipboard monitoring only works via XWayland's bridge, the `XGrabKey` hotkey never fires while a native app is focused, `GtkStatusIcon` needs a shell extension, and window positioning, keep-above and skip-taskbar are ignored. Beyond platform issues: the "Max entries" setting is never saved or read, images are never persisted, history is written only at exit, previews are cut at byte boundaries and can produce invalid UTF-8, search is not Unicode-aware, every clipboard change destroys and rebuilds every row, each emoji button gets its own CSS provider, clipboard reads block the main loop, and Settings spawns `curl` against a dead placeholder URL. The visual layer hard-codes a blue palette, 11 to 13 px fonts and emoji as navigation icons, and offers a "Save Settings" button and an unconfirmed "Clear all history".

Nothing from `src/` is reused except the curated symbol list, which moves to a data file.

## 2. Architecture

### 2.1 Stack

| Layer | Choice | Why |
|---|---|---|
| Language | Rust 2024 edition | Memory safety for a long-running daemon parsing untrusted clipboard data; `String` makes the UTF-8 truncation bug class impossible. |
| UI | `gtk4` 0.11 (`v4_22`) + `libadwaita` 0.9 (`v1_9`), Blueprint `.blp` templates | Native GNOME look, system light/dark/accent, animations and accessibility for free. |
| Async | `glib::MainContext::spawn_local` on the GTK main loop; `ashpd` with its default `async-io` transport | No second runtime, no thread hopping for UI updates. |
| Portals | `ashpd` 0.13 | Typed async wrappers for GlobalShortcuts, RemoteDesktop, Clipboard, Background. |
| Storage | `rusqlite` 0.38 (bundled feature off, link system SQLite) | Write-through history, WAL mode, survives crashes. |
| Hashing | `blake3` | Content dedupe and own-set detection. |
| Settings | `gio::Settings` with a compiled schema | Bindable to widgets, visible in `dconf-editor`. |
| Logging | `tracing` + `tracing-subscriber` (journald-friendly) | |
| i18n | `gettext-rs` | |
| Build | Meson wrapping Cargo (GNOME Builder's Rust template layout) | Meson installs desktop/metainfo/schema/icons; Cargo builds the binary. |
| Packaging | RPM via `cargo-rpm-macros`, published on COPR | `dnf copr enable` + `dnf install clipperino`. |

### 2.2 Platform integration (all Wayland-correct, all through portals)

| Concern | Mechanism | Fallback |
|---|---|---|
| Clipboard monitoring | `ashpd::desktop::remote_desktop::RemoteDesktop` session with `ashpd::desktop::clipboard::Clipboard::request` → `select_devices(DeviceType::Keyboard, persist_mode = ExplicitlyRevoked, restore_token)` → `start` → stream `selection_owner_changed`, read via `selection_read` fd. Token stored in GSettings. | If permission denied: `AdwStatusPage` explaining why, with "Grant Access" button. No X11 fallback in 2.0. |
| Paste into previous app | Same session: `notify_keyboard_keycode(KEY_LEFTCTRL, Pressed)`, `KEY_V`, releases. | Setting `paste-on-select` (default decided by spike 4). Off = copy and close with toast. |
| Global shortcut | `ashpd::desktop::global_shortcuts::GlobalShortcuts`: `create_session` → `bind_shortcuts([NewShortcut::new("toggle", "Show clipboard history").preferred_trigger("<Super><Shift>v")])` → stream `receive_activated`. GNOME lists it under Settings › Keyboard. | Document `clipperino --toggle` for a manual custom shortcut. |
| Autostart | `ashpd::desktop::background::Background::request().auto_start(true).reason(...)`; app appears in Settings › Apps › Background. | RPM installs nothing in `/etc/xdg/autostart`; the app asks once on first run. |
| Single instance, CLI | `adw::Application` with D-Bus activation (`io.github.<owner>.Clipperino.service`), actions `--toggle`, `--quit`. | – |
| Tray | Optional `ksni` StatusNotifierItem, registered only when `org.kde.StatusNotifierWatcher` is on the bus. Off by default. | – |
| Window | `adw::ApplicationWindow`, 380×560 default, resizable, size remembered, placed by Mutter. | – |
| Emoji data | GTK's own emoji database (`/org/gtk/libgtk/emoji/en.data` GVariant, localized) | Curated symbols in `data/symbols.json`. |

### 2.3 Spikes (throwaway, half a day each, before any real code)

1. RemoteDesktop + Clipboard portal: `selection_owner_changed` fires for copies from Firefox, Terminal, Files, GNOME Screenshot, a Flatpak app, an XWayland app? Does the `restore_token` survive logout? Exact dialog wording.
2. GlobalShortcuts: after `Activated`, does `window.present()` take focus over a full-screen app? Does GNOME hand us an xdg-activation token?
3. `notify_keyboard_keycode` Ctrl+V lands in the previously focused app after the window hides? Minimum delay.
4. Decision: is auto-paste worth the "remote desktop" permission wording? This sets the `paste-on-select` default.
5. Frame-time check: an `adw::ApplicationWindow` with a 1,000-row `gtk::ListView` scrolling at 120 Hz on this GPU with the Orchis stylesheet active. Confirms the budget before designing rows.

## 3. Design system: Apple HIG applied to a GNOME app

Principles were taken from developer.apple.com (Designing for macOS, Layout, Typography, Color, Dark Mode, Materials, Motion, Feedback, Searching, Settings, Menus, Icons, Accessibility). Apple's own rule is to feel "at home" on each platform, so the principles are applied with libadwaita components rather than by imitating macOS chrome.

### 3.1 Rules

| Apple HIG principle | Rule for Clipperino |
|---|---|
| Respect systemwide appearance; never add an app-specific light/dark switch | `adw::StyleManager` defaults. No custom palette. Semantic CSS only (`@window_bg_color`, `@card_bg_color`, `@accent_bg_color`, `.dim-label`). |
| Contrast ≥ 4.5:1, 7:1 for small text | Adwaita label colours only. Verified in high-contrast mode. |
| System fonts and text styles; no thin weights; support text-size changes | Zero hard-coded `font-size`. Style classes `.title-4`, `.heading`, `.body`, `.caption`, `.numeric`. Tested with Large Text. |
| Hierarchy: important content top-leading; group with negative space | Search entry at the top of every page. `.boxed-list` rows with 12 px margins. Preferences in `AdwPreferencesGroup`s. |
| Progressive disclosure | Row = kind icon, one-line preview, relative time. Full text in a detail dialog, never inline. |
| Differentiate controls from content; let people move and resize windows | `AdwToolbarView` + `AdwHeaderBar`, CSD, resizable. No custom drawing, no keep-above. |
| macOS: nothing critical at the bottom of a window | `AdwViewSwitcher` in the header bar; `AdwViewSwitcherBar` at the bottom only under a narrow `AdwBreakpoint`. |
| Search: primary position, descriptive placeholder, one location, live results | `gtk::SearchEntry` per page, placeholders "Search history" / "Search emoji" / "Search symbols". Focused on show. `key-capture-widget` set so typing anywhere searches. |
| Settings: good defaults, few of them, apply immediately, `Ctrl+,`, no Save button | `AdwPreferencesDialog`, every control bound with `settings.bind()`. Groups: Shortcut, History, Startup, Appearance. |
| Feedback: confirm significant actions, warn on unexpected irreversible loss, prefer undo | "Copied" toast. Clear history → `AdwAlertDialog` with destructive "Clear". Delete one item → toast with Undo. Permission denied → status page with action. |
| Motion: brief, purposeful, cancellable; respect reduce-motion | See 3.2. |
| Menus: verb labels, title case, grouped, icons only with purpose | Row menu: Copy, Pin/Unpin, separator, Delete. App menu: Preferences, Keyboard Shortcuts, About Clipperino, Quit. |
| Icons: simple, consistent weight, vector, system symbols | Adwaita symbolic icons: `edit-paste-symbolic`, `face-smile-symbolic`, `font-x-generic-symbolic`, `emblem-system-symbolic`. Accessible labels on all. |
| App icon: simple, centred, recognisable, dark variant | Cat redrawn as SVG on the GNOME 128 px grid plus `-symbolic`. |
| Accessibility: keyboard-only use, target sizes, labels | Full keyboard model (3.4). Minimum 32 px targets. `gtk::Accessible` names on every icon button. Tested with Orca. |
| Privacy before showing history | Skip password-manager copies (`x-kde-passwordManagerHint`). History file `0600`. Clear reachable from app menu. |
| Large displays: comfortable density | Two-column layout (list + preview pane) above 700 px width via `AdwBreakpoint`. |

### 3.2 Motion specification

HIG: "brief and precise", "avoid adding motion to UI interactions that occur frequently", "let people cancel motion", "make motion optional".

| Moment | Treatment | Duration / easing | Reduced motion |
|---|---|---|---|
| Window show/hide | Mutter's own map/unmap animation. No custom animation. | compositor | compositor |
| Page switch | `AdwViewStack` crossfade | 150 ms, Adwaita default | libadwaita 1.9 already switches to crossfade / none |
| New item at top of list | No movement (frequent interaction). Optional 250 ms opacity fade on the new row via `adw::TimedAnimation` with `Easing::EaseOutCubic`. | 250 ms | skipped when `gtk-enable-animations` is false |
| Hover / press | Adwaita CSS transitions (100 ms) | stock | stock |
| Delete / Undo | Row removed immediately; `AdwToast` with 5 s timeout | stock | stock |
| Toasts, dialogs | `AdwToastOverlay`, `AdwAlertDialog`, `AdwPreferencesDialog` stock transitions | stock | crossfade (1.9) |
| Search filtering | Instant, no animation | 0 | 0 |
| Scrolling | Kinetic, GTK default; `gtk::ListView` recycling keeps it at 120 fps | – | GTK 4.22 honours reduced motion in `GtkAdjustment` |

Rule: no custom `adw::SpringAnimation` or `adw::TimedAnimation` may run on a widget larger than one row, and none may block input.

### 3.2b Visual language (implemented)

The Apple feel comes from surfaces, not from imitating macOS chrome: history rows are cards (14 px radius, 1 px hairline ring, soft shadow, hover lift), the selected card is tinted with the accent colour and ringed rather than filled, the kind icon sits in a tinted rounded square, the search field is a pill with a soft focus ring, emoji and symbol cells are 44 px with rounded hover states, category chips sit in a pill group, secondary controls (⋮) appear only on hover, focus or selection, and every state change transitions in 120–150 ms ease-out. All colours are Adwaita tokens through `var(--…)` and `color-mix()`, so light, dark and accent follow the system. The stylesheet loads one notch above the user stylesheet priority because third-party GTK themes (e.g. Orchis) otherwise erase the card rules; it only targets Clipperino's own widgets.

### 3.3 Typography, colour, layout

- Fonts: system UI font (Adwaita Sans on Fedora 44) via style classes only; monospace (`.monospace`) for text previews that look like code (heuristic: contains `{`, `;`, or four leading spaces).
- Colour: accent from the system (`AdwStyleManager::accent_color`); destructive actions use `.destructive-action`; pinned marker uses `.accent` on a symbolic icon so state is also conveyed by the icon shape, not colour alone.
- Layout grid: 6 px base unit. Row height 48 px minimum (52 px with two-line previews), 12 px horizontal margins, 6 px between rows, 40 px glyph cells with 4 px gaps.
- Fractional scale 1.25: use only symbolic SVG icons and vector paths; never scale bitmaps except image thumbnails, which are generated at 2× logical size (192 px for a 96 px slot).

### 3.4 Keyboard model

| Key | Action |
|---|---|
| `Super+Shift+V` (portal; user can change) | Toggle Clipperino |
| `Escape` | Clear search if non-empty, else close |
| `↑ ↓`, `Enter` | Move selection, paste (or copy and close) |
| `Ctrl+1` … `Ctrl+9` | Paste the nth visible item |
| `Delete` | Remove selected item (undo toast) |
| `Ctrl+P` | Pin / unpin |
| `Ctrl+F` | Focus search |
| `Ctrl+,` / `Ctrl+?` / `Ctrl+Q` | Preferences / Shortcuts / Quit |

## 4. Performance engineering

Budgets (measured, not hoped for):

| Metric | Budget | How it is met |
|---|---|---|
| Shortcut → window visible | < 100 ms | App runs as a D-Bus service; window built once at startup and hidden, never rebuilt. |
| Frame time while scrolling | < 8.3 ms (120 Hz) | `gtk::ListView` / `gtk::GridView` with `SignalListItemFactory`: widgets are recycled, only visible rows exist. No per-frame allocation in `bind`. |
| Idle CPU | 0% | No polling anywhere; portal signals wake the process. |
| Resident memory, 100 text items | < 50 MB | Only thumbnails (≤ 192 px) held in RAM; full `gdk::Texture` loaded on demand for the detail view and dropped after. |
| Clipboard event → row visible | < 30 ms for text | Read fd asynchronously; hash on a `gio::spawn_blocking` thread for payloads > 64 KiB; `ListStore::insert(0)` on the main thread. |
| Image decode | never on the main thread | `gdk::Texture::from_bytes` inside `spawn_blocking` (`GdkTexture` is thread-safe), thumbnail via `gdk::Texture` downscale on the same thread. Cap decode at 20 MiB. |
| SQLite write | never on the main thread, debounced 250 ms | Single writer thread with a channel; WAL mode; `synchronous=NORMAL`. |
| Startup | < 300 ms to service ready | Lazy emoji model (parsed on first switch to the Emoji page). |

Engineering rules:
- One `gtk::CssProvider`, loaded from GResource at startup, `STYLE_PROVIDER_PRIORITY_APPLICATION`. No per-widget providers.
- All list content behind `GListModel`: `HistoryStore` (`gio::ListStore`) → `gtk::FilterListModel` (`gtk::StringFilter`, `ignore_case`, `Substring`, `watch_items` off) → `gtk::NoSelection` / `gtk::SingleSelection`.
- Search debounce 50 ms; filtering on the main thread is fine at ≤ 1,000 items because `StringFilter` short-circuits on the preview string.
- Text previews are precomputed once when the item is created (`chars().take(120)` with grapheme awareness via `unicode-segmentation`) and stored in the DB; rows never touch the full text.
- Never call `queue_resize` in a loop; never use `gtk::Box` with hundreds of children.
- Vulkan renderer default; do not override `GSK_RENDERER`. Verify with `GSK_DEBUG=profile` in the inspector (GTK 4.22) and `GDK_DEBUG=frames` for dropped-frame logs.
- Profile with `sysprof-cli --gtk` and the Inspector's recorder; regressions in the frame-time budget block a merge.
- Release profile: `opt-level = 3`, `lto = "thin"`, `codegen-units = 1`, `panic = "abort"`, `strip = true`.

## 5. Phases and steps

### Phase 0: Spikes (section 2.3)

1. `sudo dnf install rust cargo gtk4-devel libadwaita-devel meson ninja-build blueprint-compiler gettext desktop-file-utils appstream rpm-build rpmdevtools cargo-rpm-macros sqlite-devel`
2. Five throwaway binaries under `spikes/`, each ≤ 150 lines, results recorded in `docs/SPIKES.md`.

### Phase 1: Scaffolding

3. App ID `io.github.<owner>.Clipperino`; binary `clipperino`. Tag the old tree `v1.0.0-legacy` and delete `src/`, `Makefile*`, `debian/`, `build-deb.sh`, `install.sh`, `uninstall.sh`, `run.sh`, `clipped.exe`, `releases/`.
4. Tree:
   ```
   Cargo.toml  Cargo.lock  meson.build  meson_options.txt  build-aux/
   data/        <id>.desktop.in  <id>.metainfo.xml.in  <id>.gschema.xml  <id>.service.in
                icons/hicolor/{scalable,symbolic}/apps/  symbols.json  style.css  clipperino.gresource.xml
   data/ui/     window.blp  history-page.blp  history-row.blp  glyph-page.blp  glyph-cell.blp
                preferences.blp  shortcuts.blp  detail-dialog.blp
   src/         main.rs  application.rs  config.rs  window.rs
                model/{clip_item.rs, history_store.rs, glyph.rs}
                storage/{db.rs, migrate.rs, images.rs}
                platform/{clipboard_monitor.rs, shortcut.rs, paste.rs, background.rs, tray.rs}
                ui/{history_page.rs, history_row.rs, glyph_page.rs, glyph_cell.rs, preferences.rs, detail.rs}
   po/          tests/   docs/
   ```
5. `Cargo.toml` dependencies pinned to Fedora-packaged versions: `gtk4 = { version = "0.11", features = ["v4_22"] }`, `libadwaita = { version = "0.9", features = ["v1_9"] }`, `ashpd = "0.13"`, `rusqlite = "0.38"`, `blake3 = "1"`, `serde`/`serde_json = "1"`, `tracing = "0.1"`, `gettext-rs = "0.7"`, `unicode-segmentation = "1"`. Optional feature `tray = ["ksni"]`.
6. `meson.build`: `project('clipperino', 'rust', version: '2.0.0')`, `gnome.compile_resources`, `gnome.compile_schemas`, a `custom_target` running `cargo build --release --offline` with `CARGO_HOME` pointed at the build dir, `gnome.post_install(glib_compile_schemas: true, gtk_update_icon_cache: true, update_desktop_database: true)`. Blueprint compiled via `blueprint-compiler batch-compile`.
7. `config.rs.in` generated by Meson with `APP_ID`, `VERSION`, `LOCALEDIR`, `PKGDATADIR`.
8. Icons: full-colour SVG on the 128 px grid, symbolic SVG, both in `hicolor`. GResource embeds UI, CSS, `symbols.json`.
9. Tooling: `.editorconfig`, `rustfmt.toml`, `clippy` in CI with `-D warnings`, `cargo deny` for licences, `.gitignore` with `target/` and `builddir/`.

### Phase 2: Platform layer

10. `application.rs`: `adw::Application` subclass, flags `HANDLES_COMMAND_LINE`, actions `toggle`, `quit`, `preferences`, `shortcuts`, `about`. `startup` loads CSS and builds the window hidden. `activate` toggles. Service file so `gapplication launch` and the shortcut portal can wake it.
11. `platform/clipboard_monitor.rs`: one `RemoteDesktop` session shared with `paste.rs`; restore token in GSettings key `restore-token`. Emits `ClipboardEvent { mimes, read: impl Fn(mime) -> Future<Bytes> }` on a `glib` channel. Skips events where `session_is_owner` or the MIME list contains `x-kde-passwordManagerHint`. Priority order: `image/png` → `text/uri-list` → `text/plain;charset=utf-8` → `text/plain`.
12. Own-set detection: `set_selection` records the blake3 of what was written; the next event with the same hash is ignored. Replaces the boolean flag.
13. `platform/shortcut.rs`: bind on startup, rebind from Preferences ("Change Shortcut…" opens GNOME's dialog). Display the current trigger in an `AdwActionRow` subtitle using `AdwShortcutLabel`.
14. `platform/paste.rs`: `set_selection`, hide window, `glib::timeout_future(…)` from spike 3, `notify_keyboard_keycode`. Guarded by `paste-on-select`.
15. `platform/background.rs`: first run asks via the Background portal; the "Run in Background" `AdwSwitchRow` re-requests with the new value.
16. `platform/tray.rs` (feature `tray`): `ksni` item with Show / Preferences / Quit; only registered if the watcher name exists. "Show Tray Icon" switch insensitive with subtitle "No system tray available" otherwise.

### Phase 3: Data layer

17. `ClipItem` GObject (`glib::Properties` derive): `id: u64`, `kind: Kind {Text, Image, Files}`, `text: Option<String>`, `preview: String`, `thumbnail: Option<gdk::Texture>`, `image_path: Option<PathBuf>`, `timestamp: i64`, `pinned: bool`, `hash: [u8; 32]`.
18. `HistoryStore`: `gio::ListStore<ClipItem>` + `HashMap<hash, ClipItem>`; `add` moves an existing hash to index 0; `trim` respects `max-history` and never drops pinned items; `remove` returns the item for Undo.
19. `storage/db.rs`: `$XDG_DATA_HOME/clipperino/history.db`, table `items(id, kind, hash UNIQUE, text, preview, image_path, created, pinned)`, `PRAGMA journal_mode=WAL; synchronous=NORMAL`. Writer thread + `std::sync::mpsc`. Images under `images/<hash>.png`, thumbnails under `thumbs/<hash>.png`.
20. `storage/migrate.rs`: import `~/.local/share/clipman/history.txt` once, then rename it `.imported`.
21. Limits: `max-history` 10–1000 (default 100), skip text > 1 MiB, images > 20 MiB.
22. Glyph models: `Glyph { glyph, name, keywords, group }`; emoji from GTK's GVariant with the current locale, symbols from `symbols.json`; `recent-emoji` GSettings list capped at 24.

### Phase 4: UI

23. Blueprint files per §5 step 4. Window: `AdwToolbarView` → `AdwHeaderBar` (title widget `AdwViewSwitcher`, end: menu button) → `AdwViewStack` (History, Emoji, Symbols) → `AdwViewSwitcherBar` shown by `AdwBreakpoint` `max-width: 450sp`.
24. History page: `gtk::SearchEntry` + `gtk::ScrolledWindow` + `gtk::ListView` (`SingleSelection`, `single-click-activate`). `HistoryRow` composite widget with `bind()`/`unbind()`; image rows use `gtk::Picture` with the thumbnail texture and `content-fit: cover`.
25. Empty states: `AdwStatusPage` "No Clipboard History" / "No Results" / "Clipboard Access Needed" (with button).
26. Detail dialog (`AdwDialog`): full text in a `gtk::TextView` (read-only, `.monospace` when code-like) or full image in `gtk::Picture`; actions Copy, Pin, Delete.
27. Glyph pages: `gtk::GridView` (min 6, max 12 columns) of `GlyphCell`s; `AdwToggleGroup` category chips; "Recent" first; tooltip = name.
28. Preferences dialog (`AdwPreferencesDialog`): Shortcut (`AdwActionRow` + "Change…"), History (`AdwSpinRow` max entries, `AdwSwitchRow` paste on select, `AdwButtonRow` "Clear History…" destructive), Startup (`AdwSwitchRow` run in background), Appearance (`AdwSwitchRow` tray icon). All bound with `settings.bind()`.
29. `AdwShortcutsDialog` from `shortcuts.blp`; `adw::AboutDialog` with real author, MIT licence, website, issue URL.
30. Accessibility pass: `accessible-role`, `label`/`description` on every icon-only button; `gtk::ListView` rows expose the preview as their name; test with Orca and with Large Text at 200%.
31. All strings through `gettext!`; `po/POTFILES` generated by Meson.

### Phase 5: Performance pass (against §4 budgets)

32. Spike 5 harness becomes `tests/bench_list.rs`: 1,000 synthetic rows, scroll programmatically, assert no frame > 8.3 ms with `GDK_DEBUG=frames` output parsed.
33. `cargo flamegraph` on: startup, 100 rapid clipboard events, search typing.
34. `valgrind --tool=massif` for RSS with 100 text + 20 image items.
35. Verify fractional-scale crispness at 1.25 (no bitmap icons, thumbnails at 2×).
36. Verify the Orchis stylesheet does not break layout; if it does, that is the user's theme override, documented, not worked around.

### Phase 6: RPM packaging and distribution

37. `build-aux/clipperino.spec`: `BuildRequires: cargo-rpm-macros >= 24, meson, rust-packaging, pkgconfig(gtk4) >= 4.22, pkgconfig(libadwaita-1) >= 1.9, pkgconfig(sqlite3), desktop-file-utils, libappstream-glib, blueprint-compiler, gettext`; `%generate_buildrequires` with `%cargo_generate_buildrequires`; `%prep` → `%cargo_prep`; `%build` → `%meson` + `%meson_build`; `%install` → `%meson_install`; `%check` → `%cargo_test`, `desktop-file-validate`, `appstream-util validate-relax`. Files: `%{_bindir}/clipperino`, desktop, metainfo, schema, icons, D-Bus service, locale.
38. Crates not in Fedora (only `ksni` if the tray feature is on, `unicode-segmentation` is packaged): build the RPM with the tray feature off, or ship a vendored tarball and `%cargo_prep -v vendor`. Default: tray off in the RPM.
39. Local build: `rpmdev-setuptree`, `meson dist`, `rpmbuild -ba`, `rpmlint`. Install: `sudo dnf install ~/rpmbuild/RPMS/x86_64/clipperino-2.0.0-1.fc44.x86_64.rpm`.
40. COPR: `copr-cli create clipperino --chroot fedora-44-x86_64 --chroot fedora-45-x86_64 --chroot fedora-rawhide-x86_64`, `copr-cli build clipperino clipperino-2.0.0-1.fc44.src.rpm`. Users: `sudo dnf copr enable <user>/clipperino && sudo dnf install clipperino`. Also attach the RPM to each GitHub Release for one-file installs.
41. AppImage is intentionally not provided: GTK4 and libadwaita would have to be bundled wholesale with no maintained tooling, D-Bus activation and GSettings schemas do not register from a loose file, and a background service should not live in a movable file. If a distro-independent single file is wanted later, a `.flatpak` bundle needs no extra code.

### Phase 7: Testing and CI

42. Unit tests (`cargo test`): `HistoryStore` dedupe/trim/pin, grapheme-safe preview, DB round-trip with images, migration, own-set hash logic.
43. Portal tests: a `zbus` mock implementing `org.freedesktop.portal.Clipboard` under `dbus-run-session`, asserting the monitor reads and stores.
44. UI smoke test in CI with `xvfb-run` and `GTK_A11Y=test` constructing the window and switching pages.
45. Manual QA checklist on Fedora 44: copies from Firefox, Terminal, Files, GNOME Screenshot, a Flatpak, an XWayland app; shortcut fires from each; paste lands in each; permission persists after logout; Background toggle reflects in Settings › Apps; dark mode, high contrast, Large Text, reduce-motion, RTL, keyboard-only, Orca; with and without the Orchis stylesheet; 120 Hz scroll smoothness by eye and by `GDK_DEBUG=frames`.
46. GitHub Actions: `fedora:44` container job (`dnf install` deps, `cargo clippy -D warnings`, `cargo test`, `meson setup -Dwerror=true`, `rpmbuild`, `rpmlint`, `appstreamcli validate --strict`, `desktop-file-validate`); a second job on `fedora:rawhide` to catch GTK 4.24 / libadwaita 1.10 changes early.

### Phase 8: Release

47. `README.md` rewritten for Fedora: COPR install, default shortcut, permission explanation with a screenshot, building from source.
48. `CHANGELOG.md`, tag `v2.0.0`, GitHub Release with the RPM, AppStream screenshots.
49. `docs/SPIKES.md` and `docs/PERF.md` with measured numbers so Fedora 45 can be re-verified in an hour.
50. Follow-ups out of scope for 2.0: per-app exclusions once portals expose source app IDs, rich-text retention, Flatpak/Flathub, bump to `v4_24`/`v1_10` on Fedora 45.

## Appendix A: Cargo.toml sketch

```toml
[package]
name = "clipperino"
version = "2.0.0"
edition = "2024"
rust-version = "1.92"
license = "MIT"

[dependencies]
gtk = { package = "gtk4", version = "0.11", features = ["v4_22"] }
adw = { package = "libadwaita", version = "0.9", features = ["v1_9"] }
ashpd = "0.13"
rusqlite = "0.38"
blake3 = "1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tracing = "0.1"
tracing-subscriber = "0.3"
gettext-rs = { version = "0.7", features = ["gettext-system"] }
unicode-segmentation = "1"
ksni = { version = "0.3", optional = true }

[features]
default = []
tray = ["ksni"]

[profile.release]
opt-level = 3
lto = "thin"
codegen-units = 1
panic = "abort"
strip = true
```

## Appendix B: Clipboard monitor sketch (ashpd)

```rust
use ashpd::desktop::{clipboard::Clipboard, remote_desktop::{DeviceType, RemoteDesktop}, PersistMode};

pub async fn run(tx: async_channel::Sender<ClipEvent>, token: Option<String>) -> ashpd::Result<String> {
    let rd = RemoteDesktop::new().await?;
    let session = rd.create_session().await?;
    let clip = Clipboard::new().await?;
    clip.request(&session).await?;
    rd.select_devices(&session, DeviceType::Keyboard.into(), token.as_deref(), PersistMode::ExplicitlyRevoked).await?;
    let started = rd.start(&session, None).await?.response()?;
    let mut owner_changed = clip.receive_selection_owner_changed().await?;
    while let Some((_, opts)) = owner_changed.next().await {
        if opts.session_is_owner().unwrap_or(false) { continue; }
        let mimes = opts.mime_types().unwrap_or_default();
        if mimes.iter().any(|m| m == "x-kde-passwordManagerHint") { continue; }
        if let Some(mime) = pick_mime(&mimes) {
            let fd = clip.selection_read(&session, mime).await?;
            tx.send(ClipEvent::new(mime.to_string(), fd)).await.ok();
        }
    }
    Ok(started.restore_token().unwrap_or_default().to_string())
}
```

## Appendix C: Runtime dependencies of the RPM

`gtk4 >= 4.22`, `libadwaita >= 1.9`, `glib2 >= 2.88`, `sqlite-libs`, `xdg-desktop-portal-gnome` (weak dependency: `Recommends`). Nothing else. No X11, no curl.
