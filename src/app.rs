//! TPT Proof Watermarker — free edition UI (single image, text/logo overlay).
//!
//! Architecture mirrors the other hub apps (image-toolkit): the shell is
//! mounted once and never reconciled (typing stays put), field values are
//! read straight from the DOM when a button runs, and the pieces that change
//! (status line, upsell card) are small `render` islands over signals. The
//! preview canvas is created imperatively once and drawn into after every
//! operation — never part of the reconciled tree.

// Pro-only helpers (preset write-back, batch file-stem naming, "keep"
// format resolution) are unused in the free build; pro.rs consumes them
// under `--features pro`.
#![cfg_attr(not(feature = "pro"), allow(dead_code))]

use std::rc::Rc;
use tpt_appfront_core::{ContainerBuilder, Signal, UITree};
use tpt_proof_engine::spec::{Color, Position, WatermarkContent, WatermarkSpec};
use tpt_proof_engine::{apply_watermark, detect_format, ImageFormat};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};

#[cfg(feature = "pro")]
use crate::pro;

/// Completions before the Pro upsell card appears (localStorage-backed).
#[cfg(not(feature = "pro"))]
const NUDGE_AFTER: u32 = 2;
#[allow(dead_code)]
const OPS_KEY: &str = "pw-ops-done";

#[cfg(not(feature = "pro"))]
const ENGINE_NOTE: &str =
    "Free edition: one image at a time. The Pro desktop edition ($15, one-time) adds \
     batch folder watermarking, saved per-client presets, and EXIF stripping — see \
     the link above once it appears.";
#[cfg(feature = "pro")]
const ENGINE_NOTE: &str =
    "Pro edition: batch folder watermarking, saved per-client presets, and EXIF \
     stripping are unlocked in this build.";

#[derive(Debug, Clone)]
pub(crate) enum Msg {
    RunWatermark,
    Download,
    Reset,
    #[cfg(feature = "pro")]
    ProBatchWatermarkFolder,
    #[cfg(feature = "pro")]
    ProStripOnlyFolder,
    #[cfg(feature = "pro")]
    ProSavePreset,
    #[cfg(feature = "pro")]
    ProLoadPreset(String),
    #[cfg(feature = "pro")]
    ProDeletePreset(String),
}

/// The untouched upload.
#[derive(Clone)]
struct SourceFile {
    name: String,
    mime: String,
    bytes: Vec<u8>,
    width: u32,
    height: u32,
    format: Option<ImageFormat>,
}

/// The chosen logo overlay (text mode doesn't use it).
#[derive(Clone)]
pub(crate) struct LogoFile {
    pub(crate) name: String,
    pub(crate) bytes: Vec<u8>,
}

/// What the canvas currently shows (original or the watermarked output).
#[derive(Clone)]
struct WorkingImg {
    bytes: Vec<u8>,
    width: u32,
    height: u32,
    /// `None` while still the pristine upload.
    op: Option<ImageFormat>,
}

impl WorkingImg {
    fn file_stem_suffix(&self) -> (&'static str, &'static str) {
        match self.op {
            Some(ImageFormat::Jpeg) => ("-proof", "jpg"),
            Some(ImageFormat::Png) => ("-proof", "png"),
            None => ("", ""),
        }
    }
}

#[derive(Clone)]
pub(crate) enum Status {
    Idle,
    Ready,
    Busy(&'static str),
    Done(String),
    Failed(String),
}

/// Clone-able bundle of everything a handler needs.
#[derive(Clone)]
pub(crate) struct Ctx {
    pub(crate) container: web_sys::Element,
    source: Signal<Option<SourceFile>>,
    pub(crate) logo: Signal<Option<LogoFile>>,
    working: Signal<Option<WorkingImg>>,
    pub(crate) status: Signal<Status>,
    ops_done: Signal<u32>,
}

impl Ctx {
    /// Format of the pristine upload, when one is loaded (batch "keep"
    /// fallback).
    pub(crate) fn source_format(&self) -> Option<ImageFormat> {
        self.source.get().and_then(|s| s.format)
    }
}

pub fn mount_app(container: &web_sys::Element) -> Result<(), JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("no window"))?;
    let document = window
        .document()
        .ok_or_else(|| JsValue::from_str("no document"))?;

    upsert_style(&document, "pw-app-styles", BASE_CSS);

    let source: Signal<Option<SourceFile>> = Signal::new(None);
    let logo: Signal<Option<LogoFile>> = Signal::new(None);
    let working: Signal<Option<WorkingImg>> = Signal::new(None);
    let status: Signal<Status> = Signal::new(Status::Idle);
    let ops_done: Signal<u32> = Signal::new(
        storage_get(OPS_KEY)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0),
    );

    let ctx = Ctx {
        container: container.clone(),
        source: source.clone(),
        logo: logo.clone(),
        working: working.clone(),
        status: status.clone(),
        ops_done: ops_done.clone(),
    };

    #[cfg(feature = "pro")]
    let pro_ctx = pro::ProCtx {
        batch: Signal::new(pro::BatchState::Idle),
        presets: Signal::new(pro::load_presets()),
    };

    let dispatch: Rc<dyn Fn(Msg)> = {
        let ctx = ctx.clone();
        let window = window.clone();
        #[cfg(feature = "pro")]
        let pro_ctx = pro_ctx.clone();
        Rc::new(move |msg: Msg| {
            let ctx = ctx.clone();
            let window = window.clone();
            match msg {
                Msg::RunWatermark => {
                    ctx.status.set(Status::Busy("Watermarking…"));
                    defer(&window, move || run_watermark(&ctx));
                }
                Msg::Download => download_working(&ctx),
                Msg::Reset => reset_working(&ctx),
                #[cfg(feature = "pro")]
                Msg::ProBatchWatermarkFolder => {
                    let pro_ctx = pro_ctx.clone();
                    wasm_bindgen_futures::spawn_local(async move {
                        pro::run_batch(ctx, pro_ctx).await;
                    });
                }
                #[cfg(feature = "pro")]
                Msg::ProStripOnlyFolder => {
                    let pro_ctx = pro_ctx.clone();
                    wasm_bindgen_futures::spawn_local(async move {
                        pro::run_strip_only(ctx, pro_ctx).await;
                    });
                }
                #[cfg(feature = "pro")]
                Msg::ProSavePreset => {
                    let pro_ctx = pro_ctx.clone();
                    pro::save_preset(&ctx, &pro_ctx);
                }
                #[cfg(feature = "pro")]
                Msg::ProLoadPreset(name) => {
                    pro::load_preset(&ctx, &pro_ctx, &name);
                }
                #[cfg(feature = "pro")]
                Msg::ProDeletePreset(name) => {
                    let pro_ctx = pro_ctx.clone();
                    pro::delete_preset(&pro_ctx, &name);
                }
            }
        })
    };

    let ui: UITree<Msg> = shell_tree();
    let root = tpt_appfront_dom::mount(container, &ui, dispatch.clone())?;
    std::mem::forget(root); // page-lifetime root

    // Out-of-band nodes and listeners the tree builder can't express.
    install_canvas(&ctx, &document)?;
    wire_upload_button(&ctx)?;
    wire_dropzone(&ctx)?;
    wire_logo_button(&ctx)?;
    wire_live_sliders(&document);
    wire_format_select(&ctx, &document)?;
    wire_type_select(&ctx, &document)?;

    // Reactive islands.
    let status_root = query(&container, ".pw-status-root")
        .ok_or_else(|| JsValue::from_str("missing .pw-status-root"))?;
    let status_ctx = ctx.clone();
    let handle = tpt_appfront_dom::render(
        &status_root,
        Rc::new(move || status_view(ctx_status(&status_ctx))),
        dispatch.clone(),
    )?;
    std::mem::forget(handle);

    #[cfg(not(feature = "pro"))]
    {
        let upsell_root = query(&container, ".pw-upsell-root")
            .ok_or_else(|| JsValue::from_str("missing .pw-upsell-root"))?;
        let ops_counter = ctx.ops_done.clone();
        let handle = tpt_appfront_dom::render(
            &upsell_root,
            Rc::new(move || upsell_view(ops_counter.get())),
            dispatch,
        )?;
        std::mem::forget(handle);
    }
    #[cfg(feature = "pro")]
    {
        pro::mount_island(container, pro_ctx, dispatch)?;
    }

    Ok(())
}

