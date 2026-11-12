//! `Line.js` — a random walk drawn as a fading line history.
//!
//! Port of `src/renderer/src/application/renderers/2d/modules/Line.js`.
//!
//! `draw` reads `this.lineWidth` (`:122`) — the module definition object,
//! which has no `lineWidth` property, so the JS assigns `undefined` and the
//! canvas keeps its previous width (the default 1). The port uses 1 and keeps
//! `strokeStyle = "#fff"` (`"#fff" || color` is always white, :124).

use crate::two_d::modules::{
    DrawContext, ModuleContext, SimpleRng, TwoDModule, TwoDModuleMeta, TwoDProp, prop_f64,
    prop_i64, prop_rgba_255,
};

/// The `Line` module.
pub struct Line {
    rng: SimpleRng,
    /// `data.vector`: two normalised points.
    vector: [[f32; 2]; 2],
    /// `data.velocity`: one sign per axis.
    velocity: [[f32; 2]; 2],
    /// `data.history`: copies of `vector` for the last `linesToShow` frames.
    history: Vec<[[f32; 2]; 2]>,
}

impl Default for Line {
    fn default() -> Self {
        Self {
            rng: SimpleRng::new(),
            vector: [[0.0; 2]; 2],
            velocity: [[1.0; 2]; 2],
            history: Vec::new(),
        }
    }
}

impl TwoDModule for Line {
    fn meta(&self) -> TwoDModuleMeta {
        TwoDModuleMeta {
            name: "Line",
            author: "2xAA",
            version: "",
            category: None,
            audio_features: &[],
        }
    }

    fn props(&self) -> Vec<TwoDProp> {
        vec![
            TwoDProp::int("linesToShow", 100).range(5.0, 200.0),
            TwoDProp::int("spacing", 1).range(1.0, 20.0),
            TwoDProp::int("lineWidth", 1).range(1.0, 20.0),
            TwoDProp::color_255("color", [255.0, 255.0, 255.0, 1.0]),
            TwoDProp::float("speed", 1.0).range(0.0, 2.0),
        ]
    }

    fn init(&mut self, _ctx: &mut ModuleContext) {
        // `Line.js:73-85`.
        self.vector = [
            [self.rng.next_f32(), self.rng.next_f32()],
            [self.rng.next_f32(), self.rng.next_f32()],
        ];
        self.velocity = [
            [
                if self.rng.coin() { 1.0 } else { -1.0 },
                if self.rng.coin() { 1.0 } else { -1.0 },
            ],
            [
                if self.rng.coin() { 1.0 } else { -1.0 },
                if self.rng.coin() { 1.0 } else { -1.0 },
            ],
        ];
        self.history.clear();
    }

    fn update(&mut self, ctx: &mut ModuleContext) {
        // `Line.js:87-119`.
        let lines_to_show = prop_i64(ctx.props, "linesToShow", 100).clamp(5, 200) as usize;
        let speed = prop_f64(ctx.props, "speed", 1.0) as f32;
        let spacing = prop_i64(ctx.props, "spacing", 1) as f32;
        let width = ctx.canvas_width.max(1) as f32;
        let pixel = (speed / width) * spacing;

        for axis in 0..2 {
            for component in 0..2 {
                let value = self.vector[axis][component];
                if value >= 1.0 || value <= 0.0 {
                    self.velocity[axis][component] = -self.velocity[axis][component];
                }
            }
        }

        for axis in 0..2 {
            for component in 0..2 {
                let step = if self.velocity[axis][component] > 0.0 {
                    pixel
                } else {
                    -pixel
                };
                // Note: the JS reads `data.velocity` after possibly flipping
                // it above, exactly as here.
                self.vector[axis][component] += step;
            }
        }

        self.history.push(self.vector);
        if self.history.len() > lines_to_show {
            let excess = self.history.len() - lines_to_show;
            self.history.drain(0..excess);
        }
    }

    fn draw(&mut self, ctx: &mut DrawContext) {
        // `Line.js:121-136`.
        let _colour = prop_rgba_255(ctx.props, "color", [1.0, 1.0, 1.0, 1.0]);
        let width = ctx.canvas.width() as f32;
        let height = ctx.canvas.height() as f32;

        ctx.canvas.set_stroke_style([1.0, 1.0, 1.0, 1.0]);
        ctx.canvas.set_line_width(1.0);

        for entry in &self.history {
            let [p1, p2] = entry;
            ctx.canvas.begin_path();
            ctx.canvas
                .move_to((p1[0] * width).round(), (p1[1] * height).round());
            ctx.canvas
                .line_to((p2[0] * width).round(), (p2[1] * height).round());
            ctx.canvas.stroke();
        }
    }
}
