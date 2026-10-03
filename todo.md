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
- [ ] `tpt-appfront init tpt-app-proof-watermark --target dom` (path deps on the tpt-appfront checkout)
- [ ] Restructure scaffold to hub contract: `#[wasm_bindgen] pub fn mount(el)` + `#[wasm_bindgen(start)]` no-op unless `#tpt-appfront-root` present
- [ ] `index.html` carries the `#tpt-appfront-root` marker div
- [ ] Build pipeline script mirroring fea-lite/cutlist-optimizer's (`--target web` glue → web repo `public/apps/proof-watermark/`)
- [ ] Hello-world build loads through the standalone dev page (`trunk serve`)
- [ ] git init + first commit

## Phase 1 — Core engine (wrap tpt-wasmforge)
- [ ] Survey `tpt-wasmforge` for existing image compositing/text-overlay and
      EXIF read/strip support
- [ ] Adapter crate `engine/`: image bytes + watermark spec (text/logo,
      position, opacity, tiling) in → watermarked image bytes out; image
      bytes in → EXIF-stripped bytes out — as plain functions, no UI deps,
      host-unit-testable
- [ ] Validation: watermark renders correctly across a range of image
      sizes/aspect ratios, EXIF strip verified with a metadata inspector

## Phase 2 — Free tier
- [ ] Single-image upload + basic text/logo overlay watermark, download result
- [ ] Free-tier upsell copy pointing at batch folder watermarking, saved
      per-client presets, EXIF stripping
- [ ] Browser-verified: mount, upload, watermark renders correctly

## Phase 3 — Paid desktop edition (cargo feature `pro`)
- [ ] `pro` gates: batch folder watermarking, saved per-client presets
      (position/opacity/branding per client), EXIF stripping
- [ ] Pro UI: folder picker, preset manager, batch progress, EXIF-strip toggle
- [ ] Desktop shell via `tpt-appfront-webview`; Windows exe build + smoke test
- [ ] Gumroad product from apps.md listing copy ($15), exe upload, URL into
      registry — manual step, listing copy at apps.md §T2-13

## Phase 4 — Ship & measure
- [ ] Registry entry (`utility` category, `wasm: { entry: '/apps/proof-watermark/proof-watermark.js' }`)
      per the "Adding a finished app to the hub" runbook at the bottom of apps.md
- [ ] Verify hub card/detail/sitemap/runner in dev, then prod after deploy
- [ ] WASM size budget check on the shipped build