/// Clone out just the signal reads the status island needs (avoids capturing
/// the container in a render closure).
fn ctx_status(ctx: &Ctx) -> (Status, Option<(String, u32, u32, usize)>, bool) {
    let src = ctx
        .source
        .get()
        .map(|s| (s.name.clone(), s.width, s.height, s.bytes.len()));
    let has_working = ctx.working.get().is_some();
    (ctx.status.get(), src, has_working)
}

// ---------------------------------------------------------------- shell tree

fn shell_tree() -> UITree<Msg> {
    let mut b = ContainerBuilder::<Msg>::new();
    b.container(|c| {
        c.container(|c| {
            c.heading(1, "TPT Proof Watermarker");
            c.container(|c| {
                c.text(
                    "Brand client proofs before final payment — right in your browser. \
                     Your images never leave your device.",
                );
            })
            .class("pw-tagline");
        })
        .class("pw-header");

        c.container(|c| {
            // ---- controls column
            c.container(|c| {
                c.container(|c| {
                    c.container(|c| { c.text("Drop an image here"); }).class("pw-drop-main");
                    c.container(|c| {
                        c.text("or click to browse — JPEG · PNG");
                    })
                    .class("pw-drop-hint");
                })
                .class("pw-drop")
                .attr("role", "button")
                .attr("tabindex", "0");

                c.container(|c| {
                    c.container(|c| { c.text("Watermark"); }).class("pw-card-title");
                    c.container(|c| {
                        c.container(|c| { c.text("Type"); }).class("pw-lbl");
                        c.select(
                            [
                                ("text", "Text"),
                                ("logo", "Logo image (PNG recommended)"),
                            ],
                            "text",
                        )
                        .class("pw-type");
                    })
                    .class("pw-row");
                    c.container(|c| {
                        c.container(|c| { c.text("Text"); }).class("pw-lbl");
                        c.input("\u{A9} PROOF — not for reproduction").class("pw-text");
                    })
                    .class("pw-row pw-text-row");
                    c.container(|c| {
                        c.container(|c| { c.text("Color"); }).class("pw-lbl");
                        c.select(
                            [("white", "White"), ("black", "Black")],
                            "white",
                        )
                        .class("pw-color");
                    })
                    .class("pw-row pw-color-row");
                    c.container(|c| {
                        c.button("Choose logo\u{2026}").class("pw-btn pw-logo-btn");
                        c.container(|c| { c.text("No logo chosen"); })
                            .class("pw-logo-name");
                    })
                    .class("pw-row pw-logo-row");
                    c.container(|c| {
                        c.container(|c| { c.text("Size"); }).class("pw-lbl");
                        c.input("4")
                            .attr("type", "range")
                            .attr("min", "1")
                            .attr("max", "20")
                            .attr("step", "1")
                            .class("pw-size");
                        c.container(|c| { c.text("4%"); }).class("pw-s-val");
                    })
                    .class("pw-row");
                    c.container(|c| {
                        c.container(|c| { c.text("Opacity"); }).class("pw-lbl");
                        c.input("40")
                            .attr("type", "range")
                            .attr("min", "5")
                            .attr("max", "100")
                            .attr("step", "5")
                            .class("pw-opacity");
                        c.container(|c| { c.text("40%"); }).class("pw-o-val");
                    })
                    .class("pw-row");
                    c.container(|c| {
                        c.container(|c| { c.text("Position"); }).class("pw-lbl");
                        c.select(
                            [
                                ("br", "Bottom right"),
                                ("bl", "Bottom left"),
                                ("tr", "Top right"),
                                ("tl", "Top left"),
                                ("c", "Center"),
                            ],
                            "br",
                        )
                        .class("pw-position");
                    })
                    .class("pw-row pw-position-row");
                    c.checkbox("Tile across the whole image", false).class("pw-tile");
                    c.button("Watermark image")
                        .class("pw-btn pw-btn-primary")
                        .on_click(Msg::RunWatermark);
                })
                .class("pw-card");

                c.container(|c| {
                    c.container(|c| { c.text("Output"); }).class("pw-card-title");
                    c.container(|c| {
                        c.container(|c| { c.text("Format"); }).class("pw-lbl");
                        c.select(
                            [
                                ("keep", "Keep original format"),
                                ("jpeg", "JPEG"),
                                ("png", "PNG"),
                            ],
                            "keep",
                        )
                        .class("pw-format");
                    })
                    .class("pw-row");
                    c.container(|c| {
                        c.container(|c| { c.text("JPEG quality"); }).class("pw-lbl");
                        c.input("90")
                            .attr("type", "range")
                            .attr("min", "50")
                            .attr("max", "100")
                            .attr("step", "5")
                            .class("pw-quality");
                        c.container(|c| { c.text("90"); }).class("pw-q-val");
                    })
                    .class("pw-row pw-quality-row");
                })
                .class("pw-card");

                #[cfg(feature = "pro")]
                c.with(pro::card_tree());
                #[cfg(not(feature = "pro"))]
                c.container(|_c| {}).class("pw-upsell-root");
                c.container(|c| { c.text(ENGINE_NOTE); }).class("pw-note");
            })
            .class("pw-controls");

            // ---- preview column
            c.container(|c| {
                c.container(|_c| {}).class("pw-canvas-host");
                c.container(|_c| {}).class("pw-status-root");
            })
            .class("pw-preview");
        })
        .class("pw-main");
    })
    .class("pw-root");
    b.into_only_child().expect("single root")
}

