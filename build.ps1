# TPT Proof Watermarker - build pipeline (Windows).
#
# Free edition (default):  ./build.ps1
#   cargo build --release --target wasm32-unknown-unknown (no pro feature)
#   wasm-bindgen --target web  ->  web repo public/apps/proof-watermark/ when
#   TPT_WEB_REPO is set (glue named proof-watermark.js, wasm keeps its
#   _bg.wasm name - the glue resolves it via import.meta.url), otherwise
#   dist\web.
# Pro desktop edition:     ./build.ps1 -Pro
#   trunk build --release --features pro (its own independent wasm build,
#   into dist\ - NOT the wasm-bindgen path above) + cargo build the native
#   webview exe, staged with a README into release\TPT Proof Watermarker Pro\
#   and zipped for Gumroad. Deliberately does not touch
#   public/apps/proof-watermark/: that is the free in-browser demo's bundle,
#   and it must never be built with --features pro, or the site would give
#   away every paid feature (batch folder watermarking, per-client presets,
#   EXIF stripping) for free. The two editions are separate cargo builds
#   writing to separate directories on purpose.
#
# Env:
#   TPT_WEB_REPO  path to the tptsolutions.co.nz web repo; the app is copied
#                 to <repo>\public\apps\proof-watermark\ for the hub runner.
#                 Defaults to the known local checkout below if unset.

[CmdletBinding()]
param(
    # Build the Pro desktop bundle (--features pro) instead of the free one.
    # Never writes to the free edition's $Dest - see header comment.
    [switch]$Pro,
    # Free-edition only: skip the cargo build and re-run wasm-bindgen against
    # an existing release build.
    [switch]$SkipBuild,
    # Override the web repo destination.
    [string]$WebRepo
)

$ErrorActionPreference = "Stop"
Set-Location -LiteralPath $PSScriptRoot

$Crate = "tpt-app-proof-watermark"
$Target = "wasm32-unknown-unknown"
$OutName = "proof-watermark"
$Slug = "proof-watermark"
$DefaultWebRepo = "D:\Programming\2 WIP\TPT Electrician\tpt-electrician-nz-2"
$GzipBudget = 1.5MB

Write-Host "== TPT Proof Watermarker build ==" -ForegroundColor Cyan

if ($Pro) {
    Write-Host " edition: pro (batch folder watermarking + per-client presets + EXIF stripping)"
    Write-Host "-- trunk build --release --features pro (standalone bundle for the desktop shell)" -ForegroundColor Cyan
    trunk build --release --features pro
    if ($LASTEXITCODE -ne 0) { throw "trunk build failed" }

    Write-Host "-- cargo build --release -p tpt-proof-desktop (webview exe)" -ForegroundColor Cyan
    cargo build --release -p tpt-proof-desktop
    if ($LASTEXITCODE -ne 0) { throw "desktop build failed" }

    Write-Host "-- staging release\TPT Proof Watermarker Pro\" -ForegroundColor Cyan
    $Stage = Join-Path $PSScriptRoot "release\TPT Proof Watermarker Pro"
    New-Item -ItemType Directory -Force -Path $Stage | Out-Null
    Copy-Item "target\release\tpt-proof-watermarker-pro.exe" $Stage -Force
    $readme = @"
TPT Proof Watermarker Pro
=========================

Double-click tpt-proof-watermarker-pro.exe to run. Nothing to install — the
app uses the WebView2 runtime that ships with Windows 10 (20H2+) and
Windows 11.

This edition unlocks, for life, on this machine:
  - Batch folder watermarking (free is one image at a time)
  - Saved branding presets per client
  - EXIF stripping for privacy before sending proofs

Purchased from tptsolutions.co.nz — thank you for supporting one-time,
no-subscription software.
"@
    Set-Content -Path (Join-Path $Stage "README.txt") -Value $readme -Encoding UTF8
    $Zip = Join-Path $PSScriptRoot "release\TPT-Proof-Watermarker-Pro.zip"
    if (Test-Path $Zip) { Remove-Item $Zip -Force }
    Compress-Archive -Path $Stage -DestinationPath $Zip
    Write-Host "   exe: target\release\tpt-proof-watermarker-pro.exe (serves dist\)" -ForegroundColor Cyan
    Write-Host "   zip: $Zip" -ForegroundColor Cyan
    Write-Host "-- done" -ForegroundColor Green
    Write-Host "   (public/apps/proof-watermark/ was not touched — run ./build.ps1 with no flags to refresh the free demo)"
    exit 0
}

