#!/usr/bin/env sh
# TPT Proof Watermarker - build pipeline (macOS/Linux).
#
# Free edition (default):  ./build.sh
# Pro desktop edition:     ./build.sh --pro
#   (adds trunk build --release --features pro + the native webview exe)
#
# Env:
#   TPT_WEB_REPO  path to the web repo; the bundle is copied to
#                 $TPT_WEB_REPO/public/apps/proof-watermark/ (glue named
#                 proof-watermark.js, wasm keeps its _bg.wasm name),
#                 otherwise dist/web in this repo.

set -eu
cd "$(dirname "$0")"

CRATE=tpt-app-proof-watermark
TARGET=wasm32-unknown-unknown
OUT_NAME=proof-watermark

PRO=0
for arg in "$@"; do
  case "$arg" in
    --pro) PRO=1 ;;
    *) echo "usage: build.sh [--pro]" >&2; exit 2 ;;
  esac
done

if [ -n "${TPT_WEB_REPO:-}" ]; then
  DEST="$TPT_WEB_REPO/public/apps/proof-watermark"
else
  DEST="dist/web"
fi

echo "== TPT Proof Watermarker build =="
echo " edition: $([ "$PRO" = 1 ] && echo 'pro (batch folders + per-client presets + EXIF strip)' || echo 'free (single image, basic text/logo overlay)')"
echo " dest:    $DEST"

if [ "$PRO" = 1 ]; then
  trunk build --release --features pro
  cargo build --release -p tpt-proof-desktop
  echo "   exe: target/release/tpt-proof-watermarker-pro (serves dist/)"
  echo "-- done"
  exit 0
fi

cargo build --release --target "$TARGET" -p "$CRATE"

WASM="target/$TARGET/release/tpt_app_proof_watermark.wasm"
[ -f "$WASM" ] || { echo "missing $WASM" >&2; exit 1; }

mkdir -p "$DEST"
wasm-bindgen --target web --out-name "$OUT_NAME" --out-dir "$DEST" "$WASM"

GLUE="$DEST/$OUT_NAME.js"
BG_WASM="$DEST/${OUT_NAME}_bg.wasm"
[ -f "$GLUE" ] || { echo "wasm-bindgen did not produce $GLUE" >&2; exit 1; }
[ -f "$BG_WASM" ] || { echo "wasm-bindgen did not produce $BG_WASM" >&2; exit 1; }

RAW=$(wc -c < "$BG_WASM" | tr -d ' ')
if command -v gzip >/dev/null 2>&1; then
  GZ=$(gzip -9 -c "$BG_WASM" | wc -c | tr -d ' ')
  echo "-- $(basename "$BG_WASM"): $RAW bytes raw, $GZ bytes gzipped"
  if [ "$GZ" -gt 1572864 ]; then
    echo "warning: payload exceeds the ~1.5 MB gzip budget for the image-codec build" >&2
  fi
fi

echo "-- done"
[ -n "${TPT_WEB_REPO:-}" ] || echo "   (set TPT_WEB_REPO to copy the bundle into the hub web repo)"