// ------------------------------------------------------------- status island

fn status_view(reads: (Status, Option<(String, u32, u32, usize)>, bool)) -> UITree<Msg> {
    let (status, src, has_working) = reads;
    let mut b = ContainerBuilder::<Msg>::new();
    b.container(|c| {
        file_info_div(c, &src);
        match status {
            Status::Idle if src.is_none() => {
                c.container(|c| {
                    c.text("No image loaded yet — choose or drop an image to get started.");
                })
                .class("pw-status-empty");
            }
            Status::Busy(what) => {
                c.container(|c| { c.text(what); }).class("pw-busy");
            }
            Status::Done(summary) => {
                c.container(|c| { c.text(summary); }).class("pw-summary");
                c.container(|c| {
                    c.button("Download result")
                        .class("pw-btn pw-btn-primary")
                        .on_click(Msg::Download);
                    c.button("Reset to original")
                        .class("pw-btn")
                        .on_click(Msg::Reset);
                })
                .class("pw-row pw-actions");
                if has_working {
                    c.container(|c| {
                        c.text("You're viewing the watermarked result — Reset restores your original upload.");
                    })
                    .class("pw-hint");
                }
            }
            Status::Failed(err) => {
                c.container(|c| { c.text(err); }).class("pw-error");
            }
            Status::Ready | Status::Idle => {}
        }
    })
    .class("pw-status");
    b.into_only_child().expect("single root")
}

/// The "name · WxH · size" line; appended into the tree being built.
fn file_info_div(
    b: &mut ContainerBuilder<Msg>,
    src: &Option<(String, u32, u32, usize)>,
) -> Option<()> {
    let (name, w, h, size) = src.as_ref()?;
    let mut b2 = ContainerBuilder::<Msg>::new();
    b2.container(|c| {
        c.text(format!("{name} · {w}×{h} · {}", format_size(*size)));
    })
    .class("pw-fileinfo");
    b.with(b2.into_only_child().expect("one"));
    Some(())
}

// ------------------------------------------------------------- upsell island

#[cfg(not(feature = "pro"))]
fn upsell_view(ops_done: u32) -> UITree<Msg> {
    let mut b = ContainerBuilder::<Msg>::new();
    if ops_done >= NUDGE_AFTER {
        b.container(|c| {
            c.heading(3, "Watermarking a whole shoot?");
            c.container(|c| {
                c.text(
                    "The Pro desktop edition is built for proof delivery: batch-watermark \
                     an entire folder in one go, save branding presets per client, and \
                     strip EXIF before sending. One-time $15 — no subscription, works \
                     offline.",
                );
            })
            .class("pw-upsell-body");
            c.link(
                "https://tptsolutions.co.nz/tools/proof-watermark",
                "Get TPT Proof Watermarker Pro",
            )
            .attr("target", "_blank")
            .attr("rel", "noreferrer")
            .class("pw-upsell-link");
        })
        .class("pw-upsell");
    } else {
        b.container(|_c| {}).class("pw-upsell-placeholder");
    }
    b.into_only_child().expect("single root")
}

// ----------------------------------------------------------------- handlers

