//! The 15 hand-written 2D modules, ported from
//! `src/renderer/src/application/renderers/2d/modules/*.js`.
//!
//! Each JS default export becomes a struct implementing [`TwoDModule`]:
//!
//! | JS module | Rust file | Notes |
//! |-----------|-----------|-------|
//! | `Ball.js` | `ball.rs` | bouncing balls, RMS/ZCR radius |
//! | `Text.js` | `text.rs` | `canvas-text-wrapper` behaviour inlined (no scratch canvas) |
//! | `Polygon.js` | `polygon.rs` | rotating n-gon |
//! | `Texture2d.js` | `texture_2d.rs` | needs `DrawContext::input_texture` |
//! | `Buffer.js` | `buffer.rs` | owns a scratch [`Canvas`] as its buffer output |
//! | `Concentrics.js` | `concentrics.rs` | HSL rings |
//! | `X-Drips.js` | `x_drips.rs` | pattern of drips + circular clip |
//! | `Line.js` | `line.rs` | random-walk line history |
//! | `Pixelate.js` | `pixelate.rs` | scratch-canvas downsample/upsample |
//! | `Waveform.js` | `waveform.rs` | windowed audio buffer polyline |
//! | `GridStretch.js` | `grid_stretch.rs` | scratch-canvas tile stretch |
//! | `Bar.js` | `bar.rs` | strobe bar |
//! | `Webcam.js` | `webcam.rs` | needs `DrawContext::input_texture` |
//! | `Smear.js` | `smear.rs` | self-sampling drawImage |
//! | `Counter.js` | `counter.rs` | white fill + `fillText` |
//!
//! # Props
//!
//! [`TwoDProp`] mirrors the JS prop descriptor: `name`, `type`, `default`,
//! `min`/`max`, `label`, enum members, plus the control hints (`component`,
//! `step`) the UI reads. Defaults are ported verbatim from each module's
//! `props` export — the only exception is a JS *function* default (Buffer's
//! timestamp-based `bufferId`), which cannot be represented in JSON; it
//! becomes `""` and the module falls back to the same `"Buffer"` name.
//!
//! The engine resolves tween/colour controls before calling the module; the
//! helpers here also accept the raw control object so a module renders
//! standalone in tests (`prop_tween_rgb`).
//!
//! # Audio
//!
//! `docs/rust-hybrid.md` WS-5: `features` is the Meyda feature bag and the
//! corpus reads `zcr`, `rms` and `buffer`. [`AudioFrame`] carries those plus
//! the AGC bands; the engine adapter (M2e) fills it.

#![forbid(unsafe_code)]

use serde_json::{Map, Value, json};

use crate::two_d::{AudioFrame, Canvas};

mod ball;
mod bar;
mod buffer;
mod concentrics;
mod counter;
mod grid_stretch;
mod line;
mod pixelate;
mod polygon;
mod smear;
mod text;
mod texture_2d;
mod waveform;
mod webcam;
mod x_drips;

pub use ball::Ball;
pub use bar::Bar;
pub use buffer::Buffer;
pub use concentrics::Concentrics;
pub use counter::Counter;
pub use grid_stretch::GridStretch;
pub use line::Line;
pub use pixelate::Pixelate;
pub use polygon::Polygon;
pub use smear::Smear;
pub use text::Text;
pub use texture_2d::Texture2d;
pub use waveform::Waveform;
pub use webcam::Webcam;
pub use x_drips::XDrips;

/// Module metadata (`meta` in JS).
#[derive(Clone, Debug, PartialEq)]
pub struct TwoDModuleMeta {
    /// Display name (`meta.name`).
    pub name: &'static str,
    /// Author (`meta.author`), `""` when the JS omits it.
    pub author: &'static str,
    /// Version (`meta.version`), `""` when the JS omits it.
    pub version: &'static str,
    /// Gallery category (`meta.category`), e.g. `"effects"`.
    pub category: Option<&'static str>,
    /// Requested Meyda features (`meta.audioFeatures`).
    pub audio_features: &'static [&'static str],
}

impl TwoDModuleMeta {
    /// Metadata with the JS defaults (`author`, `version`, `category` empty).
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            author: "",
            version: "",
            category: None,
            audio_features: &[],
        }
    }
}

