//! `Text.js` — text rendering via `canvas-text-wrapper`.
//!
//! Port of `src/renderer/src/application/renderers/2d/modules/Text.js`.
//!
//! # Scratch canvas elided
//!
//! The JS module keeps an `OffscreenCanvas`, re-renders the text into it only
//! when a prop's `set()` fires, and `draw()` blits it onto the group canvas
//! (`Text.js:135-137`). Here the text is drawn directly through
//! [`crate::two_d::Canvas::fill_text`] every frame: the intermediate canvas is
//! fully transparent apart from the glyphs, so the blit is a no-op
//! visually — it only saved CPU work in JS, which the GPU facade does not
//! need. Prop-`set()` timing is also irrelevant now that `draw` re-runs.
//!
//! # Wrapper behaviour (`canvas-text-wrapper`)
//!
//! `Text.js:176-183` passes `verticalAlign: "middle"`,
//! `textAlign: "center"`, the default `lineHeight: 1` and `offsetX`/`offsetY`
//! in percent of the canvas. The wrapper derives
//!
//! ```text
//! MAX_TXT_WIDTH   = width  - offsetX          (no padding)
//! textPos.x       = (width + offsetX) / 2     (textAlign center)
//! textPos.y       = (height - blockHeight + offsetY) / 2
//! per line: textPos.y += lineHeight; fillText(line, textPos.x, textPos.y)
//! ```
//!
//! which is reproduced below. `strokeText` is drawn *before* the fill here so
//! the stroke stamps sit behind opaque glyphs — see the note in
//! `Canvas::stroke_text`.

use crate::two_d::Canvas;
use crate::two_d::modules::{
    DrawContext, ModuleContext, TwoDModule, TwoDModuleMeta, TwoDProp, prop_bool, prop_f64,
    prop_rgba_unit, prop_str,
};

/// The `Text` module.
#[derive(Default)]
pub struct Text;

impl TwoDModule for Text {
    fn meta(&self) -> TwoDModuleMeta {
        TwoDModuleMeta::new("Text")
    }

    fn props(&self) -> Vec<TwoDProp> {
        vec![
            TwoDProp::text("text", ""),
            TwoDProp::int("size", 16).range(0.0, 1000.0),
            TwoDProp::float("offsetX", 0.0)
                .range(-100.0, 100.0)
                .step(1.0),
            TwoDProp::float("offsetY", 0.0)
                .range(-100.0, 100.0)
                .step(1.0),
            TwoDProp::float("strokeSize", 1.0)
                .range(0.0, 50.0)
                .step(1.0),
            TwoDProp::text("font", "sans-serif").component("FontControl"),
            TwoDProp::text("weight", "bold"),
            TwoDProp::bool("fill", true),
            TwoDProp::color_unit("fillColor", [1.0, 1.0, 1.0, 1.0]),
            TwoDProp::bool("stroke", false),
            TwoDProp::color_unit("strokeColor", [1.0, 0.0, 0.0, 1.0]),
        ]
    }

    fn draw(&mut self, ctx: &mut DrawContext) {
        // `Text.js:139-184` (`drawText`).
        let text = prop_str(ctx.props, "text", "");
        let size = prop_f64(ctx.props, "size", 16.0) as f32;
        if text.trim().is_empty() || size <= 0.0 {
            // `0px` font / empty text draw nothing in the browser too.
            return;
        }
        let offset_x_prop = prop_f64(ctx.props, "offsetX", 0.0) as f32;
        let offset_y_prop = prop_f64(ctx.props, "offsetY", 0.0) as f32;
        let stroke_size = prop_f64(ctx.props, "strokeSize", 1.0) as f32;
        let font = prop_str(ctx.props, "font", "sans-serif");
        let weight = prop_str(ctx.props, "weight", "bold");
        let fill = prop_bool(ctx.props, "fill", true);
        let fill_colour = prop_rgba_unit(ctx.props, "fillColor", [1.0, 1.0, 1.0, 1.0]);
        let stroke = prop_bool(ctx.props, "stroke", false);
        let stroke_colour = prop_rgba_unit(ctx.props, "strokeColor", [1.0, 0.0, 0.0, 1.0]);

        let width = ctx.canvas.width() as f32;
        let height = ctx.canvas.height() as f32;
        let offset_x = (width / 100.0) * offset_x_prop;
        let offset_y = (height / 100.0) * offset_y_prop;

        // `context.strokeStyle`, `context.lineWidth` (`Text.js:167-170`).
        ctx.canvas.set_stroke_style(stroke_colour);
        ctx.canvas.set_line_width(stroke_size);
        // `context.fillStyle` — transparent when `fill` is off (`Text.js:160-166`).
        ctx.canvas.set_fill_style(if fill {
            fill_colour
        } else {
            [0.0, 0.0, 0.0, 0.0]
        });
        ctx.canvas.set_font(weight, size, font);
        ctx.canvas.set_text_align(crate::two_d::TextAlign::Center);
        ctx.canvas
            .set_text_baseline(crate::two_d::TextBaseline::Bottom);

        let max_width = (width - offset_x).max(1.0);
        let block_height = ctx.canvas.text_block_height(text, Some(max_width));
        // Wrapper: `textPos.y = (EL_HEIGHT - textBlockHeight + offsetY) / 2`,
        // then `textPos.y += lineHeight` before the first line.
        let first_baseline = (height - block_height + offset_y) / 2.0 + size;
        let anchor_x = (width + offset_x) / 2.0;

        // Stroke first so the fill covers the stamp interiors.
        if stroke && stroke_size > 0.0 {
            ctx.canvas
                .stroke_text(text, anchor_x, first_baseline, Some(max_width));
        }
        if fill {
            ctx.canvas
                .fill_text(text, anchor_x, first_baseline, Some(max_width));
        }
    }

    fn resize(&mut self, ctx: &mut ModuleContext) {
        // The JS re-renders its scratch canvas on resize (`Text.js:128-133`);
        // nothing to do now that drawing is per frame.
        let _ = ctx;
    }

    fn dispose(&mut self) {}
}

/// Compile-time reminder that `Canvas` is only used through `DrawContext`.
#[allow(dead_code)]
fn _canvas_type(_: &mut Canvas) {}