/// The current watermark spec, read straight from the controls.
pub(crate) fn read_spec(container: &web_sys::Element) -> WatermarkSpec {
    let kind = read_select(container, ".pw-type").unwrap_or_else(|| "text".into());
    let content = if kind == "logo" {
        WatermarkContent::Logo
    } else {
        let text = query(container, ".pw-text")
            .and_then(|el| el.dyn_into::<web_sys::HtmlInputElement>().ok())
            .map(|i| i.value())
            .unwrap_or_else(|| "PROOF".into());
        let color = match read_select(container, ".pw-color").as_deref() {
            Some("black") => Color::BLACK,
            _ => Color::WHITE,
        };
        WatermarkContent::Text { text, color }
    };
    let position = match read_select(container, ".pw-position").as_deref() {
        Some("tl") => Position::TopLeft,
        Some("tr") => Position::TopRight,
        Some("bl") => Position::BottomLeft,
        Some("c") => Position::Center,
        _ => Position::BottomRight,
    };
    WatermarkSpec {
        content,
        position,
        scale: (read_number(container, ".pw-size").clamp(1.0, 20.0) / 100.0) as f32,
        opacity: (read_number(container, ".pw-opacity").clamp(5.0, 100.0) / 100.0) as f32,
        tile: read_checked(container, ".pw-tile"),
        tile_gap: 0.25,
    }
}

/// Output format: keep resolves against the source file (unknown sources
/// fall back to PNG, which is lossless-safe).
fn resolve_format(select_val: &str, source: Option<ImageFormat>) -> ImageFormat {
    match select_val {
        "jpeg" => ImageFormat::Jpeg,
        "png" => ImageFormat::Png,
        _ => source.unwrap_or(ImageFormat::Png),
    }
}

fn run_watermark(ctx: &Ctx) {
    let Some(working) = ctx.working.get() else { return };
    let Some(source) = ctx.source.get() else { return };
    let spec = read_spec(&ctx.container);
    let select_val = read_select(&ctx.container, ".pw-format").unwrap_or_else(|| "keep".into());
    let format = resolve_format(&select_val, source.format);
    let quality = read_number(&ctx.container, ".pw-quality").clamp(50.0, 100.0) as u8;
    let logo = if matches!(spec.content, WatermarkContent::Logo) {
        match ctx.logo.get() {
            Some(l) => Some(l.bytes),
            None => {
                ctx.status.set(Status::Failed(
                    "Choose a logo image first (Watermark → Choose logo…).".into(),
                ));
                return;
            }
        }
    } else {
        None
    };

    match apply_watermark(&working.bytes, &spec, logo.as_deref(), format, quality) {
        Ok(bytes) => {
            let before = working.bytes.len();
            let after = bytes.len();
            let fmt_note = match format {
                ImageFormat::Jpeg => "JPEG".to_string(),
                ImageFormat::Png => "PNG".to_string(),
            };
            let out = WorkingImg {
                bytes,
                width: working.width,
                height: working.height,
                op: Some(format),
            };
            ctx.working.set(Some(out));
            note_success(ctx);
            ctx.status.set(Status::Done(format!(
                "Watermarked ({fmt_note}, {} → {})",
                format_size(before),
                format_size(after)
            )));
            draw_working(ctx);
        }
        Err(e) => ctx
            .status
            .set(Status::Failed(format!("Watermarking failed: {e}"))),
    }
}

fn reset_working(ctx: &Ctx) {
    let Some(source) = ctx.source.get() else { return };
    let (width, height) = (source.width, source.height);
    ctx.working.set(Some(WorkingImg {
        bytes: source.bytes.clone(),
        width,
        height,
        op: None,
    }));
    ctx.status.set(Status::Ready);
    draw_working(ctx);
}

fn download_working(ctx: &Ctx) {
    let window = match web_sys::window() {
        Some(w) => w,
        None => return,
    };
    let document = match window.document() {
        Some(d) => d,
        None => return,
    };
    let Some(working) = ctx.working.get() else { return };
    let mime = match working.op {
        Some(f) => f.mime_type().to_string(),
        None => ctx
            .source
            .get()
            .map(|s| s.mime.clone())
            .unwrap_or_else(|| "application/octet-stream".to_string()),
    };
    let (suffix, ext) = match working.op {
        Some(_) => {
            let (s, e) = working.file_stem_suffix();
            (s.to_string(), e.to_string())
        }
        // Downloading an unwatermarked file: keep the original extension.
        None => (
            String::new(),
            ctx.source
                .get()
                .and_then(|s| s.name.rsplit_once('.').map(|(_, ext)| ext.to_string()))
                .unwrap_or_else(|| "png".into()),
        ),
    };
    let stem = ctx
        .source
        .get()
        .and_then(|s| s.name.rsplit_once('.').map(|(stem, _)| stem.to_string()))
        .unwrap_or_else(|| "image".into());
    let filename = format!("{stem}{suffix}.{ext}");

    let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(working.bytes.as_slice()).into());
    let bag = web_sys::BlobPropertyBag::new();
    bag.set_type(&mime);
    let Ok(obj_url) = web_sys::Url::create_object_url_with_blob(
        web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &bag)
            .unwrap()
            .as_ref(),
    ) else {
        ctx.status
            .set(Status::Failed("Could not create the download.".into()));
        return;
    };

    if let Ok(anchor) = document.create_element("a") {
        let anchor: &web_sys::HtmlAnchorElement = anchor.unchecked_ref();
        anchor.set_href(&obj_url);
        anchor.set_download(&filename);
        let body = document.body().unwrap();
        let _ = body.append_child(anchor);
        anchor.click();
        let _ = body.remove_child(anchor);
    }
    let _ = web_sys::Url::revoke_object_url(&obj_url);
}

/// Count a completed watermark toward the upsell nudge.
fn note_success(ctx: &Ctx) {
    let next = ctx.ops_done.get() + 1;
    ctx.ops_done.set(next);
    storage_set(OPS_KEY, &next.to_string());
}

// ------------------------------------------------------------ file ingestion

