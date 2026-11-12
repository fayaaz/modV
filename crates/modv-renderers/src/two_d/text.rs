//! Text shaping and glyph rasterisation for the canvas facade.
//!
//! Port of `canvas-text-wrapper`'s behaviour as used by `Text.js`
//! (`src/renderer/src/application/renderers/2d/modules/Text.js:176-183`):
//! word wrap at a maximum width, `textAlign: center`, `verticalAlign: middle`,
//! `lineHeight: 1`, and optional stroked text. Only that subset is ported —
//! the wrapper's `sizeToFill`, justify, underline and HDPI paths are unused
//! by the corpus.
//!
//! # Why cosmic-text
//!
//! `docs/rust-hybrid.md` WS-5 recommends `cosmic-text` + `fontdb`: shape on
//! the CPU, upload glyph coverage as an `R8` texture, and draw it with the
//! same textured-quad pipeline as `drawImage`.
//!
//! # The mask
//!
//! [`TextEngine::rasterise`] lays the whole text block out once, then
//! rasterises every glyph with [`cosmic_text::SwashCache`] (which wraps
//! swash's scaler) into a single straight-alpha `R8` bitmap covering the
//! block's bounding box. `Canvas::fill_text` uploads that bitmap as one
//! texture and draws **one quad**, which keeps the facade's command list
//! small and makes text reuse the path pipeline's transform/clip/alpha
//! handling.
//!
//! Glyph bitmaps are obtained twice (measure, then blit); the second call is
//! served from `SwashCache`'s image cache, so the cost is one rasterisation
//! per glyph.
//!
//! # Baseline
//!
//! `Text.js` relies on the wrapper's `textBaseline = "bottom"` setting. The
//! facade treats every baseline mode as an offset from the alphabetic
//! baseline (bottom: `y`, middle: `y + 0.35 em`, top: `y + 0.8 em`), which
//! differs from the browser's "bottom of the em box" by the font descent
//! (≲ 0.2 em). Documented as a deliberate deviation — the wrapper's
//! vertical centring stays within a few pixels at the sizes the corpus uses.

use cosmic_text::{
    Attrs, Buffer, Family, FontSystem, Metrics, Shaping, Style, SwashCache, SwashContent, Weight,
    Wrap,
};
/// Canvas `textAlign` values used by the corpus (`Text.js` sets `center`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextAlign {
    /// Left edge at the anchor x (canvas default).
    #[default]
    Left,
    /// Centred on the anchor x.
    Center,
    /// Right edge at the anchor x.
    Right,
}

/// Canvas `textBaseline` values. Only `Bottom` (the wrapper's setting) and
/// the default `Alphabetic` are exercised by the corpus.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextBaseline {
    /// Alphabetic baseline at the anchor y (canvas default).
    #[default]
    Alphabetic,
    /// Em-box top at the anchor y (approximated as `0.8 em` above the
    /// alphabetic baseline).
    Top,
    /// Em-box middle at the anchor y (approximated as `0.35 em` above the
    /// alphabetic baseline).
    Middle,
    /// Em-box bottom at the anchor y (treated as the alphabetic baseline —
    /// see the module docs).
    Bottom,
}

/// The canvas' current text state (`context.font` +
/// `context.textAlign`/`textBaseline` in JS).
#[derive(Clone, Debug, PartialEq)]
pub struct TextStyle {
    /// Font size in CSS pixels (`Text.js` `size` prop).
    pub size: f32,
    /// Family name; the canvas default is `sans-serif`.
    pub family: String,
    /// Bold (`Text.js` `weight` prop is `"bold"` by default).
    pub bold: bool,
    /// Horizontal alignment.
    pub align: TextAlign,
    /// Vertical baseline mode.
    pub baseline: TextBaseline,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            // Canvas 2D's default font is `10px sans-serif`.
            size: 10.0,
            family: "sans-serif".to_owned(),
            bold: false,
            align: TextAlign::Left,
            baseline: TextBaseline::Alphabetic,
        }
    }
}

/// A rasterised text block: straight-alpha coverage plus its top-left origin
/// in canvas (user-space) pixels.
#[derive(Clone, Debug)]
pub struct RasterisedText {
    /// Mask width in pixels.
    pub width: u32,
    /// Mask height in pixels.
    pub height: u32,
    /// `width * height` coverage bytes, row-major.
    pub pixels: Vec<u8>,
    /// Top-left of the mask in canvas user space (fractional).
    pub origin: [f32; 2],
}

