//! `X-Drips.js` — a dripping-X pattern with circular clear notches.
//!
//! Port of `src/renderer/src/application/renderers/2d/modules/X-Drips.js`.
//! `generatePattern` (`:41-79`) uses `Math.random()`; [`SimpleRng`] replaces
//! it. The palette is hex strings parsed by [`hex_to_rgba`]. `clearCircle`
//! (`:81-88`) is a circular `clip()` + `clearRect`; the facade clips circles
//! analytically so the notches stay round.

use crate::two_d::modules::{
    DrawContext, ModuleContext, SimpleRng, TwoDModule, TwoDModuleMeta, TwoDProp, hex_to_rgba,
    prop_f64, prop_i64,
};

/// The pattern's piece kind (`"width"` / `"space"`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PieceKind {
    Width,
    Space,
}

/// One generated pattern piece.
#[derive(Clone, Debug)]
struct Piece {
    kind: PieceKind,
    width: f32,
    height: f32,
    random: f32,
}

/// The `X Drips` module.
pub struct XDrips {
    rng: SimpleRng,
    spacings: [f32; 2],
    widths: [f32; 3],
    heights: [f32; 3],
    colors: [&'static str; 6],
    color: &'static str,
    pattern: Vec<Piece>,
}

impl Default for XDrips {
    fn default() -> Self {
        Self {
            rng: SimpleRng::new(),
            spacings: [50.0, 30.0],
            widths: [30.0, 50.0, 80.0],
            heights: [80.0, 145.0, 220.0],
            colors: [
                "#15216b", "#f33a58", "#f76272", "#ff9fda", "#e10079", "#a231ef",
            ],
            color: "#000",
            pattern: Vec::new(),
        }
    }
}

impl XDrips {
    /// `generatePattern` (`X-Drips.js:41-79`).
    fn generate_pattern(&mut self, width: f32) -> Vec<Piece> {
        let mut pattern = Vec::new();
        let mut overall_width = 0.0f32;
        let mut flip = false;

        while overall_width < width {
            let selected_height = self.heights[self.rng.index(self.heights.len())];
            let random = self.rng.next_f32();

            let (selected_width, kind) = if flip {
                (
                    self.widths[self.rng.index(self.widths.len())],
                    PieceKind::Width,
                )
            } else {
                (
                    self.spacings[self.rng.index(self.spacings.len())],
                    PieceKind::Space,
                )
            };

            pattern.push(Piece {
                kind,
                width: selected_width,
                height: selected_height,
                random,
            });
            overall_width += selected_width;
            flip = !flip;
        }

        pattern
    }

    /// `clearCircle` (`X-Drips.js:81-88`).
    fn clear_circle(ctx: &mut DrawContext, x: f32, y: f32, radius: f32) {
        ctx.canvas.save();
        ctx.canvas.begin_path();
        ctx.canvas
            .arc(x, y, radius, 0.0, std::f32::consts::TAU, true);
        ctx.canvas.clip();
        ctx.canvas
            .clear_rect(x - radius, y - radius, radius * 2.0, radius * 2.0);
        ctx.canvas.restore();
    }
}

impl TwoDModule for XDrips {
    fn meta(&self) -> TwoDModuleMeta {
        TwoDModuleMeta {
            name: "X Drips",
            author: "",
            version: "",
            category: Some("effects"),
            audio_features: &[],
        }
    }

    fn props(&self) -> Vec<TwoDProp> {
        vec![
            TwoDProp::int("dripBaseHeight", 70).range(0.0, 70.0),
            TwoDProp::int("insetHeight", 15).range(0.0, 70.0),
            TwoDProp::float("maxDripHeight", 1.0).range(0.0, 1.0),
        ]
    }

    fn init(&mut self, ctx: &mut ModuleContext) {
        // `X-Drips.js:90-94`.
        self.color = self.colors[self.rng.index(self.colors.len())];
        self.pattern = self.generate_pattern(ctx.canvas_width as f32);
    }

    fn resize(&mut self, ctx: &mut ModuleContext) {
        // `X-Drips.js:96-99`.
        self.pattern = self.generate_pattern(ctx.canvas_width as f32);
    }

    fn draw(&mut self, ctx: &mut DrawContext) {
        // `X-Drips.js:101-156`.
        let drip_base_height = prop_i64(ctx.props, "dripBaseHeight", 70) as f32;
        let inset_height = prop_i64(ctx.props, "insetHeight", 15) as f32;
        let max_drip_height = prop_f64(ctx.props, "maxDripHeight", 1.0) as f32;
        let color = hex_to_rgba(self.color);
        let width = ctx.canvas.width() as f32;
        let height = ctx.canvas.height() as f32;

        ctx.canvas.clear_rect(0.0, 0.0, width, height);
        ctx.canvas.set_fill_style(color);
        ctx.canvas
            .fill_rect(0.0, 0.0, width, drip_base_height + inset_height);

        let mut x = 0.0f32;
        for piece in &self.pattern {
            let h = (piece.width / 2.0 + ((ctx.delta / 500.0) * piece.random).sin() * 30.0)
                * max_drip_height;

            match piece.kind {
                PieceKind::Width => {
                    ctx.canvas.set_fill_style(color);
                    ctx.canvas
                        .fill_rect(x, inset_height, piece.width, piece.height + h);
                    ctx.canvas.begin_path();
                    ctx.canvas.arc(
                        x + piece.width / 2.0,
                        inset_height + piece.height + h,
                        piece.width / 2.0,
                        0.0,
                        std::f32::consts::TAU,
                        false,
                    );
                    ctx.canvas.fill();
                }
                PieceKind::Space => {
                    ctx.canvas.clear_rect(
                        x,
                        drip_base_height - h / 3.0,
                        piece.width,
                        inset_height + piece.height,
                    );
                    Self::clear_circle(
                        ctx,
                        x + piece.width / 2.0,
                        drip_base_height - h / 3.0,
                        piece.width / 2.0,
                    );
                    Self::clear_circle(
                        ctx,
                        x + piece.width / 2.0,
                        inset_height + piece.height + drip_base_height - h / 3.0,
                        piece.width / 2.0,
                    );
                }
            }

            x += piece.width;
        }
    }
}
