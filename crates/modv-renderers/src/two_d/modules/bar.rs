//! `Bar.js` — a rotated bar that strobes between two colours.
//!
//! Port of `src/renderer/src/application/renderers/2d/modules/Bar.js`.
//! `delta` is the `requestAnimationFrame` timestamp the JS compares against
//! `strobe` (`:62`); a `strobe` of 0 makes the JS compute `NaN % 0`, whose
//! comparison is false → `color2`. That branch is ported explicitly so the
//! default (`strobe: 0`) is deterministic.

use crate::two_d::modules::{
    DrawContext, TwoDModule, TwoDModuleMeta, TwoDProp, prop_f64, prop_i64, prop_rgba_unit,
};

/// The `Bar` module.
#[derive(Default)]
pub struct Bar;

impl TwoDModule for Bar {
    fn meta(&self) -> TwoDModuleMeta {
        TwoDModuleMeta {
            name: "Bar",
            author: "2xAA",
            version: "",
            category: None,
            audio_features: &[],
        }
    }

    fn props(&self) -> Vec<TwoDProp> {
        vec![
            TwoDProp::float("rotation", 0.0).range(0.0, 360.0),
            TwoDProp::float("thickness", 20.0).range(0.0, 200.0),
            TwoDProp::int("strobe", 0).range(0.0, 240.0),
            TwoDProp::color_unit("color1", [0.0, 0.0, 0.0, 1.0]),
            TwoDProp::color_unit("color2", [1.0, 1.0, 1.0, 1.0]),
        ]
    }

    fn draw(&mut self, ctx: &mut DrawContext) {
        // `Bar.js:51-72`.
        let thickness = prop_f64(ctx.props, "thickness", 20.0) as f32;
        let strobe = prop_i64(ctx.props, "strobe", 0) as f32;
        let rotation = prop_f64(ctx.props, "rotation", 0.0) as f32;
        let color1 = prop_rgba_unit(ctx.props, "color1", [0.0, 0.0, 0.0, 1.0]);
        let color2 = prop_rgba_unit(ctx.props, "color2", [1.0, 1.0, 1.0, 1.0]);

        let width = ctx.canvas.width() as f32;
        let height = ctx.canvas.height() as f32;

        ctx.canvas.translate(width / 2.0, height / 2.0);
        ctx.canvas.rotate((rotation * std::f32::consts::PI) / 180.0);
        ctx.canvas.translate(-width / 2.0, -height / 2.0);

        ctx.canvas.set_line_width(thickness);
        ctx.canvas.begin_path();
        ctx.canvas.move_to(0.0, height / 2.0);
        ctx.canvas.line_to(width, height / 2.0);

        let phase = if strobe > 0.0 {
            ctx.delta.round() % strobe
        } else {
            f32::NAN
        };
        if phase < (strobe / 2.0).round() {
            ctx.canvas.set_stroke_style(color1);
        } else {
            ctx.canvas.set_stroke_style(color2);
        }
        ctx.canvas.stroke();
    }
}
