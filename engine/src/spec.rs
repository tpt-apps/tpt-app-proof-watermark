//! The watermark spec: everything the UI (and a saved preset) needs to
//! describe one watermark, as plain data.

/// RGB color, 0–255 per channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color(pub [u8; 3]);

impl Color {
    pub const WHITE: Color = Color([255, 255, 255]);
    pub const BLACK: Color = Color([0, 0, 0]);
}

/// Where a single (non-tiled) watermark sits on the image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Position {
    TopLeft,
    TopRight,
    Center,
    BottomLeft,
    BottomRight,
}

/// What gets drawn. Text is rasterized with the engine's bitmap font; logo
/// bytes are supplied separately to [`crate::apply_watermark`] so presets can
/// stay pure data.
#[derive(Debug, Clone, PartialEq)]
pub enum WatermarkContent {
    Text { text: String, color: Color },
    Logo,
}

/// Bounds for the scale fraction (of the image's smaller dimension) applied
/// in [`crate::apply_watermark`]; the UI sliders stay inside these too.
pub const MIN_SCALE: f32 = 0.01;
pub const MAX_SCALE: f32 = 0.6;

/// A complete watermark description.
#[derive(Debug, Clone, PartialEq)]
pub struct WatermarkSpec {
    pub content: WatermarkContent,
    pub position: Position,
    /// Text height / logo width as a fraction of the image's smaller
    /// dimension (clamped to `MIN_SCALE`..`MAX_SCALE`).
    pub scale: f32,
    /// 0.0–1.0, clamped.
    pub opacity: f32,
    /// Repeat the watermark in a grid over the whole image instead of one
    /// placement at `position`.
    pub tile: bool,
    /// Gap between tiles as a fraction of the smaller dimension (clamped to
    /// 0.0–1.0; only used when `tile` is set).
    pub tile_gap: f32,
}

impl Default for WatermarkSpec {
    fn default() -> Self {
        WatermarkSpec {
            content: WatermarkContent::Text {
                text: "PROOF".to_string(),
                color: Color::WHITE,
            },
            position: Position::BottomRight,
            scale: 0.04,
            opacity: 0.4,
            tile: false,
            tile_gap: 0.25,
        }
    }
}
