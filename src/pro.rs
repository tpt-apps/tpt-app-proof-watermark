//! Pro edition extras: batch folder watermarking, saved per-client presets,
//! and EXIF-only stripping.
//!
//! Everything here compiles only with `--features pro` on wasm32, so the
//! free hub bundle physically ships none of it. The webview sandbox has no
//! filesystem access, so the UI drives the desktop shell's file-bridge IPC
//! (`window.__appfront.post`, replies resolved as promises):
//!
//! - `dialog.open` (dir) — native folder picker
//! - `list_images {path}` → `[{name, path, size}]`
//! - `read_file_b64 {path}` → `{data}` (base64)
//! - `write_file_b64 {path, data}` — shell confines writes to the picked root
//!
//! Batch watermarking writes to `<folder>/watermarked/`; EXIF-only stripping
//! writes to `<folder>/stripped/` and rewrites container metadata without
//! re-encoding pixels. Watermarked output never carries the input's EXIF
//! either way (the pipeline decodes and re-encodes), which the card states.
//!
//! Per-client presets persist in localStorage (`pw-presets`) — inside the
//! desktop shell that storage survives restarts, so "Smith Wedding" keeps its
//! position/opacity/branding between shoots. Small logos are embedded in the
//! preset; oversized ones are refused with a note rather than silently
//! dropped.

use std::rc::Rc;

use base64::Engine as _;
use js_sys::{JsString, Object, Reflect};
use tpt_appfront_core::{ContainerBuilder, Signal, UITree};
use tpt_proof_engine::spec::{WatermarkContent, WatermarkSpec};
use tpt_proof_engine::{apply_watermark, detect_format, strip_metadata, ImageFormat};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;

use crate::app::{self, Ctx, Status};

/// Preset storage key (localStorage; survives restarts in the shell).
const PRESETS_KEY: &str = "pw-presets";
/// Refuse to embed logos bigger than this (base64 chars) in a preset —
/// localStorage quotas are ~5 MB and a watermark logo should be tiny.
const LOGO_B64_BUDGET: usize = 400_000;

#[derive(Clone)]
pub enum BatchState {
    Idle,
    Running { done: usize, total: usize, current: String },
    Done { ok: usize, failed: usize, out_dir: String },
    Failed(String),
}

#[derive(Clone)]
pub struct ProCtx {
    pub batch: Signal<BatchState>,
    pub presets: Signal<Vec<Preset>>,
}

/// One saved per-client preset, exactly as stored in localStorage.
#[derive(Clone)]
pub struct Preset {
    pub name: String,
    pub json: String,
    /// Embedded logo: (file name, base64) when the preset carries one.
    pub logo: Option<(String, String)>,
}

fn window() -> web_sys::Window {
    web_sys::window().expect("no window")
}

fn js_err_string(e: JsValue) -> String {
    e.as_string().unwrap_or_else(|| "unknown shell error".to_string())
}

// ------------------------------------------------------------- shell extras

/// The pro-only card added to the controls column of the main shell.
pub fn card_tree() -> UITree<crate::app::Msg> {
    let mut b = ContainerBuilder::<crate::app::Msg>::new();
    b.container(|c| {
        c.container(|c| { c.text("Batch & presets (Pro)"); }).class("pw-card-title");
        c.container(|c| {
            c.text(
                "Save the settings above as a per-client preset, then batch-watermark a \
                 whole folder with them. Watermarked output never carries EXIF; use \
                 EXIF-strip only for finals you send without watermarking.",
            );
        })
        .class("pw-hint");
        c.container(|c| {
            c.input("").attr("placeholder", "Client name, e.g. Smith Wedding")
                .class("pw-preset-name");
            c.button("Save preset")
                .class("pw-btn")
                .on_click(crate::app::Msg::ProSavePreset);
        })
        .class("pw-row");
        c.container(|_c| {}).class("pw-presets-root");
        c.container(|c| {
            c.button("Watermark folder\u{2026}")
                .class("pw-btn pw-btn-primary")
                .on_click(crate::app::Msg::ProBatchWatermarkFolder);
            c.button("EXIF-strip only\u{2026}")
                .class("pw-btn")
                .on_click(crate::app::Msg::ProStripOnlyFolder);
        })
        .class("pw-row pw-actions");
        c.container(|_c| {}).class("pw-batch-root");
    })
    .class("pw-card pw-card-pro");
    b.into_only_child().expect("single pro card root")
}

// ------------------------------------------------------------------ islands

