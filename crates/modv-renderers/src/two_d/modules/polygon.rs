//! `Polygon.js` — a rotating n-gon whose radius/side count react to RMS/ZCR.
//!
//! Port of `src/renderer/src/application/renderers/2d/modules/Polygon.js`.
//! The JS `polygon()` helper (`:153-172`) builds its path inside
//! `save`/`translate`/`rotate`/`restore` and the caller strokes *after* the
//! restore (`:137-150`); the facade transforms points when they are added, so
//! the same sequence is transcribed directly.

use serde_json::json;

use crate::two_d::modules::{
    DrawContext, ModuleContext, TwoDModule, TwoDModuleMeta, TwoDProp, prop_bool, prop_f64,
    prop_tween_rgb,
};

/// The `Polygon` module.
#[derive(Default)]
pub struct Polygon {
    /// `data.rotation`, advanced by `rotateSpeed` each frame.
    rotation: f32,
}

impl TwoDModule for Polygon {
    fn meta(&self) -> TwoDModuleMeta {
        TwoDModuleMeta {
            name: "Polygon",
            author: "2xAA",
            version: "0.2.0",
            category: None,
            audio_features: &["rms", "zcr"],
        }
    }

    fn props(&self) -> Vec<TwoDProp> {
        vec![
            TwoDProp::int("intensity", 15)
                .label("RMS/ZCR Intensity")
                .range(0.0, 30.0)
                .step(1.0),
            TwoDProp::int("shapeSize", 60)
                .label("Shape Size")
                .range(0.0, 300.0)
                .step(1.0),
            TwoDProp::int("strokeWeight", 1)
                .label("Stroke Weight")
                .range(1.0, 20.0)
                .step(1.0),
            TwoDProp::bool("fill", false).label("Fill"),
            TwoDProp::bool("rotateToggle", false).label("Rotate"),
            TwoDProp::float("rotateSpeed", 5.0)
                .label("Rotate Speed")
                .range(0.1, 10.0)
                .step(0.1),
            TwoDProp::tween(
                "color",
                json!({
                    "data": [
                        [199, 64, 163], [97, 214, 199], [222, 60, 75], [101, 151, 220],
                        [213, 158, 151], [100, 132, 129], [154, 94, 218], [194, 211, 205],
                        [201, 107, 152], [119, 98, 169], [214, 175, 208], [218, 57, 123],
                        [196, 96, 98], [218, 74, 219], [138, 100, 121], [96, 118, 225],
                        [132, 195, 223], [82, 127, 162], [209, 121, 211], [181, 152, 220],
                    ],
                    "duration": 500,
                    "easing": "linear",
                }),
            ),
        ]
    }

    fn update(&mut self, ctx: &mut ModuleContext) {
        // `Polygon.js:96-104` — note the JS resets to 0 rather than subtracting
        // 360; the phase jump is part of the effect, so it is kept.
        self.rotation += prop_f64(ctx.props, "rotateSpeed", 5.0) as f32;
        if self.rotation > 360.0 {
            self.rotation = 0.0;
        }
    }

    fn draw(&mut self, ctx: &mut DrawContext) {
        // `Polygon.js:106-151`.
        let intensity = prop_f64(ctx.props, "intensity", 15.0) as f32;
        let sound_type = prop_bool(ctx.props, "soundType", false);
        let shape_size = prop_f64(ctx.props, "shapeSize", 60.0) as f32;
        let fill = prop_bool(ctx.props, "fill", false);
        let stroke_weight = prop_f64(ctx.props, "strokeWeight", 1.0) as f32;
        let rotate_toggle = prop_bool(ctx.props, "rotateToggle", false);
        let colour = prop_tween_rgb(ctx.props, "color", [1.0, 1.0, 1.0, 1.0]);

        let rotate = if rotate_toggle { self.rotation } else { 0.0 };

        let analysed = if sound_type {
            (ctx.audio.features.zcr / 10.0) * intensity
        } else {
            ctx.audio.features.rms * 10.0 * intensity
        };

        ctx.canvas.set_stroke_style(colour);
        ctx.canvas.set_fill_style(colour);
        ctx.canvas.set_line_width(stroke_weight);

        let width = ctx.canvas.width() as f32;
        let height = ctx.canvas.height() as f32;
        let radius = analysed + shape_size;
        let sides = 3 + (analysed / 10.0).round() as i32;
        let start_angle = rotate * 0.0174533;

        ctx.canvas.begin_path();
        if sides >= 3 {
            // `polygon()` (`Polygon.js:153-172`).
            let step = std::f32::consts::TAU / sides as f32;
            ctx.canvas.save();
            ctx.canvas
                .translate((width / 2.0).round(), (height / 2.0).round());
            ctx.canvas.rotate(start_angle);
            ctx.canvas.move_to(radius, 0.0);
            for index in 1..sides {
                ctx.canvas.line_to(
                    radius * (step * index as f32).cos(),
                    radius * (step * index as f32).sin(),
                );
            }
            ctx.canvas.close_path();
            ctx.canvas.restore();
        }
        ctx.canvas.close_path();
        ctx.canvas.stroke();
        if fill {
            ctx.canvas.fill();
        }
    }
}