pub(crate) fn pick_image_file(accept: &str, done: impl FnOnce(String, String, Vec<u8>) + 'static) {
    let window = match web_sys::window() {
        Some(w) => w,
        None => return,
    };
    let document = match window.document() {
        Some(d) => d,
        None => return,
    };
    let input = match document.create_element("input") {
        Ok(el) => el,
        Err(_) => return,
    };
    let _ = input.set_attribute("type", "file");
    let _ = input.set_attribute("accept", accept);
    let input_js: &web_sys::HtmlInputElement = input.unchecked_ref();
    input_js.style().set_property("display", "none").ok();

    let input_el: web_sys::HtmlInputElement = input
        .clone()
        .dyn_into()
        .expect("file input is an input element");
    let on_change: Closure<dyn FnMut()> = Closure::once(move || {
        let Some(file) = input_el.files().and_then(|files| files.item(0)) else {
            return;
        };
        let name = file.name();
        let mime = file.type_();
        let Ok(reader) = web_sys::FileReader::new() else { return };
        let reader_for_cb = reader.clone();
        let on_load: Closure<dyn FnMut()> = Closure::once(move || {
            let bytes = reader_for_cb
                .result()
                .ok()
                .map(|buf| js_sys::Uint8Array::new(&buf).to_vec())
                .unwrap_or_default();
            done(name, mime, bytes);
        });
        reader.set_onload(Some(on_load.as_ref().unchecked_ref()));
        on_load.forget();
        let _ = reader.read_as_array_buffer(&file);
    });
    input
        .add_event_listener_with_callback("change", on_change.as_ref().unchecked_ref())
        .ok();
    on_change.forget();

    let body = document.body().expect("no body");
    let _ = body.append_child(&input);
    input_js.click();
    let _ = body.remove_child(&input);
}

pub(crate) fn on_source_read(ctx: Ctx, name: String, mime: String, bytes: Vec<u8>) {
    if bytes.is_empty() {
        ctx.status
            .set(Status::Failed("That file came through empty — try again.".into()));
        return;
    }
    let format = detect_format(&bytes);
    let decoded = match tpt_proof_engine::decode(&bytes) {
        Ok(img) => img,
        Err(_) => {
            ctx.status.set(Status::Failed(
                "Can't read that image — Proof Watermarker handles JPEG and PNG.".into(),
            ));
            return;
        }
    };
    let (w, h) = (decoded.width, decoded.height);
    ctx.working.set(Some(WorkingImg {
        bytes: bytes.clone(),
        width: w,
        height: h,
        op: None,
    }));
    ctx.source.set(Some(SourceFile {
        name,
        mime: if mime.is_empty() {
            format
                .map(|f| f.mime_type().to_string())
                .unwrap_or_else(|| "application/octet-stream".into())
        } else {
            mime
        },
        bytes,
        width: w,
        height: h,
        format,
    }));
    ctx.status.set(Status::Ready);
    draw_working(&ctx);
    if let Some(window) = web_sys::window() {
        if let Some(document) = window.document() {
            refresh_quality_row(&ctx, &document);
        }
    }
}

// ------------------------------------------------------------- canvas + wiring

fn install_canvas(ctx: &Ctx, document: &web_sys::Document) -> Result<(), JsValue> {
    let host = query(&ctx.container, ".pw-canvas-host")
        .ok_or_else(|| JsValue::from_str("missing .pw-canvas-host"))?;
    let canvas = document.create_element("canvas")?;
    canvas.set_class_name("pw-canvas");
    host.append_child(&canvas)?;
    Ok(())
}

fn draw_working(ctx: &Ctx) {
    let Some(working) = ctx.working.get() else {
        return;
    };
    let Some(canvas_el) = query(&ctx.container, ".pw-canvas") else { return };
    let Ok(canvas) = canvas_el.dyn_into::<web_sys::HtmlCanvasElement>() else { return };
    let img = match tpt_proof_engine::decode(&working.bytes) {
        Ok(img) => img,
        Err(_) => return,
    };
    let (w, h) = (working.width, working.height);
    canvas.set_width(w);
    canvas.set_height(h);
    let Ok(Some(ctx2d)) = canvas.get_context("2d") else { return };
    let Ok(ctx2d) = ctx2d.dyn_into::<web_sys::CanvasRenderingContext2d>() else { return };
    let data = js_sys::Uint8ClampedArray::from(img.data.as_slice());
    if let Ok(image_data) = web_sys::ImageData::new_with_js_u8_clamped_array_and_sh(&data, w, h) {
        let _ = ctx2d.put_image_data(&image_data, 0.0, 0.0);
    }
}

fn wire_upload_button(ctx: &Ctx) -> Result<(), JsValue> {
    let Some(drop) = query(&ctx.container, ".pw-drop") else { return Ok(()) };
    let ctx = ctx.clone();
    let ctx2 = ctx.clone();
    let on_click: Closure<dyn FnMut()> = Closure::new(move || {
        let ctx = ctx.clone();
        pick_image_file("image/jpeg,image/png", move |name, mime, bytes| {
            on_source_read(ctx, name, mime, bytes);
        });
    });
    drop
        .add_event_listener_with_callback("click", on_click.as_ref().unchecked_ref())?;
    on_click.forget();

    // Keyboard activation for the role=button dropzone.
    let on_key: Closure<dyn FnMut(web_sys::Event)> = Closure::new(move |e: web_sys::Event| {
        let key = e.unchecked_ref::<web_sys::KeyboardEvent>().key();
        if key == "Enter" || key == " " {
            e.prevent_default();
            let ctx = ctx2.clone();
            pick_image_file("image/jpeg,image/png", move |name, mime, bytes| {
                on_source_read(ctx, name, mime, bytes);
            });
        }
    });
    drop.add_event_listener_with_callback("keydown", on_key.as_ref().unchecked_ref())?;
    on_key.forget();
    Ok(())
}