pub fn mount_island(
    container: &web_sys::Element,
    pro_ctx: ProCtx,
    dispatch: Rc<dyn Fn(crate::app::Msg)>,
) -> Result<(), JsValue> {
    // Batch progress island.
    let root = container
        .query_selector(".pw-batch-root")
        .ok()
        .flatten()
        .ok_or_else(|| JsValue::from_str("missing .pw-batch-root"))?;
    let batch_ctx = pro_ctx.clone();
    let handle = tpt_appfront_dom::render(
        &root,
        Rc::new(move || batch_view(&batch_ctx)),
        dispatch.clone(),
    )?;
    std::mem::forget(handle);

    // Preset list island.
    let root = container
        .query_selector(".pw-presets-root")
        .ok()
        .flatten()
        .ok_or_else(|| JsValue::from_str("missing .pw-presets-root"))?;
    let handle = tpt_appfront_dom::render(
        &root,
        Rc::new(move || presets_view(&pro_ctx)),
        dispatch,
    )?;
    std::mem::forget(handle);
    Ok(())
}

fn batch_view(pro_ctx: &ProCtx) -> UITree<crate::app::Msg> {
    let mut b = ContainerBuilder::<crate::app::Msg>::new();
    b.container(|c| match pro_ctx.batch.get() {
        BatchState::Idle => {}
        BatchState::Running { done, total, current } => {
            let pct = if total == 0 { 0 } else { done * 100 / total };
            c.container(|c| { c.text(format!("Processing {done}/{total} ({pct}%)")); })
                .class("pw-batch-note");
            c.container(|c| { c.text(current); }).class("pw-fileinfo");
            c.container(|_c| {})
                .class("pw-progress")
                .attr("data-pct", &pct.to_string());
        }
        BatchState::Done { ok, failed, out_dir } => {
            c.container(|c| {
                c.text(format!(
                    "Done: {ok} written{} → {out_dir}",
                    if failed > 0 { format!(", {failed} failed") } else { String::new() }
                ));
            })
            .class("pw-batch-note");
        }
        BatchState::Failed(err) => {
            c.container(|c| { c.text(err); }).class("pw-error");
        }
    })
    .class("pw-batch");
    b.into_only_child().expect("single batch root")
}

fn presets_view(pro_ctx: &ProCtx) -> UITree<crate::app::Msg> {
    let presets = pro_ctx.presets.get();
    let mut b = ContainerBuilder::<crate::app::Msg>::new();
    if presets.is_empty() {
        b.container(|c| { c.text("No presets saved yet."); })
            .class("pw-batch-note");
    } else {
        for preset in presets.iter() {
            let name = preset.name.clone();
            let logo_note = if preset.logo.is_some() { " · logo" } else { "" };
            b.container(|c| {
                c.container(|c| { c.text(format!("{name}{logo_note}")); })
                    .class("pw-fileinfo");
                c.button("Load")
                    .class("pw-btn")
                    .on_click(crate::app::Msg::ProLoadPreset(name.clone()));
                c.button("Delete")
                    .class("pw-btn pw-btn-danger")
                    .on_click(crate::app::Msg::ProDeletePreset(name));
            })
            .class("pw-row pw-preset-row");
        }
    }
    b.into_only_child().expect("single presets root")
}

// --------------------------------------------------------------- JS bridge

/// `window.__appfront` only exists inside the desktop shell; the hub page
/// never gets pro UI anyway, but this guards double.
fn appfront() -> Option<js_sys::Object> {
    let w = window();
    let value = Reflect::get(&w, &JsString::from("__appfront").into()).ok()?;
    value.dyn_into::<Object>().ok()
}

fn next_request_id() -> u32 {
    thread_local! {
        static COUNTER: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    }
    COUNTER.with(|c| {
        let n = c.get() + 1;
        c.set(n);
        n
    })
}

/// Await a shell command reply: `post(action, params, requestId)` resolves
/// via `window.__appfrontResolve`.
async fn post(action: &str, params: &Object) -> Result<JsValue, JsValue> {
    let af = appfront().ok_or_else(|| JsValue::from_str("not running in the desktop shell"))?;
    let post_fn: js_sys::Function = Reflect::get(&af, &JsString::from("post").into())?
        .dyn_into()
        .map_err(|_| JsValue::from_str("__appfront.post missing"))?;
    let id = next_request_id().to_string();
    let promise = post_fn.call3(
        &af,
        &JsString::from(action).into(),
        params,
        &JsString::from(id).into(),
    )?;
    let future = JsFuture::from(promise.dyn_into::<js_sys::Promise>()?);
    future.await
}

