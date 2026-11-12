//! `Waveform.js` — the time-domain audio buffer as a polyline.
//!
//! Port of `src/renderer/src/application/renderers/2d/modules/Waveform.js`.
//! The JS calls `meyda.windowing(features.buffer, windowing)` (`:75`); the
//! window functions are reimplemented here (`meyda`'s `rect`, `hanning`,
//! `hamming`, `blackman`, `sine`) because the Rust engine has no Meyda
//! instance. Values are used exactly as the JS uses them (`v = buffer[i] / 100
//! * maxHeight`, `:87`) — including the quiet output that produces, which is
//! the module's actual look.

use crate::two_d::modules::{
    DrawContext, TwoDModule, TwoDModuleMeta, TwoDProp, prop_f64, prop_i64, prop_str, prop_tween_rgb,
};

/// The `Waveform` module.
#[derive(Default)]
pub struct Waveform;

/// Apply a Meyda window function to a copy of `buffer`.
pub fn window(buffer: &[f32], kind: &str) -> Vec<f32> {
    let length = buffer.len();
    if length < 2 {
        return buffer.to_vec();
    }
    let denominator = (length - 1) as f32;
    buffer
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let position = std::f32::consts::TAU * index as f32 / denominator;
            let factor = match kind {
                "hanning" => 0.5 - 0.5 * position.cos(),
                "hamming" => 0.54 - 0.46 * position.cos(),
                "blackman" => 0.42 - 0.5 * position.cos() + 0.08 * (2.0 * position).cos(),
                "sine" => (std::f32::consts::PI * index as f32 / denominator).sin(),
                // "rect" and anything unknown: no window.
                _ => 1.0,
            };
            value * factor
        })
        .collect()
}

impl TwoDModule for Waveform {
    fn meta(&self) -> TwoDModuleMeta {
        TwoDModuleMeta {
            name: "Waveform",
            author: "2xAA",
            version: "1.0.0",
            category: None,
            audio_features: &["buffer"],
        }
    }

    fn props(&self) -> Vec<TwoDProp> {
        vec![
            TwoDProp::int("strokeWeight", 1)
                .label("Stroke")
                .range(1.0, 30.0),
            TwoDProp::float("maxHeight", 100.0)
                .label("Height")
                .range(1.0, 100.0),
            TwoDProp::float("maxWidth", 1.0)
                .label("Width")
                .range(0.0, 1.0),
            TwoDProp::enum_prop(
                "windowing",
                "hanning",
                &[
                    ("Rectangular (no window)", "rect"),
                    ("Hanning", "hanning"),
                    ("Hamming", "hamming"),
                    ("Blackman", "blackman"),
                    ("Sine", "sine"),
                ],
            )
            .label("Windowing"),
            TwoDProp::tween(
                "color",
                serde_json::json!({
                    "data": [[255, 255, 255], [255, 255, 255]],
                    "duration": 10000,
                    "easing": "linear",
                }),
            ),
        ]
    }

    fn draw(&mut self, ctx: &mut DrawContext) {
        // `Waveform.js:65-101`.
        let colour = prop_tween_rgb(ctx.props, "color", [1.0, 1.0, 1.0, 1.0]);
        let max_width = prop_f64(ctx.props, "maxWidth", 1.0) as f32;
        let max_height = prop_f64(ctx.props, "maxHeight", 100.0) as f32;
        let stroke_weight = prop_i64(ctx.props, "strokeWeight", 1) as f32;
        let windowing = prop_str(ctx.props, "windowing", "hanning");

        let width = ctx.canvas.width() as f32;
        let height = ctx.canvas.height() as f32;
        let buffer = window(&ctx.audio.features.buffer, windowing);
        if buffer.is_empty() {
            return;
        }
        let buffer_length = buffer.len();

        ctx.canvas.set_line_width(stroke_weight);
        ctx.canvas.set_stroke_style(colour);
        ctx.canvas.begin_path();

        let slice_width = (width * max_width) / buffer_length as f32;
        let mut x = 0.0f32;

        for (index, sample) in buffer.iter().enumerate() {
            let value = (sample / 100.0) * max_height;
            let y = height / 2.0 + (height / 2.0) * value;
            if index == 0 {
                ctx.canvas.move_to(x, y);
            } else {
                ctx.canvas.line_to(x, y);
            }
            x += slice_width;
        }

        ctx.canvas.line_to(width * max_width, height / 2.0);
        ctx.canvas.stroke();
    }
}