/// An `enum` prop member (`{ label, value }`).
#[derive(Clone, Debug, PartialEq)]
pub struct TwoDEnumMember {
    /// UI label.
    pub label: &'static str,
    /// Stored value.
    pub value: &'static str,
}

/// A module prop descriptor.
#[derive(Clone, Debug, PartialEq)]
pub struct TwoDProp {
    /// Prop key (`props.<name>`).
    pub name: &'static str,
    /// Control type: `int`, `float`, `bool`, `text`, `color`, `vec2`,
    /// `enum`, `tween`, `texture`, `event`.
    pub prop_type: &'static str,
    /// Default value, as the untyped JSON the store/controls use.
    pub default: Value,
    /// Minimum (numeric props).
    pub min: Option<f64>,
    /// Maximum (numeric props).
    pub max: Option<f64>,
    /// UI label.
    pub label: Option<&'static str>,
    /// Members for `enum` props.
    pub enum_members: Option<Vec<TwoDEnumMember>>,
    /// Control component (`PaletteControl`, `FontControl`), when the JS names
    /// one.
    pub component: Option<&'static str>,
    /// Control step (numeric props).
    pub step: Option<f64>,
}

impl TwoDProp {
    /// An integer prop.
    pub fn int(name: &'static str, default: i64) -> Self {
        Self::numeric(name, "int", json!(default), None, None)
    }

    /// A float prop.
    pub fn float(name: &'static str, default: f64) -> Self {
        Self::numeric(name, "float", json!(default), None, None)
    }

    /// A boolean prop.
    pub fn bool(name: &'static str, default: bool) -> Self {
        Self::numeric(name, "bool", json!(default), None, None)
    }

    /// A text prop.
    pub fn text(name: &'static str, default: &str) -> Self {
        Self::numeric(name, "text", json!(default), None, None)
    }

    /// A `vec2` prop with min/max applied to both components.
    pub fn vec2(name: &'static str, default: [f64; 2], min: f64, max: f64) -> Self {
        Self::numeric(name, "vec2", json!(default), Some(min), Some(max))
    }

    /// A `color` prop whose components are `0..1`.
    pub fn color_unit(name: &'static str, default: [f64; 4]) -> Self {
        Self::numeric(
            name,
            "color",
            json!({ "r": default[0], "g": default[1], "b": default[2], "a": default[3] }),
            None,
            None,
        )
    }

    /// A `color` prop whose `r`/`g`/`b` are `0..255`.
    pub fn color_255(name: &'static str, default: [f64; 4]) -> Self {
        Self::color_unit(name, default)
    }

    /// An `enum` prop.
    pub fn enum_prop(
        name: &'static str,
        default: &'static str,
        members: &[(&'static str, &'static str)],
    ) -> Self {
        Self::numeric(name, "enum", json!(default), None, None).members(members)
    }

    /// A `tween` prop (`PaletteControl` in the JS).
    pub fn tween(name: &'static str, default: Value) -> Self {
        Self {
            component: Some("PaletteControl"),
            ..Self::numeric(name, "tween", default, None, None)
        }
    }

    /// A `texture` prop (no default; the engine fills the input texture).
    pub fn texture(name: &'static str) -> Self {
        Self::numeric(name, "texture", Value::Null, None, None)
    }

    /// An `event` prop (momentary trigger).
    pub fn event(name: &'static str) -> Self {
        Self::numeric(name, "event", Value::Null, None, None)
    }

    fn numeric(
        name: &'static str,
        prop_type: &'static str,
        default: Value,
        min: Option<f64>,
        max: Option<f64>,
    ) -> Self {
        Self {
            name,
            prop_type,
            default,
            min,
            max,
            label: None,
            enum_members: None,
            component: None,
            step: None,
        }
    }

    /// Set the UI label.
    pub fn label(mut self, label: &'static str) -> Self {
        self.label = Some(label);
        self
    }

    /// Set the min/max range.
    pub fn range(mut self, min: f64, max: f64) -> Self {
        self.min = Some(min);
        self.max = Some(max);
        self
    }

