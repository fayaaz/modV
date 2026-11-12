//! `Ball.js` — bouncing balls whose radius reacts to RMS/ZCR.
//!
//! Ported line-for-line from
//! `src/renderer/src/application/renderers/2d/modules/Ball.js`:
//!
//! * `setupBalls` (`:131-145`) → [`Ball::setup_balls`], 300 balls with random
//!   positions/directions;
//! * `updateBall` (`:163-212`) → [`Ball::update_ball`], bounce or wrap;
//! * `draw`/`drawBall` (`:116-129`, `:214-229`) → [`TwoDModule::draw`]:
//!   `arc(x, y, round(radius + radius * analysed), 0, 2π, true)` + `fill`.

use serde_json::json;

use crate::two_d::modules::{
    DrawContext, ModuleContext, SimpleRng, TwoDModule, TwoDModuleMeta, TwoDProp, prop_bool,
    prop_f64, prop_i64, prop_tween_rgb,
};

/// One ball (`Ball.js`'s `ballFactory` object).
#[derive(Clone, Debug)]
struct BallState {
    x: f32,
    y: f32,
    direction_x: bool,
    direction_y: bool,
    radius: f32,
}

/// The `Ball` module.
pub struct Ball {
    rng: SimpleRng,
    balls: Vec<BallState>,
}

impl Default for Ball {
    fn default() -> Self {
        Self {
            rng: SimpleRng::new(),
            balls: Vec::new(),
        }
    }
}

impl Ball {
    /// `setupBalls` (`Ball.js:131-145`): always 300 balls, regardless of
    /// `props.amount`.
    fn setup_balls(&mut self, width: f32, height: f32) {
        let mut balls = Vec::with_capacity(300);
        for _ in 0..300 {
            balls.push(BallState {
                x: (self.rng.next_f32() * width + 1.0).floor(),
                y: (self.rng.next_f32() * height + 1.0).floor(),
                direction_x: self.rng.coin(),
                direction_y: self.rng.coin(),
                radius: 0.0,
            });
        }
        self.balls = balls;
    }

    /// `updateBall` (`Ball.js:163-212`).
    fn update_ball(
        ball: &mut BallState,
        width: f32,
        height: f32,
        speed: f32,
        radius: f32,
        wrap: bool,
    ) {
        ball.radius = radius;

        let mut dx = speed;
        let mut dy = speed;
        if ball.direction_x {
            dx = -dx;
        }
        if ball.direction_y {
            dy = -dy;
        }

        if wrap {
            if ball.x - radius < 1.0 {
                ball.x = width - radius;
            }
            if ball.y - radius < 1.0 {
                ball.y = height - radius;
            }
            if ball.x + radius > width {
                ball.x = radius;
            }
            if ball.y + radius > height {
                ball.y = radius;
            }
        } else {
            if ball.x + dx > width - radius || ball.x + dx < radius {
                ball.direction_x = !ball.direction_x;
            }
            if ball.y + dy > height - radius || ball.y + dy < radius {
                ball.direction_y = !ball.direction_y;
            }
        }

        ball.x += dx;
        ball.y += dy;
    }
}

impl TwoDModule for Ball {
    fn meta(&self) -> TwoDModuleMeta {
        TwoDModuleMeta {
            name: "Ball",
            author: "2xAA",
            version: "1.0.1",
            category: None,
            audio_features: &["zcr", "rms"],
        }
    }

    fn props(&self) -> Vec<TwoDProp> {
        vec![
            TwoDProp::int("amount", 10)
                .label("Amount")
                .range(1.0, 300.0),
            TwoDProp::float("speed", 2.0)
                .label("Speed")
                .range(0.0, 20.0)
                .step(0.01),
            TwoDProp::bool("wrap", false).label("Wrap"),
            TwoDProp::int("size", 2)
                .label("Size")
                .range(1.0, 50.0)
                .step(1.0),
            TwoDProp::int("intensity", 15)
                .label("RMS/ZCR Intensity")
                .range(0.0, 30.0)
                .step(1.0),
            TwoDProp::bool("soundType", false).label("RMS (unchecked) / ZCR (checked)"),
            TwoDProp::tween(
                "color",
                json!({
                    "data": [[0, 0, 0], [255, 255, 255]],
                    "duration": 10000,
                    "easing": "linear",
                }),
            ),
        ]
    }

    fn init(&mut self, ctx: &mut ModuleContext) {
        self.setup_balls(ctx.canvas_width as f32, ctx.canvas_height as f32);
    }

    fn resize(&mut self, ctx: &mut ModuleContext) {
        self.setup_balls(ctx.canvas_width as f32, ctx.canvas_height as f32);
    }

    fn update(&mut self, ctx: &mut ModuleContext) {
        // `Ball.js:100-114`.
        let amount = prop_i64(ctx.props, "amount", 10).clamp(0, 300) as usize;
        let speed = prop_f64(ctx.props, "speed", 2.0) as f32;
        let wrap = prop_bool(ctx.props, "wrap", false);
        let radius = prop_f64(ctx.props, "size", 2.0) as f32;
        let width = ctx.canvas_width as f32;
        let height = ctx.canvas_height as f32;

        for index in 0..amount.min(self.balls.len()) {
            Self::update_ball(&mut self.balls[index], width, height, speed, radius, wrap);
        }
    }

    fn draw(&mut self, ctx: &mut DrawContext) {
        // `Ball.js:116-129`.
        let intensity = prop_f64(ctx.props, "intensity", 15.0) as f32;
        let sound_type = prop_bool(ctx.props, "soundType", false);
        let analysed = if sound_type {
            (ctx.audio.features.zcr / 10.0) * intensity
        } else {
            ctx.audio.features.rms * 10.0 * intensity
        };
        let colour = prop_tween_rgb(ctx.props, "color", [1.0, 1.0, 1.0, 1.0], ctx.delta);
        let amount = prop_i64(ctx.props, "amount", 10).clamp(0, 300) as usize;

        for index in 0..amount.min(self.balls.len()) {
            let ball = &self.balls[index];
            // `drawBall` (`Ball.js:214-229`).
            let radius = (ball.radius + ball.radius * analysed).round();
            if radius < 0.5 || !radius.is_finite() {
                continue;
            }
            ctx.canvas.begin_path();
            ctx.canvas.set_fill_style(colour);
            ctx.canvas
                .arc(ball.x, ball.y, radius, 0.0, std::f32::consts::TAU, true);
            ctx.canvas.fill();
            ctx.canvas.close_path();
        }
    }
}