fn wire_dropzone(ctx: &Ctx) -> Result<(), JsValue> {
    let Some(drop) = query(&ctx.container, ".pw-drop") else { return Ok(()) };
    let on_over: Closure<dyn FnMut(web_sys::Event)> = Closure::new(move |e: web_sys::Event| {
        e.prevent_default();
        let Ok(de) = e.dyn_into::<web_sys::DragEvent>() else { return };
        if let Some(dt) = de.data_transfer() {
            dt.set_drop_effect("copy");
        }
    });
    drop.add_event_listener_with_callback("dragover", on_over.as_ref().unchecked_ref())?;
    on_over.forget();

    let ctx = ctx.clone();
    let on_drop: Closure<dyn FnMut(web_sys::Event)> = Closure::new(move |e: web_sys::Event| {
        e.prevent_default();
        let Ok(de) = e.dyn_into::<web_sys::DragEvent>() else { return };
        if let Some(file) = de.data_transfer().and_then(|dt| dt.files()).and_then(|f| f.item(0)) {
            let name = file.name();
            let mime = file.type_();
            let ctx = ctx.clone();
            pick_from_file(&ctx, name, mime, &file);
        }
    });
    drop.add_event_listener_with_callback("drop", on_drop.as_ref().unchecked_ref())?;
    on_drop.forget();
    Ok(())
}

/// Read an already-in-hand `web_sys::File` (drop path) through a FileReader.
fn pick_from_file(ctx: &Ctx, name: String, mime: String, file: &web_sys::File) {
    let Ok(reader) = web_sys::FileReader::new() else { return };
    let ctx = std::rc::Rc::new(ctx.clone());
    let reader_for_cb = reader.clone();
    let ctx_for_cb = ctx.clone();
    let on_load: Closure<dyn FnMut()> = Closure::once(move || {
        let bytes = reader_for_cb
            .result()
            .ok()
            .map(|buf| js_sys::Uint8Array::new(&buf).to_vec())
            .unwrap_or_default();
        on_source_read((*ctx_for_cb).clone(), name, mime, bytes);
    });
    reader.set_onload(Some(on_load.as_ref().unchecked_ref()));
    on_load.forget();
    if let Err(err) = reader.read_as_array_buffer(file) {
        ctx.status
            .set(Status::Failed(format!("Could not read file: {err:?}")));
    }
}

/// The logo picker row: pick a PNG/JPEG to use as the overlay.
fn wire_logo_button(ctx: &Ctx) -> Result<(), JsValue> {
    let Some(btn) = query(&ctx.container, ".pw-logo-btn") else { return Ok(()) };
    let ctx = ctx.clone();
    let on_click: Closure<dyn FnMut()> = Closure::new(move || {
        let ctx = ctx.clone();
        let name_label = query(&ctx.container, ".pw-logo-name");
        pick_image_file("image/png,image/jpeg", move |name, _mime, bytes| {
            if bytes.is_empty() {
                ctx.status
                    .set(Status::Failed("That logo file came through empty.".into()));
                return;
            }
            if detect_format(&bytes).is_none() {
                ctx.status
                    .set(Status::Failed("Logos must be PNG or JPEG images.".into()));
                return;
            }
            if let Some(label) = name_label.as_ref() {
                label.set_text_content(Some(&name));
            }
            ctx.logo.set(Some(LogoFile { name: name.clone(), bytes }));
            ctx.status.set(Status::Done(format!("Logo loaded: {name}")));
        });
    });
    btn.add_event_listener_with_callback("click", on_click.as_ref().unchecked_ref())?;
    on_click.forget();
    Ok(())
}

/// Slider value labels update live, without dispatching through the tree.
fn wire_live_sliders(document: &web_sys::Document) {
    for (slider_sel, label_sel, suffix) in [
        (".pw-size", ".pw-s-val", "%"),
        (".pw-opacity", ".pw-o-val", "%"),
        (".pw-quality", ".pw-q-val", ""),
    ] {
        let Some(slider) = query_document(document, slider_sel) else { continue };
        let Some(label) = query_document(document, label_sel) else { continue };
        let Ok(slider) = slider.dyn_into::<web_sys::HtmlInputElement>() else { continue };
        let slider_for_cb = slider.clone();
        let on_input: Closure<dyn FnMut()> = Closure::new(move || {
            label.set_text_content(Some(&format!("{}{suffix}", slider_for_cb.value())));
        });
        if slider
            .add_event_listener_with_callback("input", on_input.as_ref().unchecked_ref())
            .is_err()
        {
            continue;
        }
        on_input.forget();
    }
}

/// The quality row only applies when the effective output is JPEG.
fn refresh_quality_row(ctx: &Ctx, document: &web_sys::Document) {
    let val = read_select(&ctx.container, ".pw-format").unwrap_or_else(|| "keep".into());
    let src = ctx.source.get().and_then(|s| s.format);
    let jpeg = matches!(resolve_format(&val, src), ImageFormat::Jpeg);
    if let Some(row) = query_document(document, ".pw-quality-row") {
        if let Some(el) = row.dyn_ref::<web_sys::HtmlElement>() {
            let _ = el.style().set_property("display", if jpeg { "" } else { "none" });
        }
    }
}

/// Text controls apply in text mode; the logo row in logo mode.
fn refresh_type_rows(ctx: &Ctx, document: &web_sys::Document) {
    let logo_mode = read_select(&ctx.container, ".pw-type").as_deref() == Some("logo");
    for sel in [".pw-text-row", ".pw-color-row"] {
        if let Some(row) = query_document(document, sel) {
            if let Some(el) = row.dyn_ref::<web_sys::HtmlElement>() {
                let _ = el.style().set_property("display", if logo_mode { "none" } else { "" });
            }
        }
    }
    if let Some(row) = query_document(document, ".pw-logo-row") {
        if let Some(el) = row.dyn_ref::<web_sys::HtmlElement>() {
            let _ = el.style().set_property("display", if logo_mode { "" } else { "none" });
        }
    }
    // Position is fixed to the grid when tiling; still selectable either way.
    let _ = ctx;
}