    /// Set the control step.
    pub fn step(mut self, step: f64) -> Self {
        self.step = Some(step);
        self
    }

    /// Mark the control component.
    pub fn component(mut self, component: &'static str) -> Self {
        self.component = Some(component);
        self
    }

    /// Set enum members.
    pub fn members(mut self, members: &[(&'static str, &'static str)]) -> Self {
        self.enum_members = Some(
            members
                .iter()
                .map(|(label, value)| TwoDEnumMember { label, value })
                .collect(),
        );
        self
    }
}

/// Context handed to `init`/`resize`/`update`.
///
/// `delta` is the JS `delta` argument, which is **not** a frame delta:
/// `renderers/2d/index.js:24` documents it as the
/// `requestAnimationFrame` timestamp and modules use it as a monotonic clock
/// (`Concentrics.js:78` `sin(delta / 1000)`, `X-Drips.js:116`
/// `sin((delta / 500) * random)`, `Bar.js:62` `Math.round(delta) % strobe`).
/// The engine supplies milliseconds since the first frame.
pub struct ModuleContext<'a> {
    /// Canvas width in pixels.
    pub canvas_width: u32,
    /// Canvas height in pixels.
    pub canvas_height: u32,
    /// Monotonic timestamp in milliseconds (see the struct docs).
    pub delta: f32,
    /// The current audio frame.
    pub audio: &'a AudioFrame,
    /// Frame counter since the module was activated.
    pub frame: u64,
    /// Resolved prop values (the same JSON the engine passes to `draw`).
    pub props: &'a Value,
    /// GPU device, for modules that own a scratch canvas.
    pub device: &'a wgpu::Device,
    /// GPU queue, for modules that own a scratch canvas.
    pub queue: &'a wgpu::Queue,
}

/// Context handed to [`TwoDModule::draw`].
pub struct DrawContext<'a> {
    /// The canvas facade (already `save`d by the caller in JS terms).
    pub canvas: &'a mut Canvas,
    /// Resolved prop values.
    pub props: &'a Value,
    /// The current audio frame.
    pub audio: &'a AudioFrame,
    /// Monotonic timestamp in milliseconds (the JS `delta` argument).
    pub delta: f32,
    /// Frame counter since the module was activated.
    pub frame: u64,
    /// GPU device (scratch canvas creation).
    pub device: &'a wgpu::Device,
    /// GPU queue (scratch canvas creation).
    pub queue: &'a wgpu::Queue,
    /// Upstream video/texture input for `Webcam`/`Texture2d`, supplied by the
    /// engine (M6). `None` means the prop is unset, so those modules draw
    /// nothing — exactly what the JS does when `texture.value`/the video
    /// source is missing.
    pub input_texture: Option<&'a wgpu::TextureView>,
    /// Pixel size of [`Self::input_texture`].
    pub input_size: Option<(u32, u32)>,
    /// Whether this is a Gallery preview (the JS `isGallery` flag).
    pub is_gallery: bool,
}

impl<'a> DrawContext<'a> {
    /// The input texture and its size, when both are available.
    pub fn input(&self) -> Option<(&'a wgpu::TextureView, (u32, u32))> {
        match (self.input_texture, self.input_size) {
            (Some(view), Some(size)) if size.0 > 0 && size.1 > 0 => Some((view, size)),
            _ => None,
        }
    }
}

/// The module lifecycle contract (the JS `{meta, props, init, resize, update,
/// draw}` object).
pub trait TwoDModule: Send {
    /// Static metadata.
    fn meta(&self) -> TwoDModuleMeta;

    /// Prop descriptors, in the JS declaration order.
    fn props(&self) -> Vec<TwoDProp>;

    /// Per-instance data initialisation, once at activation (`init` in JS).
    fn init(&mut self, ctx: &mut ModuleContext) {
        let _ = ctx;
    }

    /// Called on canvas resize (`resize` in JS).
    fn resize(&mut self, ctx: &mut ModuleContext) {
        let _ = ctx;
    }

    /// Called once per frame before `draw` (`update` in JS). May mutate `self`
    /// and ignores the returned data of the JS version.
    fn update(&mut self, ctx: &mut ModuleContext) {
        let _ = ctx;
    }

