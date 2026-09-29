#!/bin/sh
# Invoked by Meson: builds the crate with Cargo and copies the binary to OUTPUT.
set -eu
MESON_BUILD_ROOT="$1"
MESON_SOURCE_ROOT="$2"
OUTPUT="$3"
BUILDTYPE="$4"
APP_BIN="$5"
OFFLINE="$6"
LOCALEDIR="${7:-}"
if [ -n "$LOCALEDIR" ]; then export CLIPPERINO_LOCALEDIR="$LOCALEDIR"; fi

# Meson hands us @OUTPUT@ relative to the build root; make it absolute before we cd.
case "$OUTPUT" in
    /*) ;;
    *) OUTPUT="$MESON_BUILD_ROOT/$OUTPUT" ;;
esac

export CARGO_TARGET_DIR="$MESON_BUILD_ROOT/target"
# Keep downloads inside the build tree unless the packager (e.g. %cargo_prep)
# already configured CARGO_HOME.
export CARGO_HOME="${CARGO_HOME:-$MESON_BUILD_ROOT/cargo-home}"

# cargo reads .cargo/config.toml relative to the working directory, which is
# where RPM builds put their offline registry configuration.
cd "$MESON_SOURCE_ROOT"

ARGS=""
if [ "$OFFLINE" = "offline" ]; then ARGS="$ARGS --offline"; fi

if [ "$BUILDTYPE" = "release" ] || [ "$BUILDTYPE" = "plain" ]; then
    cargo build --release $ARGS
    cp "$CARGO_TARGET_DIR/release/$APP_BIN" "$OUTPUT"
else
    cargo build $ARGS
    cp "$CARGO_TARGET_DIR/debug/$APP_BIN" "$OUTPUT"
fi
