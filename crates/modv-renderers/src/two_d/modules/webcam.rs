//! `Webcam.js` — draws the selected camera feed.
//!
//! Port of `src/renderer/src/application/renderers/2d/modules/Webcam.js`.
//! The JS picks a source with `selectWebcamSource(videos, camera, video)`
//! (`:42`) and exits when there is none (`:45-47`); the Rust engine hands the
//! chosen camera's frame to the module as [`DrawContext::input_texture`]
//! (M6 wires the per-camera captures into that texture). The camera index
//! prop is still exposed so the engine can pick the right capture, but the
//! module itself only needs the frame it is given.
//!
//! Position maths matches `:54-60`: the destination is centred on
//! `(width * position.x, height * (1 - position.y))` (the y flip makes the
//! default `[0.5, 0.5]` the centre).

use crate::two_d::modules::{
    DrawContext, TwoDModule, TwoDModuleMeta, TwoDProp, prop_bool, prop_f64, prop_vec2,
};

/// The `Webcam` module.
#[derive(Default)]
pub struct Webcam;

impl TwoDModule for Webcam {
    fn meta(&self) -> TwoDModuleMeta {
        TwoDModuleMeta {
            name: "Webcam",
            author: "2xAA",
            version: "1.0.0",
            category: None,
            audio_features: &[],
        }
    }

    fn props(&self) -> Vec<TwoDProp> {
        vec![
            TwoDProp::float("scale", 1.0).range(0.0, 10.0),
            TwoDProp::vec2("position", [0.5, 0.5], 0.0, 1.0),
            TwoDProp::bool("imageSmoothing", true),
            TwoDProp::int("camera", 1).label("Camera").range(1.0, 8.0),
        ]
    }

    fn draw(&mut self, ctx: &mut DrawContext) {
        // `Webcam.js:40-61`.
        let Some((view, (video_width, video_height))) = ctx.input() else {
            return;
        };
        let position = prop_vec2(ctx.props, "position", [0.5, 0.5]);
        let scale = prop_f64(ctx.props, "scale", 1.0) as f32;
        let smoothing = prop_bool(ctx.props, "imageSmoothing", true);

        let width = ctx.canvas.width() as f32;
        let height = ctx.canvas.height() as f32;
        let video_width = video_width as f32;
        let video_height = video_height as f32;

        ctx.canvas.set_image_smoothing(smoothing);

        ctx.canvas.draw_image(
            view,
            width * position[0] - (video_width * scale) / 2.0,
            height * (1.0 - position[1]) - (video_height * scale) / 2.0,
            video_width * scale,
            video_height * scale,
        );
    }
}