    /// Draw one frame through the canvas facade.
    fn draw(&mut self, ctx: &mut DrawContext);

    /// Release per-instance resources.
    fn dispose(&mut self) {}
}

/// Every built-in 2D module, in the plan's table order
/// (`docs/rust-hybrid.md` WS-5).
pub fn all_modules() -> Vec<Box<dyn TwoDModule>> {
    vec![
        Box::new(Ball::default()),
        Box::new(Text::default()),
        Box::new(Polygon::default()),
        Box::new(Texture2d::default()),
        Box::new(Buffer::default()),
        Box::new(Concentrics::default()),
        Box::new(XDrips::default()),
        Box::new(Line::default()),
        Box::new(Pixelate::default()),
        Box::new(Waveform::default()),
        Box::new(GridStretch::default()),
        Box::new(Bar::default()),
        Box::new(Webcam::default()),
        Box::new(Smear::default()),
        Box::new(Counter::default()),
    ]
}

/// Build a props object from a module's descriptors (the store's resolved
/// props in production, defaults in tests).
pub fn default_props(props: &[TwoDProp]) -> Value {
    let mut map = Map::new();
    for prop in props {
        map.insert(prop.name.to_owned(), prop.default.clone());
    }
    Value::Object(map)
}

// ---------------------------------------------------------------------------
// Prop readers (the JS destructures `props`; these are the typed equivalents)
// ---------------------------------------------------------------------------

/// Read a number prop, falling back to `default` when absent or not numeric.
pub fn prop_f64(props: &Value, name: &str, default: f64) -> f64 {
    props
        .get(name)
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
        .unwrap_or(default)
}

/// Read an integer prop.
pub fn prop_i64(props: &Value, name: &str, default: i64) -> i64 {
    props
        .get(name)
        .and_then(|value| {
            value
                .as_i64()
                .or_else(|| value.as_f64().map(|number| number.round() as i64))
        })
        .unwrap_or(default)
}

/// Read a boolean prop.
pub fn prop_bool(props: &Value, name: &str, default: bool) -> bool {
    props.get(name).and_then(Value::as_bool).unwrap_or(default)
}

/// Read a string prop.
pub fn prop_str<'a>(props: &'a Value, name: &str, default: &'a str) -> &'a str {
    props.get(name).and_then(Value::as_str).unwrap_or(default)
}

/// Read a two-element numeric prop (`vec2`).
pub fn prop_vec2(props: &Value, name: &str, default: [f32; 2]) -> [f32; 2] {
    let Some(entries) = props.get(name).and_then(Value::as_array) else {
        return default;
    };
    if entries.len() < 2 {
        return default;
    }
    let x = entries[0].as_f64().unwrap_or(default[0] as f64) as f32;
    let y = entries[1].as_f64().unwrap_or(default[1] as f64) as f32;
    [x, y]
}

/// Read a `color` prop whose `r`/`g`/`b` are `0..255` and `a` is `0..1`.
pub fn prop_rgba_255(props: &Value, name: &str, default: [f32; 4]) -> [f32; 4] {
    let Some(colour) = props.get(name) else {
        return default;
    };
    rgba_from_object(colour, 255.0).unwrap_or(default)
}

/// Read a `color` prop whose components are all `0..1` (Text/Bar use this).
pub fn prop_rgba_unit(props: &Value, name: &str, default: [f32; 4]) -> [f32; 4] {
    let Some(colour) = props.get(name) else {
        return default;
    };
    rgba_from_object(colour, 1.0).unwrap_or(default)
}

fn rgba_from_object(value: &Value, rgb_scale: f32) -> Option<[f32; 4]> {
    let object = value.as_object()?;
    let r = object.get("r")?.as_f64()? as f32 / rgb_scale;
    let g = object.get("g")?.as_f64()? as f32 / rgb_scale;
    let b = object.get("b")?.as_f64()? as f32 / rgb_scale;
    let a = object.get("a").and_then(Value::as_f64).unwrap_or(1.0) as f32;
    Some([r, g, b, a])
}