# Where the browser-loadable free bundle goes.
$WebRepo = if ($WebRepo) { $WebRepo } elseif ($env:TPT_WEB_REPO) { $env:TPT_WEB_REPO } elseif (Test-Path $DefaultWebRepo) { $DefaultWebRepo } else { $null }
$Dest = if ($WebRepo) {
    Join-Path $WebRepo "public\apps\proof-watermark"
} else {
    Join-Path $PSScriptRoot "dist\web"
}

Write-Host " edition: free (single image, basic text/logo overlay)"
Write-Host " dest:    $Dest"

if (-not $SkipBuild) {
    Write-Host "-- cargo build --release --target $Target" -ForegroundColor Cyan
    cargo build --release --target $Target -p $Crate
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
}

$Wasm = Join-Path $PSScriptRoot "target\$Target\release\tpt_app_proof_watermark.wasm"
if (-not (Test-Path $Wasm)) { throw "missing $Wasm - run the cargo build step first" }

Write-Host "-- wasm-bindgen --target web --out-name $OutName" -ForegroundColor Cyan
New-Item -ItemType Directory -Force -Path $Dest | Out-Null
wasm-bindgen --target web --out-name $OutName --out-dir $Dest $Wasm
if ($LASTEXITCODE -ne 0) { throw "wasm-bindgen failed" }

$Glue = Join-Path $Dest "$OutName.js"
$BgWasm = Join-Path $Dest "$($OutName)_bg.wasm"
foreach ($artifact in @($Glue, $BgWasm)) {
    if (-not (Test-Path $artifact)) { throw "wasm-bindgen did not produce $artifact" }
}

# Payload size report (todo Phase 4 budget: image codecs are the bulk).
$raw = (Get-Item $BgWasm).Length
Add-Type -AssemblyName System.IO.Compression
$rawBytes = [System.IO.File]::ReadAllBytes($BgWasm)
$memoryStream = [System.IO.MemoryStream]::new()
$gzipStream = [System.IO.Compression.GZipStream]::new($memoryStream, [System.IO.Compression.CompressionLevel]::Optimal)
$gzipStream.Write($rawBytes, 0, $rawBytes.Length)
$gzipStream.Dispose()
$gzipped = $memoryStream.ToArray().Length
$memoryStream.Dispose()
$gzKb = [math]::Round($gzipped / 1KB, 1)
Write-Host "-- $(Split-Path $BgWasm -Leaf): $raw bytes raw, $gzipped bytes gzipped ($gzKb KB gz)" -ForegroundColor Cyan
if ($gzipped -gt $GzipBudget) {
    Write-Warning "payload exceeds the ~1.5 MB gzip budget for the image-codec build"
}

Write-Host "-- done" -ForegroundColor Green
if (-not $WebRepo) {
    Write-Host "   (set TPT_WEB_REPO to copy the bundle into the hub web repo)"
} else {
    Write-Host ""
    Write-Host "Registry entry (src/lib/tpt-apps-registry.ts), then flip status to 'live':" -ForegroundColor Cyan
    Write-Host @"
  {
    slug: '$Slug',
    name: 'TPT Proof Watermarker',
    description: 'Watermark client proofs before final payment — in your browser, on-device.',
    category: 'utility',
    status: 'coming-soon',
    wasm: { entry: '/apps/$Slug/$OutName.js' },
    gumroadUrl: 'https://tptsolutions.gumroad.com/l/proof-watermarker',
    price: '`$15',
  },
"@
}
