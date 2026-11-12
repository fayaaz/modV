//! `Buffer.js` — captures the group canvas into a per-module buffer output.
//!
//! Port of `src/renderer/src/application/renderers/2d/modules/Buffer.js`.
//!
//! # Auxiliary output, translated
//!
//! The JS registers a `<modV>-buffer` auxillary output through the `outputs`
//! store module (`:54-84`) and then draws the group canvas into it and/or the
//! buffer back over the canvas at `bufferOpacity` (`:106-163`). In the Rust
//! engine the module owns a scratch [`Canvas`] of the same size — the "buffer
//! output" — created in `init`. The output *name*/registration is the
//! engine's business (the JS `update` only renames the auxillary output), so
//! `update` has nothing to do here.
//!
//! `bufferId`'s JS default is a function (a `Date.now()` string); a JSON prop
//! cannot hold a function, so the default is `""` and the module behaves like
//! the JS fallback name (`"Buffer"`). The canvas is read through
//! [`Canvas::snapshot`] (frame N samples frame N-1), never as the live target.

use crate::two_d::Canvas;
use crate::two_d::modules::{
    DrawContext, ModuleContext, TwoDModule, TwoDModuleMeta, TwoDProp, prop_bool, prop_f64, prop_str,
};

/// The `Buffer` module.
pub struct Buffer {
    /// The scratch canvas acting as the module's buffer output.
    buffer: Option<Canvas>,
    /// `data.captured`.
    captured: bool,
    /// `data.lastTrigger`.
    last_trigger: bool,
    /// The most recent `bufferId` (kept for the engine's output naming; see
    /// the module docs).
    buffer_id: String,
}

impl Default for Buffer {
    fn default() -> Self {
        Self {
            buffer: None,
            captured: false,
            last_trigger: false,
            buffer_id: String::new(),
        }
    }
}

impl TwoDModule for Buffer {
    fn meta(&self) -> TwoDModuleMeta {
        TwoDModuleMeta {
            name: "Buffer",
            author: "modV",
            version: "1.0.0",
            category: None,
            audio_features: &[],
        }
    }

    fn props(&self) -> Vec<TwoDProp> {
        vec![
            TwoDProp::text("bufferId", "").label("Buffer ID"),
            TwoDProp::bool("enabled", true).label("Enabled"),
            TwoDProp::enum_prop(
                "captureMode",
                "continuous",
                &[
                    ("Continuous", "continuous"),
                    ("On Trigger", "trigger"),
                    ("Once", "once"),
                ],
            )
            .label("Capture Mode"),
            TwoDProp::event("trigger").label("Trigger Capture"),
            TwoDProp::float("bufferOpacity", 0.0)
                .label("Buffer Opacity")
                .range(0.0, 1.0)
                .step(0.01),
            TwoDProp::bool("clearBuffer", false).label("Clear Buffer"),
        ]
    }

    fn init(&mut self, ctx: &mut ModuleContext) {
        // `addActiveModule` (`Buffer.js:54-74`): allocate the buffer output at
        // the current canvas size.
        self.buffer = Some(Canvas::new(
            ctx.device,
            ctx.queue,
            ctx.canvas_width.max(1),
            ctx.canvas_height.max(1),
        ));
        self.captured = false;
        self.last_trigger = false;
    }

    fn resize(&mut self, ctx: &mut ModuleContext) {
        if let Some(buffer) = self.buffer.as_mut() {
            buffer.resize(
                ctx.device,
                ctx.queue,
                ctx.canvas_width.max(1),
                ctx.canvas_height.max(1),
            );
        }
    }

    fn update(&mut self, ctx: &mut ModuleContext) {
        // `Buffer.js:85-105` only renames the auxillary output; the engine
        // owns output names. Keep the id for diagnostics.
        self.buffer_id = prop_str(ctx.props, "bufferId", "").to_owned();
    }

    fn draw(&mut self, ctx: &mut DrawContext) {
        // `Buffer.js:106-163`.
        let enabled = prop_bool(ctx.props, "enabled", true);
        let capture_mode = prop_str(ctx.props, "captureMode", "continuous");
        let trigger = prop_bool(ctx.props, "trigger", false);
        let buffer_opacity = prop_f64(ctx.props, "bufferOpacity", 0.0) as f32;
        let clear_buffer = prop_bool(ctx.props, "clearBuffer", false);

        let Some(buffer) = self.buffer.as_mut() else {
            return;
        };
        if !enabled {
            return;
        }

        if clear_buffer {
            buffer.clear_all();
            buffer.submit();
            self.captured = false;
        }

        let mut should_capture = false;
        if capture_mode == "trigger" {
            if trigger && !self.last_trigger {
                should_capture = true;
            }
            self.last_trigger = trigger;
        } else if capture_mode == "once" && !self.captured {
            should_capture = true;
        } else if capture_mode == "continuous" {
            should_capture = true;
        }

        if should_capture {
            let width = ctx.canvas.width();
            let height = ctx.canvas.height();
            // `bufferContext.drawImage(canvas, 0, 0, bufferCanvas.width, bufferCanvas.height)`.
            // The canvas is sampled through a snapshot: the live texture is the
            // frame the group is writing, so reading it directly is undefined.
            let snapshot = ctx.canvas.snapshot();
            buffer.clear_all();
            buffer.draw_image(&snapshot, 0.0, 0.0, width as f32, height as f32);
            buffer.submit();
            self.captured = true;
        }

        // `Buffer.js:157-162`: draw the buffer back with `globalAlpha`.
        if buffer_opacity > 0.0 && self.captured {
            ctx.canvas.save();
            ctx.canvas.set_global_alpha(buffer_opacity);
            ctx.canvas.draw_image(
                buffer.view(),
                0.0,
                0.0,
                ctx.canvas.width() as f32,
                ctx.canvas.height() as f32,
            );
            ctx.canvas.restore();
        }
    }

    fn dispose(&mut self) {
        self.buffer = None;
    }
}
