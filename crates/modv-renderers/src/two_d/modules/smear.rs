//! `Smear.js` — draws the canvas onto itself, offset and scaled.
//!
//! Port of `src/renderer/src/application/renderers/2d/modules/Smear.js`:
//! `context.drawImage(canvas, -speedX, -speedY, width + sizeX, height + sizeY)`
//! (`:40`) — the 5-argument form, so it samples the *whole* canvas and scales
//! it into a destination rect that is shifted by `-speed` and grown by `size`.
//! The facade equivalent is [`Canvas::draw_image_self`] (a snapshot of the
//! canvas as of this draw).

use crate::two_d::modules::{DrawContext, TwoDModule, TwoDModuleMeta, TwoDProp, prop_i64};

/// The `Smear` module.
#[derive(Default)]
pub struct Smear;

impl TwoDModule for Smear {
    fn meta(&self) -> TwoDModuleMeta {
        TwoDModuleMeta {
            name: "Smear",
            author: "",
            version: "",
            category: Some("effects"),
            audio_features: &["rms", "energy"],
        }
    }

    fn props(&self) -> Vec<TwoDProp> {
        vec![
            TwoDProp::int("speedX", 0).range(-20.0, 20.0),
            TwoDProp::int("speedY", 10).range(-20.0, 20.0),
            TwoDProp::int("sizeX", 0).range(-20.0, 20.0),
            TwoDProp::int("sizeY", 0).range(-20.0, 20.0),
        ]
    }

    fn draw(&mut self, ctx: &mut DrawContext) {
        // `Smear.js:36-41`.
        let speed_x = prop_i64(ctx.props, "speedX", 0) as f32;
        let speed_y = prop_i64(ctx.props, "speedY", 10) as f32;
        let size_x = prop_i64(ctx.props, "sizeX", 0) as f32;
        let size_y = prop_i64(ctx.props, "sizeY", 0) as f32;

        let width = ctx.canvas.width() as f32;
        let height = ctx.canvas.height() as f32;
        let destination_width = width + size_x;
        let destination_height = height + size_y;
        // A non-positive destination is a no-op in the browser (`drawImage`
        // with an empty destination rect); the facade skips it too, but the
        // guard keeps the intent explicit.
        if destination_width <= 0.0 || destination_height <= 0.0 {
            return;
        }

        ctx.canvas
            .draw_image_self(-speed_x, -speed_y, destination_width, destination_height);
    }
}