/// Read a `tween` prop's resolved colour (`props.color.value` in JS).
///
/// The engine resolves the tween and stores `[r, g, b]` (0..255) under
/// `value`. A raw control object (`{ data: [[r, g, b], …] }`) falls back to
/// the first palette entry so the module renders standalone.
pub fn prop_tween_rgb(props: &Value, name: &str, default: [f32; 4], time_ms: f32) -> [f32; 4] {
    let Some(control) = props.get(name) else {
        return default;
    };
    if let Some(value) = control.get("value") {
        if let Some(rgba) = rgb_triplet(value) {
            return rgba;
        }
    }
    // Tween props whose value has not been pushed by the (JS) tween engine yet:
    // advance through the palette ring on the frame clock. The JS tween is a
    // time-driven anime.js animation over `duration`; interpolating the ring
    // linearly reproduces its visible colour progression.
    let Some(entries) = control.get("data").and_then(Value::as_array) else {
        return default;
    };
    let palette: Vec<[f32; 4]> = entries.iter().filter_map(rgb_triplet).collect();
    if palette.is_empty() {
        return default;
    }
    if palette.len() == 1 {
        return palette[0];
    }
    let duration = control
        .get("duration")
        .and_then(Value::as_f64)
        .unwrap_or(1000.0)
        .max(1.0) as f32;
    let progress = ((time_ms % duration) / duration).clamp(0.0, 1.0);
    let scaled = progress * palette.len() as f32;
    let index = (scaled.floor() as usize) % palette.len();
    let next = (index + 1) % palette.len();
    let t = scaled.fract();
    let lerp = |a: f32, b: f32| a + (b - a) * t;
    [
        lerp(palette[index][0], palette[next][0]),
        lerp(palette[index][1], palette[next][1]),
        lerp(palette[index][2], palette[next][2]),
        lerp(palette[index][3], palette[next][3]),
    ]
}

fn rgb_triplet(value: &Value) -> Option<[f32; 4]> {
    if let Some(entries) = value.as_array() {
        if entries.len() >= 3 {
            let r = entries[0].as_f64()? as f32 / 255.0;
            let g = entries[1].as_f64()? as f32 / 255.0;
            let b = entries[2].as_f64()? as f32 / 255.0;
            let a = entries.get(3).and_then(Value::as_f64).unwrap_or(1.0) as f32;
            return Some([r, g, b, a]);
        }
        return None;
    }
    rgba_from_object(value, 255.0)
}

