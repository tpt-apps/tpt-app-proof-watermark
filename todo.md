# TPT Proof Watermarker — Build Todo

> Hub listing: `tptsolutions.co.nz/tools/proof-watermark` (registry slug
> `proof-watermark`, category `utility`). Spec: `apps.md` §T2-13 in the
> tpt-electrician-nz-2 repo. Base wraps `tpt-wasmforge`. Free version: single
> image, basic overlay. Paid offline edition ($15 USD, Gumroad): batch folder
> watermarking, saved per-client presets, EXIF stripping. UI on
> `tpt-appfront`.
>
> Hub contract: `wasm-bindgen --target web` glue exposes `mount(container)`
> (called by `WasmAppRunner` after `default()` init). Standalone `trunk serve`
> mounts into the `#tpt-appfront-root` marker div instead — never
> `document.body` — so the two hosts never double-mount.

## Phase 0 — Scaffold & pipeline proof
- [x] Scaffold to hub contract: `#[wasm_bindgen] pub fn mount(el)` +
      `#[wasm_bindgen(start)]` no-op unless `#tpt-appfront-root` present
      (mirrored `tpt-app-image-toolkit` directly rather than running
      `tpt-appfront init` — the finished siblings are the canonical shape)
- [x] `index.html` carries the `#tpt-appfront-root` marker div
- [x] Build pipeline script mirroring fea-lite/cutlist-optimizer's
      (`--target web` glue → web repo `public/apps/proof-watermark/`)
- [x] Hello-world build loads through the standalone dev page (`trunk serve`)
- [x] git init + first commit

## Phase 1 — Core engine (wrap tpt-wasmforge)
- [x] Survey `tpt-wasmforge`: `tpt-media-image` has PNG/JPEG codecs, bilinear
      resize, and text/logo compositing; NO EXIF support and no tiling /
      alpha-capable overlay placement — those live in the adapter
- [x] Adapter crate `engine/`: image bytes + watermark spec (text/logo,
      position, opacity, tiling) in → watermarked image bytes out; image
      bytes in → EXIF/IPTC/XMP/comment-stripped bytes out (lossless segment
      rewrite for JPEG, chunk rewrite for PNG) — plain functions, no UI deps,
      host-unit-testable
- [x] Validation (`engine/tests/validation.rs`, 10 tests): watermark renders
      across portrait/landscape/square/panorama/tiny sizes, long text
      shrinks to fit the image width instead of clipping, opacity 0 is a
      no-op, logo scales by short side, tiling covers all four quadrants,
      PNG alpha preserved, dedicated © glyph; EXIF strip verified with an
      independent metadata inspector (segments before/after) +
      pixel-identical decode + idempotency

## Phase 2 — Free tier
- [x] Single-image upload + basic text/logo overlay watermark, download result
- [x] Free-tier upsell copy pointing at batch folder watermarking, saved
      per-client presets, EXIF stripping (nudges after 2 watermarks)
- [x] Browser-verified: mount, upload (drop), watermark renders correctly,
      download/reset appear, live sliders + conditional rows work

## Phase 3 — Paid desktop edition (cargo feature `pro`)
- [x] `pro` gates: batch folder watermarking, saved per-client presets
      (position/opacity/branding per client, localStorage-backed, small
      logos embedded), EXIF stripping (strip-only lossless batch)
- [x] Pro UI: folder picker, preset manager, batch progress, EXIF-strip entry
- [x] Desktop shell via `tpt-appfront-webview`; Windows exe build + smoke
      test (launches, "TPT Proof Watermarker Pro" window, responding)
      → `release/TPT-Proof-Watermarker-Pro.zip` staged for Gumroad
- [ ] Gumroad product from apps.md listing copy ($15), exe upload, URL into
      registry — manual step, listing copy staged in `GUMROAD.md`; after
      publishing paste the URL into the registry entry's `gumroadUrl` and
      flip `status` to `'live'`

## Phase 4 — Ship & measure
- [x] Registry entry (`utility` category, `wasm: { entry:
      '/apps/proof-watermark/proof-watermark.js' }`) committed in
      `tpt-apps-registry.ts`, status `coming-soon` until the Gumroad step
- [x] Free bundle built into web repo `public/apps/proof-watermark/` and
      committed (203 KB gzipped — well inside the ~1.5 MB budget); hub
      card/detail/sitemap verified in dev (`/tools`,
      `/tools/proof-watermark`, `/sitemap.xml` all serve the app)
- [ ] Runner click-through on the hub page observed in a browser (the
      WasmAppRunner `mount(container)` contract is verified — the glue's
      named export matches the shipped apps — but the in-browser hub
      verification was cut short by tooling; re-check on next dev run)
- [ ] Flip registry `status` to `'live'` after the Gumroad listing, verify
      prod after deploy
