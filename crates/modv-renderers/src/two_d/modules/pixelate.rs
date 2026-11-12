//! `Pixelate.js` — downsample the canvas and blow it back up.
//!
//! Port of `src/renderer/src/application/renderers/2d/modules/Pixelate.js`.
//! The JS keeps a scratch `OffscreenCanvas` (`:44-60`); here the module owns a
//! scratch [`Canvas`] created in `init`. `imageSmoothingEnabled = false`
//! (`:47`, `:83`) maps to [`Canvas::set_image_smoothing`], which selects the
//! nearest-neighbour sampler.

use crate::two_d::Canvas;
use crate::two_d::modules::{
    DrawContext, ModuleContext, TwoDModule, TwoDModuleMeta, TwoDProp, prop_bool, prop_f64, prop_i64,
};

/// The `Pixelate` module.
pub struct Pixelate {
    scratch: Option<Canvas>,
}

impl Default for Pixelate {
    fn default() -> Self {
        Self { scratch: None }
    }
}

impl TwoDModule for Pixelate {
    fn meta(&self) -> TwoDModuleMeta {
        TwoDModuleMeta {
            name: "Pixelate",
            author: "2xAA",
            version: "1.0.0",
            category: Some("effects"),
            audio_features: &["zcr", "rms"],
        }
    }

    fn props(&self) -> Vec<TwoDProp> {
        vec![
            TwoDProp::int("pixelAmount", 5)
                .label("Amount")
                .range(2.0, 30.0)
                .step(1.0),
            TwoDProp::bool("soundReactive", false).label("Sound Reactive"),
            TwoDProp::int("intensity", 15)
                .label("RMS/ZCR Intensity")
                .range(0.0, 30.0)
                .step(1.0),
            TwoDProp::bool("soundType", false).label("RMS (unchecked) / ZCR (checked)"),
        ]
    }

    fn init(&mut self, ctx: &mut ModuleContext) {
        // `Pixelate.js:44-53`.
        self.scratch = Some(Canvas::new(
            ctx.device,
            ctx.queue,
            ctx.canvas_width.max(1),
            ctx.canvas_height.max(1),
        ));
    }

    fn resize(&mut self, ctx: &mut ModuleContext) {
        // `Pixelate.js:55-60`.
        if let Some(scratch) = self.scratch.as_mut() {
            scratch.resize(
                ctx.device,
                ctx.queue,
                ctx.canvas_width.max(1),
                ctx.canvas_height.max(1),
            );
        }
    }

    fn draw(&mut self, ctx: &mut DrawContext) {
        // `Pixelate.js:62-107`.
        let Some(scratch) = self.scratch.as_mut() else {
            return;
        };
        let sound_reactive = prop_bool(ctx.props, "soundReactive", false);
        let sound_type = prop_bool(ctx.props, "soundType", false);
        let intensity = prop_i64(ctx.props, "intensity", 15) as f32;
        let pixel_amount = prop_f64(ctx.props, "pixelAmount", 5.0) as f32;

        let width = ctx.canvas.width() as f32;
        let height = ctx.canvas.height() as f32;

        let (w, h) = if sound_reactive {
            let analysed = if sound_type {
                ctx.audio.features.zcr / (10.0 * intensity)
            } else {
                ctx.audio.features.rms * 10.0 * intensity
            };
            // The JS happily divides by zero (`w = Infinity`) and `drawImage`
            // then becomes a no-op; skipping non-finite values is equivalent.
            if !analysed.is_finite() || analysed.abs() < f32::EPSILON {
                return;
            }
            (width / analysed, height / analysed)
        } else {
            (width / pixel_amount, height / pixel_amount)
        };
        if !(w > 0.0) || !(h > 0.0) || !w.is_finite() || !h.is_finite() {
            return;
        }

        ctx.canvas.save();

        // `Pixelate.js:82-94`: the source canvas is downscaled into the
        // scratch's top-left `w × h` region (the JS 9-argument `drawImage`
        // with the destination size, not a source crop), with smoothing off.
        scratch.clear_all();
        scratch.set_image_smoothing(false);
        scratch.draw_image(ctx.canvas.view(), 0.0, 0.0, w, h);
        scratch.submit();

        // `Pixelate.js:95-105`: blow the scratch's `w × h` corner back up,
        // with the main context's smoothing disabled.
        ctx.canvas.set_image_smoothing(false);
        ctx.canvas.draw_image_region(
            scratch.view(),
            0.0,
            0.0,
            w / width,
            h / height,
            0.0,
            0.0,
            width,
            height,
        );

        ctx.canvas.restore();
    }

    fn dispose(&mut self) {
        self.scratch = None;
    }
}