/// CSS `hsl()` → straight-alpha RGBA, used by `Concentrics.js:143-149`.
pub fn hsl_to_rgba(hue: f32, saturation: f32, lightness: f32) -> [f32; 4] {
    let hue = hue.rem_euclid(360.0) / 360.0;
    let saturation = saturation.clamp(0.0, 1.0);
    let lightness = lightness.clamp(0.0, 1.0);
    if saturation == 0.0 {
        return [lightness, lightness, lightness, 1.0];
    }
    let q = if lightness < 0.5 {
        lightness * (1.0 + saturation)
    } else {
        lightness + saturation - lightness * saturation
    };
    let p = 2.0 * lightness - q;
    let channel = |t: f32| {
        let t = t.rem_euclid(1.0);
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    [
        channel(hue + 1.0 / 3.0),
        channel(hue),
        channel(hue - 1.0 / 3.0),
        1.0,
    ]
}

/// Parse `#rgb`/`#rrggbb` (the `X-Drips` palette is hex strings).
pub fn hex_to_rgba(hex: &str) -> [f32; 4] {
    let value = hex.trim().trim_start_matches('#');
    let parse = |slice: &str| u8::from_str_radix(slice, 16).ok();
    match value.len() {
        3 => {
            let nibble = |index: usize| {
                value[index..index + 1]
                    .chars()
                    .next()
                    .and_then(|c| c.to_digit(16))
                    .map(|digit| (digit * 17) as u8)
            };
            match (nibble(0), nibble(1), nibble(2)) {
                (Some(r), Some(g), Some(b)) => {
                    [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0]
                }
                _ => [0.0, 0.0, 0.0, 1.0],
            }
        }
        6 => {
            let r = parse(&value[0..2]);
            let g = parse(&value[2..4]);
            let b = parse(&value[4..6]);
            match (r, g, b) {
                (Some(r), Some(g), Some(b)) => {
                    [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0]
                }
                _ => [0.0, 0.0, 0.0, 1.0],
            }
        }
        _ => [0.0, 0.0, 0.0, 1.0],
    }
}

/// Deterministic xorshift RNG used where the JS calls `Math.random()`
/// (`Ball.setupBalls`, `Concentrics.circlesObjectFactory`,
/// `X-Drips.generatePattern`, `Line.init`).
///
/// Each module instance seeds from a process-wide counter, so two instances
/// of the same module do not look identical (the JS behaviour) while tests
/// remain reproducible for a given construction order.
#[derive(Clone, Debug)]
pub struct SimpleRng {
    state: u64,
}

impl SimpleRng {
    /// Create an RNG seeded from the process-wide instance counter.
    pub fn new() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0x9E37_79B9_7F4A_7C15);
        let seed = COUNTER.fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed);
        Self::seeded(seed)
    }

    /// Create an RNG with an explicit seed (tests).
    pub fn seeded(seed: u64) -> Self {
        Self {
            state: if seed == 0 {
                0x2545_F491_4F6C_DD1D
            } else {
                seed
            },
        }
    }

    /// Next `f64` in `0..1`, matching `Math.random`.
    pub fn next_f64(&mut self) -> f64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        (x >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Next `f32` in `0..1`.
    pub fn next_f32(&mut self) -> f32 {
        self.next_f64() as f32
    }

    /// `Math.floor(Math.random() * count)` in `usize` form.
    pub fn index(&mut self, count: usize) -> usize {
        if count == 0 {
            return 0;
        }
        ((self.next_f64() * count as f64).floor() as usize).min(count - 1)
    }

    /// `Math.round(Math.random())` (0 or 1).
    pub fn coin(&mut self) -> bool {
        self.next_f64() >= 0.5
    }
}

impl Default for SimpleRng {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rng_is_in_range_and_deterministic() {
        let mut first = SimpleRng::seeded(42);
        let mut second = SimpleRng::seeded(42);
        for _ in 0..100 {
            let a = first.next_f32();
            let b = second.next_f32();
            assert_eq!(a, b);
            assert!((0.0..1.0).contains(&a));
        }
    }

    #[test]
    fn hex_colours_parse() {
        assert_eq!(hex_to_rgba("#ff0000")[0], 1.0);
        assert_eq!(hex_to_rgba("#f00"), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(hex_to_rgba("nope"), [0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn hsl_primaries() {
        let red = hsl_to_rgba(0.0, 1.0, 0.5);
        assert!((red[0] - 1.0).abs() < 1e-5 && red[1] < 1e-5 && red[2] < 1e-5);
        let green = hsl_to_rgba(120.0, 1.0, 0.5);
        assert!((green[1] - 1.0).abs() < 1e-5);
        let blue = hsl_to_rgba(240.0, 1.0, 0.5);
        assert!((blue[2] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn tween_prop_reads_value_then_first_palette_entry() {
        let resolved = json!({ "color": { "value": [255.0, 0.0, 0.0] } });
        assert_eq!(
            prop_tween_rgb(&resolved, "color", [0.0; 4], 0.0),
            [1.0, 0.0, 0.0, 1.0]
        );
        let raw = json!({ "color": { "data": [[0, 0, 255], [255, 255, 255]], "duration": 10 } });
        assert_eq!(
            prop_tween_rgb(&raw, "color", [1.0; 4], 0.0),
            [0.0, 0.0, 1.0, 1.0]
        );
    }

    #[test]
    fn colour_props_scale_255_or_unit() {
        let value = json!({ "c255": { "r": 255, "g": 0, "b": 0, "a": 1 },
                            "cunit": { "r": 1, "g": 0, "b": 0, "a": 1 } });
        assert_eq!(
            prop_rgba_255(&value, "c255", [0.0; 4]),
            [1.0, 0.0, 0.0, 1.0]
        );
        assert_eq!(
            prop_rgba_unit(&value, "cunit", [0.0; 4]),
            [1.0, 0.0, 0.0, 1.0]
        );
    }
}
