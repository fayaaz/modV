//! `Concentrics.js` — two offset rings of stroked circles.
//!
//! Port of `src/renderer/src/application/renderers/2d/modules/Concentrics.js`.
//! `hsl()` strings (`:143-149`) become [`hsl_to_rgba`]; the last partial ring
//! keeps its dimmer lightness (`(1 - (amp - round(amp))) * 50 %`).

use crate::two_d::modules::{
    DrawContext, ModuleContext, SimpleRng, TwoDModule, TwoDModuleMeta, TwoDProp, hsl_to_rgba,
    prop_bool, prop_f64,
};

/// One ring centre (`circlesObjectFactory`).
#[derive(Clone, Debug)]
struct Circles {
    hue: f32,
    x: f32,
    y: f32,
}

/// The `Concentrics` module.
pub struct Concentrics {
    rng: SimpleRng,
    circles: Vec<Circles>,
}

impl Default for Concentrics {
    fn default() -> Self {
        Self {
            rng: SimpleRng::new(),
            circles: Vec::new(),
        }
    }
}

impl Concentrics {
    /// `updateCircles` (`Concentrics.js:128-139`).
    fn update_circles(circles: &mut Circles, x: f32, y: f32) {
        if circles.hue > 360.0 {
            circles.hue = 0.0;
        } else {
            circles.hue += 0.2;
        }
        circles.x = x;
        circles.y = y;
    }
}

impl TwoDModule for Concentrics {
    fn meta(&self) -> TwoDModuleMeta {
        TwoDModuleMeta {
            name: "Concentrics",
            author: "2xAA",
            version: "1.0.0",
            category: None,
            audio_features: &["zcr", "rms"],
        }
    }

    fn props(&self) -> Vec<TwoDProp> {
        vec![
            TwoDProp::bool("rms", true).label("Use RMS"),
            TwoDProp::float("intensity", 1.0)
                .label("RMS/ZCR Intensity")
                .range(0.0, 30.0),
            TwoDProp::float("spacing", 5.0)
                .label("Circle Spacing")
                .range(0.0, 100.0),
            TwoDProp::float("objectDistance", 40.0)
                .label("Object Distance")
                .range(0.0, 200.0),
            TwoDProp::float("strokeWeight", 1.0)
                .label("Stroke Weight")
                .range(1.0, 20.0),
        ]
    }

    fn init(&mut self, ctx: &mut ModuleContext) {
        // `Concentrics.js:58-69`.
        let x = ctx.canvas_width as f32 / 2.0;
        let y = ctx.canvas_height as f32 / 2.0;
        let mut circles = Vec::with_capacity(2);
        for _ in 0..2 {
            circles.push(Circles {
                hue: (self.rng.next_f32() * 360.0).round(),
                x,
                y,
            });
        }
        self.circles = circles;
    }

    fn update(&mut self, ctx: &mut ModuleContext) {
        // `Concentrics.js:71-89`.
        if self.circles.len() < 2 {
            return;
        }
        let width_half = ctx.canvas_width as f32 / 2.0;
        let height_half = ctx.canvas_height as f32 / 2.0;
        let distance = prop_f64(ctx.props, "objectDistance", 40.0) as f32;
        let phase = ctx.delta / 1000.0;

        Self::update_circles(
            &mut self.circles[0],
            width_half + phase.sin() * distance,
            height_half + phase.cos() * (distance / 2.0),
        );
        Self::update_circles(
            &mut self.circles[1],
            width_half + -phase.sin() * distance,
            height_half + -phase.cos() * (distance / 2.0),
        );
    }

    fn draw(&mut self, ctx: &mut DrawContext) {
        // `Concentrics.js:91-118`.
        let stroke_weight = prop_f64(ctx.props, "strokeWeight", 1.0) as f32;
        let spacing = prop_f64(ctx.props, "spacing", 5.0) as f32;
        let rms = prop_bool(ctx.props, "rms", true);
        let intensity = prop_f64(ctx.props, "intensity", 1.0) as f32;

        let mut amp = if rms {
            ctx.audio.features.rms
        } else {
            ctx.audio.features.zcr
        };
        amp *= intensity;
        if rms {
            amp *= 50.0;
        }

        // `Concentrics.js:141-157`.
        let steps = if amp.is_finite() && amp > 0.0 {
            amp.ceil() as i64
        } else {
            0
        };
        for circles in &self.circles {
            ctx.canvas.set_line_width(stroke_weight);
            let base = hsl_to_rgba(circles.hue, 0.5, 0.5);
            for index in 0..steps {
                let mut colour = base;
                if (index as f32) == amp - 1.0 {
                    let lightness = (1.0 - (amp - amp.round())) * 0.5;
                    colour = hsl_to_rgba(circles.hue, 0.5, lightness);
                }
                ctx.canvas.set_stroke_style(colour);
                ctx.canvas.begin_path();
                ctx.canvas.arc(
                    circles.x,
                    circles.y,
                    index as f32 * spacing,
                    0.0,
                    std::f32::consts::TAU,
                    false,
                );
                ctx.canvas.close_path();
                ctx.canvas.stroke();
            }
        }
    }
}