/// cosmic-text font system + swash glyph cache.
///
/// Created lazily on the first text operation (system font discovery is the
/// expensive part) and owned by the canvas.
pub struct TextEngine {
    font_system: FontSystem,
    swash: SwashCache,
    /// Set when font discovery finds no font at all (see
    /// [`Canvas::text_unavailable`]).
    unavailable: bool,
}

impl std::fmt::Debug for TextEngine {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TextEngine")
            .field("unavailable", &self.unavailable)
            .finish()
    }
}

impl TextEngine {
    /// Create an engine, or `None` when fontdb finds no system font.
    ///
    /// Uses cosmic-text's default `FontSystem`, which loads system fonts via
    /// fontdb (the plan's "bundled system font discovery").
    pub fn new() -> Option<Self> {
        let font_system = FontSystem::new();
        if font_system.db().faces().count() == 0 {
            return None;
        }
        Some(Self {
            font_system,
            swash: SwashCache::new(),
            unavailable: false,
        })
    }

    /// Shape `text` into a laid-out buffer.
    fn shape(&mut self, text: &str, max_width: Option<f32>, style: &TextStyle) -> Buffer {
        let size = style.size.max(1.0);
        // The wrapper's default `lineHeight: 1` → the line box is one em.
        let metrics = Metrics::new(size, size);
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        buffer.set_size(&mut self.font_system, max_width, None);
        buffer.set_wrap(
            &mut self.font_system,
            if max_width.is_some() {
                Wrap::WordOrGlyph
            } else {
                Wrap::None
            },
        );

        let family = family_for(&style.family);
        let attrs = Attrs::new()
            .family(family)
            .weight(if style.bold {
                Weight::BOLD
            } else {
                Weight::NORMAL
            })
            .style(Style::Normal);
        buffer.set_text(&mut self.font_system, text, &attrs, Shaping::Advanced);
        buffer.shape_until_scroll(&mut self.font_system, false);
        buffer
    }

    /// Height of the laid-out text block: `lines × font size`, matching the
    /// wrapper's `textBlockHeight` (`lineHeight = fontSize * 1`).
    pub fn block_height(&mut self, text: &str, max_width: Option<f32>, style: &TextStyle) -> f32 {
        if text.trim().is_empty() {
            return 0.0;
        }
        let buffer = self.shape(text, max_width, style);
        buffer.layout_runs().count() as f32 * style.size.max(1.0)
    }