fn wire_format_select(ctx: &Ctx, document: &web_sys::Document) -> Result<(), JsValue> {
    let refresh = {
        let ctx = ctx.clone();
        let document = document.clone();
        move || refresh_quality_row(&ctx, &document)
    };
    refresh();

    let Some(select) = query(&ctx.container, ".pw-format") else { return Ok(()) };
    let on_change: Closure<dyn FnMut()> = {
        let refresh = refresh.clone();
        Closure::new(move || refresh())
    };
    select.add_event_listener_with_callback("change", on_change.as_ref().unchecked_ref())?;
    on_change.forget();
    Ok(())
}

fn wire_type_select(ctx: &Ctx, document: &web_sys::Document) -> Result<(), JsValue> {
    refresh_type_rows(ctx, document);
    let Some(select) = query(&ctx.container, ".pw-type") else { return Ok(()) };
    let on_change: Closure<dyn FnMut()> = {
        let ctx = ctx.clone();
        let document = document.clone();
        Closure::new(move || refresh_type_rows(&ctx, &document))
    };
    select.add_event_listener_with_callback("change", on_change.as_ref().unchecked_ref())?;
    on_change.forget();
    Ok(())
}

/// Re-evaluate the conditional rows after a preset overwrote the controls.
pub(crate) fn refresh_rows_after_preset(ctx: &Ctx, document: &web_sys::Document) {
    refresh_quality_row(ctx, document);
    refresh_type_rows(ctx, document);
}

// ------------------------------------------------------------------ helpers

pub(crate) fn query(root: &web_sys::Element, selector: &str) -> Option<web_sys::Element> {
    root.query_selector(selector).ok().flatten()
}

pub(crate) fn query_document(document: &web_sys::Document, selector: &str) -> Option<web_sys::Element> {
    document.query_selector(selector).ok().flatten()
}

pub(crate) fn read_select(root: &web_sys::Element, selector: &str) -> Option<String> {
    let el = query(root, selector)?;
    el.dyn_into::<web_sys::HtmlSelectElement>()
        .ok()
        .map(|s| s.value())
}

/// Numeric value of the input at `selector` (0.0 when missing/unparsable).
pub(crate) fn read_number(root: &web_sys::Element, selector: &str) -> f64 {
    query(root, selector)
        .and_then(|el| el.dyn_into::<web_sys::HtmlInputElement>().ok())
        .and_then(|input| input.value().parse().ok())
        .unwrap_or(0.0)
}

pub(crate) fn read_checked(root: &web_sys::Element, selector: &str) -> bool {
    query(root, selector)
        .and_then(|el| el.dyn_into::<web_sys::HtmlInputElement>().ok())
        .map(|input| input.checked())
        .unwrap_or(false)
}

/// Set the value of the input at `selector` (preset loading).
pub(crate) fn write_input(root: &web_sys::Element, selector: &str, value: &str) {
    if let Some(el) = query(root, selector).and_then(|el| el.dyn_into::<web_sys::HtmlInputElement>().ok()) {
        el.set_value(value);
    }
}

pub(crate) fn write_select(root: &web_sys::Element, selector: &str, value: &str) {
    if let Some(el) = query(root, selector).and_then(|el| el.dyn_into::<web_sys::HtmlSelectElement>().ok()) {
        el.set_value(value);
    }
}

pub(crate) fn write_checked(root: &web_sys::Element, selector: &str, checked: bool) {
    if let Some(el) = query(root, selector).and_then(|el| el.dyn_into::<web_sys::HtmlInputElement>().ok()) {
        el.set_checked(checked);
    }
}

/// Run `f` on the next tick so a Busy state painted first.
pub(crate) fn defer(window: &web_sys::Window, f: impl FnOnce() + 'static) {
    let cb = Closure::once(f);
    let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
        cb.as_ref().unchecked_ref(),
        30,
    );
    cb.forget();
}

pub(crate) fn upsert_style(document: &web_sys::Document, id: &str, css: &str) {
    if let Ok(Some(existing)) = document.query_selector(&format!("style#{id}")) {
        if let Some(el) = existing.dyn_ref::<web_sys::HtmlElement>() {
            el.set_text_content(Some(css));
        }
        return;
    }
    if let Ok(el) = document.create_element("style") {
        el.set_id(id);
        if let Some(head) = document.head() {
            let _ = head.append_child(&el);
        }
        if let Some(el) = el.dyn_ref::<web_sys::HtmlElement>() {
            el.set_text_content(Some(css));
        }
    }
}

pub(crate) fn storage_get(key: &str) -> Option<String> {
    let window = web_sys::window()?;
    let storage = window.local_storage().ok().flatten()?;
    storage.get_item(key).ok().flatten()
}

pub(crate) fn storage_set(key: &str, value: &str) {
    if let Some(window) = web_sys::window() {
        if let Ok(Some(storage)) = window.local_storage() {
            let _ = storage.set_item(key, value);
        }
    }
}