fn str_param(obj: &Object, name: &str, value: &str) -> Result<(), JsValue> {
    Reflect::set(obj, &JsString::from(name).into(), &JsString::from(value).into())
        .map(|_| ())
        .map_err(|_| JsValue::from_str("failed to set param"))
}

async fn pick_folder(title: &str) -> Result<Option<String>, JsValue> {
    let params = Object::new();
    str_param(&params, "title", title)?;
    Reflect::set(&params, &JsString::from("dir").into(), &true.into())?;
    let reply = post("dialog.open", &params).await?;
    let path = Reflect::get(&reply, &JsString::from("path").into())?;
    if path.is_undefined() || path.is_null() {
        return Ok(None);
    }
    Ok(Some(path.as_string().ok_or_else(|| JsValue::from_str("bad path"))?))
}

struct FileEntry {
    name: String,
    path: String,
}

async fn list_images(path: &str) -> Result<Vec<FileEntry>, JsValue> {
    let params = Object::new();
    str_param(&params, "path", path)?;
    let reply = post("list_images", &params).await?;
    let files = Reflect::get(&reply, &JsString::from("files").into())?;
    let files: js_sys::Array = files.dyn_into()?;
    let mut out = Vec::new();
    for entry in files.iter() {
        let name = Reflect::get(&entry, &JsString::from("name").into())?
            .as_string()
            .unwrap_or_default();
        let fpath = Reflect::get(&entry, &JsString::from("path").into())?
            .as_string()
            .unwrap_or_default();
        out.push(FileEntry { name, path: fpath });
    }
    Ok(out)
}

async fn read_file_b64(path: &str) -> Result<Vec<u8>, JsValue> {
    let params = Object::new();
    str_param(&params, "path", path)?;
    let reply = post("read_file_b64", &params).await?;
    let data = Reflect::get(&reply, &JsString::from("data").into())?
        .as_string()
        .ok_or_else(|| JsValue::from_str("missing data"))?;
    base64::engine::general_purpose::STANDARD
        .decode(data.as_bytes())
        .map_err(|e| JsValue::from_str(&format!("bad base64 from shell: {e}")))
}

async fn write_file_b64(path: &str, bytes: &[u8]) -> Result<(), JsValue> {
    let params = Object::new();
    str_param(&params, "path", path)?;
    Reflect::set(
        &params,
        &JsString::from("data").into(),
        &JsString::from(base64::engine::general_purpose::STANDARD.encode(bytes)).into(),
    )?;
    post("write_file_b64", &params).await?;
    Ok(())
}

// ----------------------------------------------------------------- settings

/// Settings snapshot for one batch run / preset. Read from the same controls
/// the single-image flow uses, so what you set is what the batch applies.
pub(crate) struct Settings {
    pub spec: WatermarkSpec,
    /// True when the output select says "keep": resolve each file's format
    /// from its own bytes instead of a fixed one.
    pub keep_format: bool,
    pub format: ImageFormat,
    pub quality: u8,
    pub logo: Option<(String, Vec<u8>)>,
}

pub(crate) fn read_settings(ctx: &Ctx) -> Settings {
    let container = &ctx.container;
    let select_val = app::read_select(container, ".pw-format").unwrap_or_else(|| "keep".into());
    let keep_format = select_val == "keep";
    // Fixed format for preset/batch when the select isn't "keep"; "keep"
    // resolves per-file inside the loop (PNG as the lossless-safe fallback).
    let format = match select_val.as_str() {
        "jpeg" => ImageFormat::Jpeg,
        "png" => ImageFormat::Png,
        _ => ctx.source_format().unwrap_or(ImageFormat::Png),
    };
    let quality = app::read_number(container, ".pw-quality").clamp(50.0, 100.0) as u8;
    let spec = app::read_spec(container);
    let logo = if matches!(spec.content, WatermarkContent::Logo) {
        ctx.logo.get().map(|l| (l.name, l.bytes))
    } else {
        None
    };
    Settings { spec, keep_format, format, quality, logo }
}

impl Settings {
    fn per_file_format(&self, input: &[u8]) -> ImageFormat {
        if self.keep_format {
            detect_format(input).unwrap_or(ImageFormat::Png)
        } else {
            self.format
        }
    }

