//! TPT Proof Watermarker engine.
//!
//! Adapter over `tpt-wasmforge`'s media crates: image bytes + a
//! [`spec::WatermarkSpec`] in, watermarked image bytes out; plus lossless
//! metadata (EXIF/IPTC/XMP/comment) stripping. Plain functions, no UI deps —
//! unit-testable on the host and compiled as-is into the wasm32 app.

pub mod exif;
pub mod render;
pub mod spec;

pub use spec::{Color, Position, WatermarkContent, WatermarkSpec};

pub use tpt_media_core::{ForgeError, ForgeResult, ImageBuffer, ImageFormat};

/// Decode image bytes into an RGBA [`ImageBuffer`] (PNG or JPEG,
/// auto-detected by magic bytes). Re-exported from tpt-wasmforge.
pub fn decode(bytes: &[u8]) -> ForgeResult<ImageBuffer> {
    tpt_media_image::decode(bytes)
}

/// Encode an [`ImageBuffer`] to PNG (`quality` ignored) or JPEG bytes.
/// Re-exported from tpt-wasmforge.
pub fn encode(buf: &ImageBuffer, format: ImageFormat, quality: u8) -> ForgeResult<Vec<u8>> {
    tpt_media_image::encode(buf, format, quality)
}

/// Format sniffing shared by the UI (unknown formats are rejected up front
/// rather than failing mid-watermark).
pub fn detect_format(bytes: &[u8]) -> Option<ImageFormat> {
    if bytes.len() >= 8 && bytes[0..8] == *b"\x89PNG\r\n\x1a\n" {
        Some(ImageFormat::Png)
    } else if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xD8 {
        Some(ImageFormat::Jpeg)
    } else {
        None
    }
}

/// Apply a [`WatermarkSpec`] to an image and re-encode it.
///
/// `logo_bytes` is required when `spec.content` is
/// [`WatermarkContent::Logo`] and must decode as PNG/JPEG (PNG recommended —
/// only its alpha channel survives compositing gracefully).
///
/// Note for callers: the image is decoded and re-encoded, so the output never
/// carries the input's EXIF. The explicit [`strip_metadata`] path exists for
/// stripping *without* watermarking, losslessly, and as the batch toggle.
pub fn apply_watermark(
    input: &[u8],
    spec: &WatermarkSpec,
    logo_bytes: Option<&[u8]>,
    format: ImageFormat,
    jpeg_quality: u8,
) -> ForgeResult<Vec<u8>> {
    let img = decode(input)?;
    let min_dim = img.width.min(img.height);
    let max_width = (img.width as f32 * 0.95) as u32;
    let overlay = match &spec.content {
        WatermarkContent::Text { text, color } => {
            let height_px = spec
                .scale
                .clamp(spec::MIN_SCALE, spec::MAX_SCALE)
                .mul_add(min_dim as f32, 0.5)
                .round() as u32;
            let overlay = render::text_overlay(text, height_px.max(8), color.0);
            // Long studio names on portrait images would otherwise clip at
            // the left edge — shrink to fit the width, never upscale.
            if overlay.width > max_width && overlay.width > 0 {
                let fitted = ((height_px as f64 * max_width as f64 / overlay.width as f64)
                    .round() as u32)
                    .max(8);
                render::text_overlay(text, fitted.min(height_px.max(8)), color.0)
            } else {
                overlay
            }
        }
        WatermarkContent::Logo => {
            let bytes =
                logo_bytes.ok_or_else(|| ForgeError::InvalidInput("logo watermark needs logo bytes".into()))?;
            render::logo_overlay(&decode(bytes)?, spec.scale.clamp(spec::MIN_SCALE, spec::MAX_SCALE), min_dim)?
        }
    };
    let mut out = img;
    if spec.tile {
        render::composite_tiled(&mut out, &overlay, spec.opacity, spec.tile_gap);
    } else {
        render::composite_positioned(&mut out, &overlay, spec.position, spec.opacity);
    }
    encode(&out, format, jpeg_quality)
}

/// Strip EXIF/IPTC/XMP/comment metadata without re-encoding pixel data
/// (JPEG: APP1/APP13/COM segment rewrite; PNG: eXIf/text chunk rewrite).
/// Free-tier UI never calls this — it is the Pro edition's stripping path.
pub fn strip_metadata(input: &[u8]) -> ForgeResult<Vec<u8>> {
    match detect_format(input) {
        Some(ImageFormat::Jpeg) => exif::strip_jpeg_metadata(input),
        Some(ImageFormat::Png) => exif::strip_png_metadata(input),
        None => Err(ForgeError::UnsupportedFormat(
            "unrecognised image magic bytes — only PNG and JPEG are supported".into(),
        )),
    }
}