pub(crate) fn format_size(bytes: usize) -> String {
    let b = bytes as f64;
    if b >= 1024.0 * 1024.0 {
        format!("{:.1} MB", b / (1024.0 * 1024.0))
    } else if b >= 1024.0 {
        format!("{:.0} KB", b / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

pub(crate) fn file_stem(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(stem, _)| stem.to_string())
        .unwrap_or_else(|| name.to_string())
}

// ------------------------------------------------------------------- styles

const BASE_CSS: &str = r#"
.pw-root {
  --pw-bg: transparent;
  --pw-card: #ffffff;
  --pw-border: #d9d9de;
  --pw-border-strong: #b9b9c2;
  --pw-text: #1c1c22;
  --pw-muted: #6b6b76;
  --pw-accent: #0b6bcb;
  --pw-accent-soft: #e8f1fc;
  --pw-danger: #b3261e;
  --pw-checker-a: #e8e8ec;
  --pw-checker-b: #f8f8fa;
  font-family: system-ui, -apple-system, "Segoe UI", Roboto, sans-serif;
  color: var(--pw-text);
  max-width: 1060px;
  margin: 0 auto;
  padding: 4px 4px 40px;
  line-height: 1.45;
}
@media (prefers-color-scheme: dark) {
  .pw-root {
    --pw-card: #23232a;
    --pw-border: #3a3a44;
    --pw-border-strong: #55555f;
    --pw-text: #ececf1;
    --pw-muted: #9d9da8;
    --pw-accent: #5aa2f0;
    --pw-accent-soft: #1d3250;
    --pw-checker-a: #2c2c34;
    --pw-checker-b: #3a3a44;
  }
}
.pw-header h1 { font-size: 1.45rem; margin: 0.4em 0 0.15em; }
.pw-tagline { color: var(--pw-muted); margin: 0 0 16px; }
.pw-main { display: flex; gap: 16px; flex-wrap: wrap; align-items: flex-start; }
.pw-controls { flex: 1 1 340px; display: flex; flex-direction: column; gap: 12px; min-width: 300px; }
.pw-preview { flex: 1 1 400px; display: flex; flex-direction: column; gap: 10px; min-width: 300px; }
.pw-card {
  border: 1px solid var(--pw-border);
  border-radius: 10px;
  padding: 12px 14px;
  background: var(--pw-card);
}
.pw-card-title { font-weight: 600; margin-bottom: 4px; }
.pw-card-pro { border-color: var(--pw-accent); }
.pw-drop {
  border: 2px dashed var(--pw-border-strong);
  border-radius: 10px;
  text-align: center;
  padding: 24px 12px;
  cursor: pointer;
  user-select: none;
  transition: border-color 120ms ease, background 120ms ease;
}
.pw-drop:hover, .pw-drop:focus-visible { border-color: var(--pw-accent); background: var(--pw-accent-soft); outline: none; }
.pw-drop-main { font-weight: 600; }
.pw-drop-hint { color: var(--pw-muted); font-size: 0.88em; margin-top: 4px; }
.pw-row { display: flex; align-items: center; gap: 10px; margin: 8px 0; }
.pw-lbl { flex: 0 0 110px; color: var(--pw-muted); }
.pw-row input[type="range"] { flex: 1 1 auto; accent-color: var(--pw-accent); }
.pw-row select { flex: 1 1 auto; padding: 4px 6px; border-radius: 6px; border: 1px solid var(--pw-border); background: var(--pw-card); color: inherit; }
.pw-row input[type="text"] { flex: 1 1 auto; padding: 5px 8px; border-radius: 6px; border: 1px solid var(--pw-border); background: var(--pw-card); color: inherit; font: inherit; }
.pw-s-val, .pw-o-val, .pw-q-val { flex: 0 0 4.2ch; text-align: right; font-variant-numeric: tabular-nums; }
.pw-logo-name { color: var(--pw-muted); font-size: 0.85em; overflow-wrap: anywhere; }
.pw-btn {
  border: 1px solid var(--pw-border-strong);
  border-radius: 8px;
  padding: 7px 14px;
  background: var(--pw-card);
  color: inherit;
  cursor: pointer;
  font: inherit;
}
.pw-btn:hover { border-color: var(--pw-accent); }
.pw-btn-primary { background: var(--pw-accent); border-color: var(--pw-accent); color: #fff; }
.pw-btn-primary:hover { filter: brightness(1.08); }
.pw-btn-danger { color: var(--pw-danger); border-color: var(--pw-danger); }
.pw-actions { gap: 8px; flex-wrap: wrap; }
.pw-canvas-host {
  border: 1px solid var(--pw-border);
  border-radius: 10px;
  min-height: 240px;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 8px;
  background: var(--pw-card);
}
.pw-canvas {
  max-width: 100%;
  max-height: 540px;
  width: auto;
  height: auto;
  display: block;
  background: repeating-conic-gradient(var(--pw-checker-a) 0% 25%, var(--pw-checker-b) 0% 50%) 0 0 / 20px 20px;
}
.pw-status { min-height: 1em; }
.pw-status-empty { color: var(--pw-muted); padding: 6px 2px; }
.pw-fileinfo { color: var(--pw-muted); font-size: 0.9em; overflow-wrap: anywhere; }
.pw-summary { font-weight: 600; margin: 4px 0; }
.pw-busy { color: var(--pw-accent); margin: 4px 0; }
.pw-error { color: var(--pw-danger); margin: 4px 0; }
.pw-hint { color: var(--pw-muted); font-size: 0.85em; }
.pw-upsell {
  border: 1px dashed var(--pw-accent);
  border-radius: 10px;
  padding: 12px 14px;
  background: var(--pw-accent-soft);
}
.pw-upsell h3 { margin: 0 0 6px; font-size: 1rem; }
.pw-upsell-body { color: var(--pw-text); font-size: 0.92em; }
.pw-upsell-link { display: inline-block; margin-top: 8px; color: var(--pw-accent); font-weight: 600; }
.pw-note { color: var(--pw-muted); font-size: 0.85em; }
.pw-preset-row { display: flex; align-items: center; gap: 6px; margin: 4px 0; }
.pw-preset-row select { flex: 1 1 auto; min-width: 0; padding: 4px 6px; border-radius: 6px; border: 1px solid var(--pw-border); background: var(--pw-card); color: inherit; }
.pw-batch-note { margin: 4px 0; }
.pw-progress {
  height: 8px;
  border-radius: 4px;
  background: var(--pw-accent-soft);
  margin: 6px 0;
  position: relative;
  overflow: hidden;
}
.pw-progress::after {
  content: "";
  position: absolute;
  inset: 0;
  width: calc(var(--pct, 0) * 1%);
  background: var(--pw-accent);
  border-radius: 4px;
}
"#;
