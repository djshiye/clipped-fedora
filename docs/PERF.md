# Performance notes

Measured 2026-09-28 on Fedora 44, GNOME 50.5, Wayland, 2 × 2048×1152 @ 120 Hz, scale 1.25, Radeon RX 9070.

| Metric | Budget (plan) | Measured | Build |
|---|---|---|---|
| Scrolling, 1,000 text rows, 3 s animated scroll | < 8.3 ms per frame | 359 frames, median 8.33 ms, p95 8.35 ms, max 14.6 ms, 1 frame over 9 ms | debug (unoptimised) |
| Resident memory, idle, window hidden, portal session up | < 50 MB | 53 MB | release |
| Resident memory, 1,000 seeded rows + emoji model | – | 151 MB | debug |
| Seeding 1,000 items into the list model | – | 150 ms | debug |
| Portal session restore at startup | – | 2–15 ms, no dialog | either |
| Release binary | – | 4.2 MB (stripped, thin LTO) | release |

How to reproduce (debug builds only):

```bash
CLIPPERINO_DEBUG_SEED=1000 CLIPPERINO_DEBUG_SIZE=380x560 RUST_LOG=info cargo run
# then, from another terminal:
busctl --user call io.github.djshiye.Clipperino /io/github/djshiye/Clipperino/window/1 \
  org.gtk.Actions Activate "sava{sv}" debug-scroll 0 0
```

`debug-scroll` animates the history list to the bottom and logs frame-interval statistics from the frame clock. Seeded runs never attach storage, so they cannot touch real history.

Design decisions that keep it smooth: recycled `GtkListView`/`GtkGridView` rows, one CSS provider, image decoding and thumbnailing on worker threads with only 192 px thumbnails held in memory, PNGs kept on disk, SQLite writes on a dedicated thread, and no polling anywhere (portal signals wake the process).
