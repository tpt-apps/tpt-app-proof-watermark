# Gumroad listing

Everything needed to create/update the Gumroad product once this app is
built. Source copy is `apps.md` §T2-13 in the `tpt-electrician-nz-2` repo — if
that changes, update here too (and vice versa).

## Step-by-step: publish this listing

1. Build the Pro edition (`./build.ps1 -Pro`, once the build pipeline exists
   — see `todo.md` and mirror `tpt-app-fea-lite`/`tpt-app-cutlist-optimizer`'s
   pattern) — produces the packaged exe/zip in `release/`.
2. Log in to Gumroad → **New product** → **Digital product**.
3. Set the product name, price, and upload the packaged file (all below).
4. Paste the short description, bullets, and full description (below) into
   their respective fields.
5. Set category/tags (below) and add a cover image (see note below).
6. Publish, then copy the resulting product URL (`https://gum.co/...`).
7. Paste that URL into the `proof-watermark` entry's `gumroadUrl` field in
   `tpt-electrician-nz-2\src\lib\tpt-apps-registry.ts`, and set `price: '$15'`
   on the same entry.
8. Flip that entry's `status` to `'live'` once both the free demo and this
   Gumroad listing exist.
9. Commit and push the registry repo.

## File to upload

`release/TPT-Proof-Watermarker-Pro.zip` — produced by `./build.ps1 -Pro`
(the zip contains the standalone exe plus a README; the exe embeds the web
bundle, so customers get a single double-clickable file). Rebuild with
`./build.ps1 -Pro` whenever the app changes before publishing.

## Product name

```
TPT Proof Watermarker — Batch Client Proof Watermarking for Photographers
```

## Price

**$15 USD, one-time purchase** (no subscription).

## Short description / summary field

```
Watermark an entire shoot at once, ready to send as client proofs before final payment.
```

## Bullets / feature list

```
Batch folder watermarking
Saved per-client presets
EXIF stripping
Works offline
One-time purchase
```

## Full description (long-form field)

```
TPT Proof Watermarker batch-watermarks an entire shoot with consistent
branding and folder structure matching how photographers actually deliver
proof galleries to clients before the paid, unwatermarked handoff.

Who it's for: wedding, event, and stock photographers who send watermarked
proofs to clients before final payment — a specific, universal workflow in
that industry that generic one-off watermarking tools don't build around.

Pro edition (this download):
- Batch folder watermarking (free is one image at a time)
- Saved branding presets per client
- EXIF stripping for privacy before sending proofs

Try the free single-image edition first, right in your browser, no install:
tptsolutions.co.nz/tools/proof-watermark

Requirements: Windows 10 (20H2+) or Windows 11. Uses the WebView2 runtime,
preinstalled on both — nothing extra to download.
```

## Category / tags

- Category: **Software > Utility** (or Gumroad's closest equivalent)
- Tags: `photography-tools`, `watermarking`, `client-proofs`,
  `wedding-photography`, `batch-processing`, `windows`

## Cover image / thumbnail

Not created yet. A before/after of a watermarked proof gallery is the most
convincing cover once built. No stock/AI art. (A quick option: load a proof
in the app, watermark it, screenshot the before/after preview.)

## After publishing

1. Copy the resulting product URL (`https://gum.co/...`).
2. Paste it into the `gumroadUrl` field of the `proof-watermark` entry in
   `tpt-electrician-nz-2\src\lib\tpt-apps-registry.ts`, and set `price: '$15'`.
3. Set `status` to `'live'`.
4. Commit and push the registry repo.
