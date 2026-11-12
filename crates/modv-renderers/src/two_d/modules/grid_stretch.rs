//! `GridStretch.js` — tiles of the canvas stretched by audio.
//!
//! Port of `src/renderer/src/application/renderers/2d/modules/GridStretch.js`.
//! The JS keeps a scratch `OffscreenCanvas` (`:51-66`); here the module owns a
//! scratch [`Canvas`]. The loops are inclusive (`i >= 0`, `j >= 0`, `:81-96`),
//! so the last row/column samples one slice past the canvas edge; the facade's
//! clamp-to-edge sampler matches the browser's edge extension closely enough
//! for that off-canvas strip (most of which lands outside the canvas anyway).

use crate::two_d::Canvas;
use crate::two_d::modules::{
    DrawContext, ModuleContext, TwoDModule, TwoDModuleMeta, TwoDProp, prop_bool, prop_f64, prop_i64,
};

/// The `Grid Stretch` module.
pub struct GridStretch {
    scratch: Option<Canvas>,
}

impl Default for GridStretch {
    fn default() -> Self {
        Self { scratch: None }
    }
}

impl TwoDModule for GridStretch {
    fn meta(&self) -> TwoDModuleMeta {
        TwoDModuleMeta {
            name: "Grid Stretch",
            author: "2xAA",
            version: "0.1",
            category: Some("effects"),
            audio_features: &["zcr", "rms"],
        }
    }

    fn props(&self) -> Vec<TwoDProp> {
        vec![
            TwoDProp::int("countX", 10)
                .label("Grid Size X")
                .range(1.0, 20.0)
                .step(1.0),
            TwoDProp::int("countY", 10)
                .label("Grid Size Y")
                .range(1.0, 20.0)
                .step(1.0),
            TwoDProp::float("intensity", 15.0)
                .label("RMS/ZCR Intensity")
                .range(0.0, 30.0),
            TwoDProp::bool("zcr", false).label("RMS (unchecked) / ZCR (checked)"),
        ]
    }

    fn init(&mut self, ctx: &mut ModuleContext) {
        // `GridStretch.js:51-58`.
        self.scratch = Some(Canvas::new(
            ctx.device,
            ctx.queue,
            ctx.canvas_width.max(1),
            ctx.canvas_height.max(1),
        ));
    }

    fn resize(&mut self, ctx: &mut ModuleContext) {
        // `GridStretch.js:61-65`.
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
        // `GridStretch.js:68-99`.
        let Some(scratch) = self.scratch.as_mut() else {
            return;
        };
        let count_x = prop_i64(ctx.props, "countX", 10).clamp(1, 20);
        let count_y = prop_i64(ctx.props, "countY", 10).clamp(1, 20);
        let intensity = prop_f64(ctx.props, "intensity", 15.0) as f32;
        let zcr = prop_bool(ctx.props, "zcr", false);

        let width = ctx.canvas.width() as f32;
        let height = ctx.canvas.height() as f32;
        let slice_width = width / count_x as f32;
        let slice_height = height / count_y as f32;

        scratch.clear_all();
        let analysed = if zcr {
            (ctx.audio.features.zcr / 10.0) * intensity
        } else {
            ctx.audio.features.rms * 10.0 * intensity
        };
        if !analysed.is_finite() {
            return;
        }

        // Inclusive loops (JS `for (var i = countX; i >= 0; i--)`).
        for i in (0..=count_x).rev() {
            for j in (0..=count_y).rev() {
                let source_x = i as f32 * slice_width;
                let source_y = j as f32 * slice_height;
                scratch.draw_image_region(
                    ctx.canvas.view(),
                    source_x / width,
                    source_y / height,
                    slice_width / width,
                    slice_height / height,
                    source_x - analysed,
                    source_y - analysed,
                    slice_width + analysed * 2.0,
                    slice_height + analysed * 2.0,
                );
            }
        }
        scratch.submit();

        // `GridStretch.js:98`.
        ctx.canvas
            .draw_image(scratch.view(), 0.0, 0.0, width, height);
    }

    fn dispose(&mut self) {
        self.scratch = None;
    }
}
