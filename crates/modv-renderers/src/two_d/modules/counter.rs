//! `Counter.js` — a white canvas with a running counter drawn on it.
//!
//! Port of `src/renderer/src/application/renderers/2d/modules/Counter.js`.
//! `fillText(data.counter, width / 2, height / 2)` uses the canvas' default
//! font (`10px sans-serif`, alphabetic baseline, left aligned), which is the
//! facade's [`crate::two_d::text::TextStyle::default`] — the module therefore
//! only sets colours.
//!
//! JS `String(number)` formatting is matched with `Display` on the `f32`
//! (both print `3` for `3.0` and `1.5` for `1.5`).

use crate::two_d::modules::{
    DrawContext, ModuleContext, TwoDModule, TwoDModuleMeta, TwoDProp, prop_f64,
};

/// The `Counter` module.
#[derive(Default)]
pub struct Counter {
    /// `data.counter`.
    counter: f32,
}

impl Counter {
    /// `multiply` (`Counter.js:20-22`).
    fn multiply(value: f32, multiplier: f32) -> f32 {
        value * multiplier
    }
}

impl TwoDModule for Counter {
    fn meta(&self) -> TwoDModuleMeta {
        TwoDModuleMeta::new("Counter")
    }

    fn props(&self) -> Vec<TwoDProp> {
        vec![TwoDProp::float("input1", 0.5).range(0.0, 1.0)]
    }

    fn update(&mut self, ctx: &mut ModuleContext) {
        // `Counter.js:24-27`.
        let input = prop_f64(ctx.props, "input1", 0.5) as f32;
        self.counter += Self::multiply(input, 2.0);
    }

    fn draw(&mut self, ctx: &mut DrawContext) {
        // `Counter.js:29-34`.
        let width = ctx.canvas.width() as f32;
        let height = ctx.canvas.height() as f32;

        ctx.canvas.set_fill_style([1.0, 1.0, 1.0, 1.0]);
        ctx.canvas.fill_rect(0.0, 0.0, width, height);
        ctx.canvas.set_fill_style([0.0, 0.0, 0.0, 1.0]);
        let text = format!("{}", self.counter);
        ctx.canvas.fill_text(&text, width / 2.0, height / 2.0, None);
    }
}
