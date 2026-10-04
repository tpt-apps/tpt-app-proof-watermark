# TPT Proof Watermarker

Watermark client proofs before final payment — text or logo, in your
browser, on-device. Free edition: single image, basic overlay. Pro desktop
edition ($15, one-time): batch folder watermarking, saved per-client
presets, EXIF stripping.

Part of the [TPT apps hub](https://tptsolutions.co.nz/tools) — spec at
`apps.md` §T2-13 in the `tpt-electrician-nz-2` repo, listing copy in
[`GUMROAD.md`](GUMROAD.md), build state in [`todo.md`](todo.md).

## Layout

| Path | What |
| --- | --- |
| `src/` | wasm32 DOM app (free tier; `pro.rs` only compiles under `--features pro`) |
| `engine/` | `tpt-proof-engine` — the watermark/EXIF adapter over `tpt-wasmforge`; plain functions, host-testable |
| `desktop/` | `tpt-proof-desktop` — the Pro webview shell (embeds `dist/` into the exe) |
| `build.ps1` / `build.sh` | the two-edition pipeline (see below) |

## Engine

`engine/` wraps `tpt-wasmforge`'s media crates (`tpt-media-core` +
`tpt-media-image`) rather than pulling in `image` directly: PNG/JPEG codecs
and bilinear resize come from wasmforge. The adapter adds what the proof
workflow needs and wasmforge doesn't have:

- **Watermark spec** (`spec.rs`): text/logo, 5 positions, opacity, size as a
  fraction of the short side, grid tiling with a gap fraction.
- **Render** (`render.rs`): text rasterized into a transparent overlay
  (same 6×8 bitmap font as wasmforge's text watermarker, plus a dedicated
  © glyph) composited source-over with clipping — so tiled and negative-
  origin placements work, and transparent PNG regions survive.
- **EXIF strip** (`exif.rs`): lossless metadata removal — JPEG APP1
  (Exif/XMP), APP13 (Photoshop/IPTC), COM segments dropped with every other
  segment copied verbatim; PNG `eXIf`/`tEXt`/`zTXt`/`iTXt` chunks dropped.
  Plus public inspector helpers (`jpeg_segments`, `png_chunks`) used by the
  validation tests.

Run the validation suite: `cargo test -p tpt-proof-engine` (9 tests covering
rendering across sizes/aspects, opacity bounds, logo scaling, tiling, PNG
alpha, and strip losslessness/idempotency via independent re-parsing).

## Hub contract

`wasm-bindgen --target web` glue exposes `mount(container)` — the hub's
`WasmAppRunner` dynamic-imports the glue, awaits its default export, then
calls `mount` with the page's container div. `#[wasm_bindgen(start)]` mounts
into `#tpt-appfront-root` only, which only the standalone `index.html`
provides, so `trunk serve` and the hub never double-mount.

## Building

```sh
./build.ps1          # free edition → hub web repo public/apps/proof-watermark/
                     #   (override the destination with -WebRepo or $env:TPT_WEB_REPO)
./build.ps1 -Pro     # pro edition → trunk pro bundle + tpt-proof-watermarker-pro.exe
                     #   staged with a README into release\ and zipped for Gumroad
```

The two editions are separate cargo builds on purpose: the free hub bundle
must never be compiled with `--features pro` (that would give away batch,
presets, and EXIF stripping on the site). The free bundle lands around
200 KB gzipped.

## Developing

```sh
trunk serve                # standalone dev page (mounts via #tpt-appfront-root)
cargo test -p tpt-proof-engine
cargo check --target wasm32-unknown-unknown --features pro   # pro wasm build
```

## Desktop shell (Pro)

`tpt-proof-desktop` opens the OS webview (WebView2) and serves the trunk
bundle. `dist/` is embedded into the exe at compile time, so the Gumroad zip
is a single exe — a customer double-click unpacks it from temp. The hosted
page gets a deliberately tiny file bridge over IPC (`dialog.open`,
`list_images`, `read_file_b64`, `write_file_b64`): reads restricted to
JPEG/PNG, writes confined to the most recently picked folder.