    /// Rasterise `text` into a straight-alpha coverage mask.
    ///
    /// Returns `None` for empty text, when no glyph produced a bitmap, or when
    /// the block would exceed the 4096 px texture limit.
    pub fn rasterise(
        &mut self,
        text: &str,
        max_width: Option<f32>,
        style: &TextStyle,
        x: f32,
        y: f32,
    ) -> Option<RasterisedText> {
        if text.trim().is_empty() {
            return None;
        }
        let buffer = self.shape(text, max_width, style);
        let size = style.size.max(1.0);

        let baseline_y = match style.baseline {
            TextBaseline::Alphabetic | TextBaseline::Bottom => y,
            TextBaseline::Top => y + size * 0.8,
            TextBaseline::Middle => y + size * 0.35,
        };

        let first_line_y = buffer.layout_runs().next().map_or(0.0, |run| run.line_y);

        // Pass 1: bounding box of every glyph bitmap.
        let mut min_x = f32::INFINITY;
        let mut min_y = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut max_y = f32::NEG_INFINITY;
        let mut any_glyph = false;

        for run in buffer.layout_runs() {
            let line_delta = run.line_y - first_line_y;
            let line_anchor = match style.align {
                TextAlign::Left => x,
                TextAlign::Center => x - run.line_w / 2.0,
                TextAlign::Right => x - run.line_w,
            };
            for glyph in run.glyphs.iter() {
                let physical = glyph.physical((0.0, 0.0), 1.0);
                let Some(image) = self
                    .swash
                    .get_image(&mut self.font_system, physical.cache_key)
                    .as_ref()
                else {
                    continue;
                };
                let placement = image.placement;
                if placement.width == 0 || placement.height == 0 {
                    continue;
                }
                let glyph_x = line_anchor + physical.x as f32 + placement.left as f32;
                let glyph_y = baseline_y + line_delta + physical.y as f32 - placement.top as f32;
                min_x = min_x.min(glyph_x);
                min_y = min_y.min(glyph_y);
                max_x = max_x.max(glyph_x + placement.width as f32);
                max_y = max_y.max(glyph_y + placement.height as f32);
                any_glyph = true;
            }
        }
        if !any_glyph {
            return None;
        }

        let origin = [min_x.floor(), min_y.floor()];
        let width = (max_x.ceil() - origin[0]).max(1.0);
        let height = (max_y.ceil() - origin[1]).max(1.0);
        if width > 4096.0 || height > 4096.0 {
            return None;
        }
        let width = width as u32;
        let height = height as u32;
        let mut pixels = vec![0u8; (width * height) as usize];

        // Pass 2: blit glyph coverage into the block mask (max-combine keeps
        // overlapping glyphs from erasing each other).
        for run in buffer.layout_runs() {
            let line_delta = run.line_y - first_line_y;
            let line_anchor = match style.align {
                TextAlign::Left => x,
                TextAlign::Center => x - run.line_w / 2.0,
                TextAlign::Right => x - run.line_w,
            };
            for glyph in run.glyphs.iter() {
                let physical = glyph.physical((0.0, 0.0), 1.0);
                let Some(image) = self
                    .swash
                    .get_image(&mut self.font_system, physical.cache_key)
                    .as_ref()
                else {
                    continue;
                };
                let placement = image.placement;
                if placement.width == 0 || placement.height == 0 {
                    continue;
                }
                let glyph_x = line_anchor + physical.x as f32 + placement.left as f32;
                let glyph_y = baseline_y + line_delta + physical.y as f32 - placement.top as f32;
                let offset_x = (glyph_x - origin[0]).round() as i32;
                let offset_y = (glyph_y - origin[1]).round() as i32;
                blit_glyph(
                    &mut pixels,
                    width,
                    height,
                    offset_x,
                    offset_y,
                    &image.data,
                    placement.width as i32,
                    placement.height as i32,
                    image.content,
                );
            }
        }

        Some(RasterisedText {
            width,
            height,
            pixels,
            origin,
        })
    }
}

/// Map the CSS family string from `Text.js`'s `font` prop to a cosmic family.
fn family_for(family: &str) -> Family<'_> {
    match family.trim().to_ascii_lowercase().as_str() {
        "sans-serif" | "sans" | "" => Family::SansSerif,
        "serif" => Family::Serif,
        "monospace" | "mono" => Family::Monospace,
        "cursive" => Family::Cursive,
        "fantasy" => Family::Fantasy,
        _ => Family::Name(family),
    }
}

/// Copy one glyph's coverage into the block mask.
#[allow(clippy::too_many_arguments)]
fn blit_glyph(
    pixels: &mut [u8],
    mask_width: u32,
    mask_height: u32,
    offset_x: i32,
    offset_y: i32,
    data: &[u8],
    glyph_width: i32,
    glyph_height: i32,
    content: SwashContent,
) {
    let channels = match content {
        SwashContent::Mask => 1,
        SwashContent::Color => 4,
        SwashContent::SubpixelMask => 3,
    };
    for row in 0..glyph_height {
        let y = offset_y + row;
        if y < 0 || y >= mask_height as i32 {
            continue;
        }
        for column in 0..glyph_width {
            let x = offset_x + column;
            if x < 0 || x >= mask_width as i32 {
                continue;
            }
            let source = ((row * glyph_width + column) * channels) as usize;
            if source >= data.len() {
                continue;
            }
            let alpha = match content {
                SwashContent::Mask => data[source],
                SwashContent::Color => data.get(source + 3).copied().unwrap_or(0),
                SwashContent::SubpixelMask => {
                    let sum = data[source] as u32
                        + data.get(source + 1).copied().unwrap_or(0) as u32
                        + data.get(source + 2).copied().unwrap_or(0) as u32;
                    (sum / 3) as u8
                }
            };
            let target = (y as u32 * mask_width + x as u32) as usize;
            if let Some(slot) = pixels.get_mut(target) {
                *slot = (*slot).max(alpha);
            }
        }
    }
}