    /// Serialize to preset JSON.
    fn to_preset_json(&self) -> String {
        let obj = Object::new();
        let set = |k: &str, v: JsValue| {
            let _ = Reflect::set(&obj, &JsString::from(k).into(), &v);
        };
        match &self.spec.content {
            WatermarkContent::Text { text, color } => {
                set("kind", "text".into());
                set("text", text.as_str().into());
                set("color", if color.0[0] == 0 { "black" } else { "white" }.into());
            }
            WatermarkContent::Logo => set("kind", "logo".into()),
        }
        set("position", position_slug(self.spec.position).into());
        set("scale", self.spec.scale.into());
        set("opacity", self.spec.opacity.into());
        set("tile", self.spec.tile.into());
        set("format", match self.format {
            ImageFormat::Jpeg => "jpeg",
            ImageFormat::Png => "png",
        }.into());
        set("quality", self.quality.into());
        js_sys::JSON::stringify(&obj)
            .ok()
            .and_then(|s| s.as_string())
            .unwrap_or_else(|| "{}".into())
    }

    /// Base64 of the preset logo, if one should be embedded.
    fn logo_b64(&self) -> Option<(String, String)> {
        let (name, bytes) = self.logo.as_ref()?;
        let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
        if b64.len() > LOGO_B64_BUDGET {
            return None;
        }
        Some((name.clone(), b64))
    }

    fn logo_too_big(&self) -> bool {
        self.logo
            .as_ref()
            .map(|(_, b)| base64::engine::general_purpose::STANDARD.encode(b).len() > LOGO_B64_BUDGET)
            .unwrap_or(false)
    }
}

fn position_slug(p: tpt_proof_engine::spec::Position) -> &'static str {
    use tpt_proof_engine::spec::Position::*;
    match p {
        TopLeft => "tl",
        TopRight => "tr",
        Center => "c",
        BottomLeft => "bl",
        BottomRight => "br",
    }
}

/// Apply a stored preset JSON to the UI controls.
fn apply_preset_json(ctx: &Ctx, json: &str, logo: Option<(String, Vec<u8>)>) -> Result<(), String> {
    let parsed = js_sys::JSON::parse(json).map_err(|_| "corrupt preset".to_string())?;
    let get = |k: &str| Reflect::get(&parsed, &JsString::from(k).into()).ok();

    let str_of = |k: &str| get(k).and_then(|v| v.as_string()).unwrap_or_default();
    let num_of = |k: &str, default: f64| get(k).and_then(|v| v.as_f64()).unwrap_or(default);

    let container = &ctx.container;
    match str_of("kind").as_str() {
        "logo" => {
            app::write_select(container, ".pw-type", "logo");
            if let Some((name, bytes)) = logo {
                if let Some(label) = app::query(container, ".pw-logo-name") {
                    label.set_text_content(Some(&name));
                }
                ctx.logo.set(Some(app::LogoFile { name, bytes }));
            }
        }
        _ => {
            app::write_select(container, ".pw-type", "text");
            app::write_input(container, ".pw-text", &str_of("text"));
            app::write_select(
                container,
                ".pw-color",
                if str_of("color") == "black" { "black" } else { "white" },
            );
        }
    }
    app::write_select(container, ".pw-position", &str_of("position"));
    app::write_input(
        container,
        ".pw-size",
        &format!("{}", (num_of("scale", 0.04) * 100.0).round()),
    );
    app::write_input(
        container,
        ".pw-opacity",
        &format!("{}", (num_of("opacity", 0.4) * 100.0).round()),
    );
    app::write_checked(container, ".pw-tile", get("tile").map(|v| v.is_truthy()).unwrap_or(false));
    app::write_select(container, ".pw-format", &str_of("format"));
    app::write_input(container, ".pw-quality", &format!("{}", num_of("quality", 90.0).round()));

    // Refresh the conditional rows the same way live edits do.
    if let Some(document) = window().document() {
        app::refresh_rows_after_preset(ctx, &document);
    }
    Ok(())
}

// ------------------------------------------------------------------ presets

pub fn load_presets() -> Vec<Preset> {
    let raw = match app::storage_get(PRESETS_KEY) {
        Some(raw) => raw,
        None => return Vec::new(),
    };
    let parsed = match js_sys::JSON::parse(&raw) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let Ok(arr) = parsed.dyn_into::<js_sys::Array>() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in arr.iter() {
        let name = Reflect::get(&entry, &JsString::from("name").into())
            .ok()
            .and_then(|v| v.as_string())
            .unwrap_or_default();
        let json = Reflect::get(&entry, &JsString::from("settings").into())
            .ok()
            .and_then(|v| v.as_string())
            .unwrap_or_default();
        let logo = Reflect::get(&entry, &JsString::from("logo").into())
            .ok()
            .and_then(|v| v.as_string())
            .and_then(|logo_name| {
                let data = Reflect::get(&entry, &JsString::from("logoData").into())
                    .ok()
                    .and_then(|v| v.as_string())?;
                Some((logo_name, data))
            });
        if !name.is_empty() && !json.is_empty() {
            out.push(Preset { name, json, logo });
        }
    }
    out
}

fn store_presets(presets: &[Preset]) {
    let arr = js_sys::Array::new();
    for p in presets {
        let obj = Object::new();
        let _ = Reflect::set(&obj, &JsString::from("name").into(), &p.name.as_str().into());
        let _ = Reflect::set(&obj, &JsString::from("settings").into(), &p.json.as_str().into());
        if let Some((name, data)) = &p.logo {
            let _ = Reflect::set(&obj, &JsString::from("logo").into(), &name.as_str().into());
            let _ = Reflect::set(&obj, &JsString::from("logoData").into(), &data.as_str().into());
        }
        arr.push(&obj);
    }
    let json = js_sys::JSON::stringify(&arr)
        .ok()
        .and_then(|s| s.as_string())
        .unwrap_or_else(|| "[]".into());
    app::storage_set(PRESETS_KEY, &json);
}

pub(crate) fn save_preset(ctx: &Ctx, pro_ctx: &ProCtx) {
    let name = app::query(&ctx.container, ".pw-preset-name")
        .and_then(|el| el.dyn_into::<web_sys::HtmlInputElement>().ok())
        .map(|i| i.value().trim().to_string())
        .unwrap_or_default();
    if name.is_empty() {
        ctx.status
            .set(Status::Failed("Give the preset a name first (e.g. the client's name).".into()));
        return;
    }
    let settings = read_settings(ctx);
    if settings.spec.content == WatermarkContent::Logo && settings.logo.is_none() {
        ctx.status
            .set(Status::Failed("Logo presets need a logo loaded (Choose logo…).".into()));
        return;
    }
    let too_big = settings.logo_too_big();
    let mut presets = pro_ctx.presets.get();
    presets.retain(|p| p.name != name);
    let logo = (!too_big).then(|| settings.logo_b64()).flatten();
    presets.push(Preset {
        name: name.clone(),
        json: settings.to_preset_json(),
        logo,
    });
    store_presets(&presets);
    pro_ctx.presets.set(presets);
    if let Some(input) = app::query(&ctx.container, ".pw-preset-name")
        .and_then(|el| el.dyn_into::<web_sys::HtmlInputElement>().ok())
    {
        input.set_value("");
    }
    ctx.status.set(Status::Done(format!(
        "Preset \u{201C}{name}\u{201D} saved.{}",
        if too_big {
            " (Logo too large to embed — reload the logo file after loading this preset.)"
        } else {
            ""
        }
    )));
}

pub(crate) fn load_preset(ctx: &Ctx, pro_ctx: &ProCtx, name: &str) {
    let Some(preset) = pro_ctx.presets.get().into_iter().find(|p| p.name == name) else {
        return;
    };
    let logo = preset.logo.as_ref().and_then(|(logo_name, data)| {
        base64::engine::general_purpose::STANDARD
            .decode(data.as_bytes())
            .ok()
            .map(|bytes| (logo_name.clone(), bytes))
    });
    match apply_preset_json(ctx, &preset.json, logo) {
        Ok(()) => ctx
            .status
            .set(Status::Done(format!("Preset \u{201C}{name}\u{201D} applied — check the settings above."))),
        Err(e) => ctx.status.set(Status::Failed(format!("Could not load preset: {e}"))),
    }
}

pub(crate) fn delete_preset(pro_ctx: &ProCtx, name: &str) {
    let mut presets = pro_ctx.presets.get();
    presets.retain(|p| p.name != name);
    store_presets(&presets);
    pro_ctx.presets.set(presets);
}

// ------------------------------------------------------------------- batch

/// Run the current settings over a picked folder, writing to
/// `<folder>/watermarked/`.
pub async fn run_batch(ctx: Ctx, pro_ctx: ProCtx) {
    if appfront().is_none() {
        pro_ctx
            .batch
            .set(BatchState::Failed(
                "Batch processing runs in the Pro desktop edition.".into(),
            ));
        return;
    }
    let folder = match pick_folder("Choose the shoot folder to watermark").await {
        Ok(Some(f)) => f,
        Ok(None) => return, // cancelled
        Err(e) => {
            pro_ctx.batch.set(BatchState::Failed(format!("Folder picker failed: {e:?}")));
            return;
        }
    };
    let files = match list_images(&folder).await {
        Ok(f) => f,
        Err(e) => {
            pro_ctx.batch.set(BatchState::Failed(format!("Could not list folder: {e:?}")));
            return;
        }
    };
    if files.is_empty() {
        pro_ctx
            .batch
            .set(BatchState::Failed("No JPEG/PNG images in that folder.".into()));
        return;
    }
    let settings = read_settings(&ctx);
    if matches!(settings.spec.content, WatermarkContent::Logo) && settings.logo.is_none() {
        ctx.status.set(Status::Failed(
            "Logo watermarking needs a logo loaded (Watermark → Choose logo…).".into(),
        ));
        return;
    }
    let total = files.len();
    let out_dir = format!("{}/watermarked", folder.trim_end_matches(['/']));

    let mut ok = 0usize;
    let mut failed = 0usize;
    for (i, file) in files.iter().enumerate() {
        pro_ctx.batch.set(BatchState::Running {
            done: i,
            total,
            current: file.name.clone(),
        });
        let result = async {
            let input = read_file_b64(&file.path).await.map_err(js_err_string)?;
            let format = settings.per_file_format(&input);
            let bytes = apply_watermark(
                &input,
                &settings.spec,
                settings.logo.as_ref().map(|(_, b)| b.as_slice()),
                format,
                settings.quality,
            )
            .map_err(|e| e.to_string())?;
            let ext = match format {
                ImageFormat::Jpeg => "jpg",
                ImageFormat::Png => "png",
            };
            let out_path = format!("{out_dir}/{}-proof.{ext}", app::file_stem(&file.name));
            write_file_b64(&out_path, &bytes).await.map_err(js_err_string)
        }
        .await;
        match result {
            Ok(()) => ok += 1,
            Err(_) => failed += 1,
        }
    }
    pro_ctx
        .batch
        .set(BatchState::Done { ok, failed, out_dir });
}

/// EXIF-only pass over a folder: rewrites container metadata without
/// re-encoding pixels, writing to `<folder>/stripped/`.
pub async fn run_strip_only(_ctx: Ctx, pro_ctx: ProCtx) {
    if appfront().is_none() {
        pro_ctx
            .batch
            .set(BatchState::Failed(
                "EXIF stripping runs in the Pro desktop edition.".into(),
            ));
        return;
    }
    let folder = match pick_folder("Choose the folder to strip EXIF from").await {
        Ok(Some(f)) => f,
        Ok(None) => return,
        Err(e) => {
            pro_ctx.batch.set(BatchState::Failed(format!("Folder picker failed: {e:?}")));
            return;
        }
    };
    let files = match list_images(&folder).await {
        Ok(f) => f,
        Err(e) => {
            pro_ctx.batch.set(BatchState::Failed(format!("Could not list folder: {e:?}")));
            return;
        }
    };
    if files.is_empty() {
        pro_ctx
            .batch
            .set(BatchState::Failed("No JPEG/PNG images in that folder.".into()));
        return;
    }
    let total = files.len();
    let out_dir = format!("{}/stripped", folder.trim_end_matches(['/']));

    let mut ok = 0usize;
    let mut failed = 0usize;
    for (i, file) in files.iter().enumerate() {
        pro_ctx.batch.set(BatchState::Running {
            done: i,
            total,
            current: file.name.clone(),
        });
        let result = async {
            let input = read_file_b64(&file.path).await.map_err(js_err_string)?;
            let ext = match detect_format(&input) {
                Some(ImageFormat::Jpeg) => "jpg",
                Some(ImageFormat::Png) => "png",
                None => return Err("not a JPEG/PNG".to_string()),
            };
            let bytes = strip_metadata(&input).map_err(|e| e.to_string())?;
            let out_path = format!("{out_dir}/{}-stripped.{ext}", app::file_stem(&file.name));
            write_file_b64(&out_path, &bytes).await.map_err(js_err_string)
        }
        .await;
        match result {
            Ok(()) => ok += 1,
            Err(_) => failed += 1,
        }
    }
    pro_ctx
        .batch
        .set(BatchState::Done { ok, failed, out_dir });
}
