//! BPM tweens: the anime.js frame pre-renderer and its runtime.
//!
//! Ported from `src/renderer/src/application/worker/store/modules/tweens.js`
//! (264 lines) and the `tween` data type in `store/modules/dataTypes.js:108-128`.
//!
//! A tween is a small descriptor (`id`, `data`, `duration`, `direction`,
//! `easing`, `loop`, `useBpm`, `bpmDivision`, `durationAsTotalTime`, `steps`).
//! The worker pre-renders the whole animation into an array of value frames by
//! calling anime.js `seek()` at every frame boundary ([`build_tween_frames`]),
//! then treats that array as a step sequencer: an Input Link reads the current
//! frame and advances exactly one frame per read (`dataTypes.tween.get`), so a
//! tween advances once per link evaluation, not once per render.
//!
//! The automatic Scene links (`scene-link-defaults.js`) create a slow BPM
//! tween (`bpmDivision: 32`, `easeInOutSine`, `alternate`) whose value is read
//! through `tweens.tweens['<input id>'].value[0]`; the engine resolves that
//! location through [`resolve_tween_link`].
//!
//! # Per-frame call order
//!
//! ```
//! use modv_engine::tweens::{TweenSet, resolve_tween_link};
//! use serde_json::json;
//!
//! // Preset / Scene load: `state.tweens` is the descriptor array.
//! let document = json!([{
//!     "id": "input-1",
//!     "data": [[0], [1]],
//!     "easing": "easeInOutSine",
//!     "bpmDivision": 32,
//! }]);
//! let mut tweens = TweenSet::load_from_json(&document, 60.0, 128.0).expect("tweens");
//!
//! // Every frame, before rendering: each tween link read advances its tween
//! // exactly once (`dataTypes.tween.get` semantics).
//! let alpha = resolve_tween_link(&mut tweens, "input-1", 0).expect("link value");
//! assert!((0.0..=1.0).contains(&alpha));
//!
//! // `tweens.advance_all()` is only for tweens no link read this frame —
//! // calling it for "input-1" as well would advance it twice.
//! ```
//!
//! # Fidelity notes
//!
//! The maths is ported line for line, including the parts that look odd:
//!
//! - `newDuration` divides by `data.length` unless `durationAsTotalTime` is
//!   set, so a two-row tween plays `duration / 2` per row.
//! - `framesRequired = Math.round(newDuration / (1000 / fps)) || 1` — the
//!   rounding is JS `Math.round` (half up), and a zero/`NaN` result becomes 1.
//!   A leg is capped at [`MAX_TWEEN_FRAMES`] so a corrupt document cannot
//!   exhaust memory (the JS would allocate whatever it asked for).
//! - A looped tween records frames `0..=framesRequired` and then `-1` back to
//!   `1` (the ping-pong is done by the frame recorder, not by anime's
//!   `direction`), giving `2 × framesRequired` frames per cycle.
//! - Each channel is interpolated between the first and the last `data` row.
//!   anime treats three or more rows as keyframes; that is **not** ported
//!   because every tween modV creates (`[[0],[1]]`) has exactly two rows.
//! - anime applies the easing at time 0 during `reset()`, so frame 0 is
//!   `ease(0)` (not the raw start value) — this is observable with `steps()`,
//!   where frame 0 is `1/steps` of the way rather than 0.
//! - `direction: "reverse"` is effectively a no-op in anime 3.2: `seek()`
//!   applies the reversed-time adjustment twice, which cancels for every
//!   sample except the very first one (frame 0 keeps the reset value, which is
//!   `ease(1)` instead of `ease(0)`). The legacy worker records exactly that,
//!   so [`build_tween_frames`] reproduces it. `alternate` behaves like
//!   `normal` while recording; the ping-pong comes from the loop recorder.
//!
//! [`TweenDescriptor`] keeps unknown keys verbatim so a load → save round trip
//! is lossless. Gallery tweens (`isGallery: true`) stay in the document; the
//! preset serializer decides whether to keep them (`createPresetData` filters
//! them in JS), exactly as before — nothing here changes serialization
//! semantics.
//!
//! The reference numbers in `crates/modv-engine/data/tween-reference.json`
//! were produced by anime.js 3.2.2 itself (see the tests); every easing and
//! frame is compared against them.
//!
//! Line numbers refer to the tree at commit `8a512b6e` (branch `rust-hybrid`).

use indexmap::IndexMap;
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

use crate::engine_state::serialize_minimal_f64;

/// Upper bound on recorded frames per animation leg.
///
/// The JS would allocate whatever the duration asks for, which lets a corrupt
/// document (a huge `duration` with a bogus `fps`) exhaust memory. Real tweens
/// are well under a thousand frames per leg, so the cap only ever affects
/// pathological input.
pub const MAX_TWEEN_FRAMES: usize = 100_000;

/// The anime.js version the reference fixture was generated with.
pub const ANIME_REFERENCE_VERSION: &str = "3.2.2";

// ————————————————— descriptors —————————————————

/// Which known keys were present in the document.
///
/// JS `JSON.stringify` omits `undefined`, and a tween the engine did not
/// create itself may legitimately be partial; keeping the presence flags lets
/// a load → save round trip emit exactly the keys the document had. Tweens
/// created by [`TweenDescriptor::new`] carry every key, like `createTween`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Presence {
    data: bool,
    duration: bool,
    direction: bool,
    easing: bool,
    looping: bool,
    use_bpm: bool,
    bpm_division: bool,
    duration_as_total_time: bool,
    steps: bool,
}

impl Presence {
    /// Every known key (what `createTween` writes).
    const ALL: Presence = Presence {
        data: true,
        duration: true,
        direction: true,
        easing: true,
        looping: true,
        use_bpm: true,
        bpm_division: true,
        duration_as_total_time: true,
        steps: true,
    };
}

/// A tween descriptor, as it appears in `state.tweens` (a preset array).
///
/// Field defaults follow `tweens.js` `createTween` (the destructuring
/// defaults), so a partial document behaves like the JS loader does. Keys the
/// engine does not know (including `frames` from the live worker store) are
/// kept verbatim in [`TweenDescriptor::extra`].
#[derive(Clone, Debug, PartialEq)]
pub struct TweenDescriptor {
    /// Tween id (the id of the Input Link that reads it).
    pub id: String,
    /// Keyframe rows; channel `i` of the tween is `data[0][i] .. data[last][i]`.
    pub data: Vec<Vec<f64>>,
    /// Duration in milliseconds (before the `data.length` division).
    pub duration: f64,
    /// `normal`, `reverse` or `alternate` (see the module docs).
    pub direction: String,
    /// anime.js easing name (`easeInOutSine`, `linear`, `steps(4)`, ...).
    pub easing: String,
    /// Whether the recorder ping-pongs the frames (`loop: true`).
    pub looping: bool,
    /// Whether the duration follows the tempo (`useBpm: true`).
    pub use_bpm: bool,
    /// BPM subdivision the tween advances on.
    pub bpm_division: f64,
    /// Treat `duration` as the total time instead of per-row time.
    pub duration_as_total_time: bool,
    /// `steps(n)` quantisation, or 0 (default) for the eased interpolation.
    pub steps: f64,
    /// `isGallery` (absent means "not a gallery tween", like `undefined`).
    pub is_gallery: Option<bool>,
    /// Unknown keys, kept verbatim for round-trip fidelity.
    pub extra: Map<String, Value>,
    /// Which known keys the document carried.
    presence: Presence,
}

impl TweenDescriptor {
    /// A descriptor with the `createTween` defaults, every key present.
    ///
    /// Defaults (`tweens.js:147-163`): `duration: 1000`,
    /// `direction: "alternate"`, `easing: "linear"`, `loop: true`,
    /// `useBpm: true`, `bpmDivision: 16`, `durationAsTotalTime: false`,
    /// `steps: 0`.
    pub fn new(id: impl Into<String>, data: Vec<Vec<f64>>) -> Self {
        Self {
            id: id.into(),
            data,
            duration: 1000.0,
            direction: "alternate".to_string(),
            easing: "linear".to_string(),
            looping: true,
            use_bpm: true,
            bpm_division: 16.0,
            duration_as_total_time: false,
            steps: 0.0,
            is_gallery: None,
            extra: Map::new(),
            presence: Presence::ALL,
        }
    }

    /// Number of animated channels (the widest `data` row).
    pub fn channel_count(&self) -> usize {
        self.data.iter().map(Vec::len).max().unwrap_or(0)
    }

    /// The effective duration of one animation leg, in milliseconds.
    ///
    /// `buildFrames` (`tweens.js:43-54`): BPM tweens compute
    /// `((60 * 60) / bpm) * bpmDivision`, everything else uses `duration`;
    /// both divide by `data.length` unless `durationAsTotalTime` is set. A
    /// falsy BPM (`0`, `NaN`) falls back to 120 like
    /// `store.state.beats.bpm || 120`.
    pub fn effective_duration(&self, bpm: f64) -> f64 {
        let base = if self.use_bpm {
            let bpm = if bpm == 0.0 || bpm.is_nan() {
                120.0
            } else {
                bpm
            };
            ((60.0 * 60.0) / bpm) * self.bpm_division
        } else {
            self.duration
        };

        if self.duration_as_total_time {
            base
        } else {
            base / self.data.len() as f64
        }
    }

    /// Serialize back to the preset shape.
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or_else(|_| Value::Object(Map::new()))
    }
}

/// `f64` serialized the way `JSON.stringify` writes it (see
/// [`crate::engine_state::serialize_minimal_f64`]).
struct MinimalF64(f64);

impl Serialize for MinimalF64 {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serialize_minimal_f64(&self.0, serializer)
    }
}

/// `Vec<Vec<f64>>` serialized with minimal numeric formatting.
struct MinimalF64Grid<'a>(&'a [Vec<f64>]);

impl Serialize for MinimalF64Grid<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;
        let mut sequence = serializer.serialize_seq(Some(self.0.len()))?;
        for row in self.0 {
            sequence.serialize_element(&MinimalF64Row(row))?;
        }
        sequence.end()
    }
}

/// One `data` row with minimal numeric formatting.
struct MinimalF64Row<'a>(&'a [f64]);

impl Serialize for MinimalF64Row<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;
        let mut sequence = serializer.serialize_seq(Some(self.0.len()))?;
        for value in self.0 {
            sequence.serialize_element(&MinimalF64(*value))?;
        }
        sequence.end()
    }
}

impl Serialize for TweenDescriptor {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("id", &self.id)?;
        if self.presence.data {
            map.serialize_entry("data", &MinimalF64Grid(&self.data))?;
        }
        if self.presence.duration {
            map.serialize_entry("duration", &MinimalF64(self.duration))?;
        }
        if self.presence.direction {
            map.serialize_entry("direction", &self.direction)?;
        }
        if self.presence.easing {
            map.serialize_entry("easing", &self.easing)?;
        }
        if self.presence.looping {
            map.serialize_entry("loop", &self.looping)?;
        }
        if self.presence.use_bpm {
            map.serialize_entry("useBpm", &self.use_bpm)?;
        }
        if self.presence.bpm_division {
            map.serialize_entry("bpmDivision", &MinimalF64(self.bpm_division))?;
        }
        if self.presence.duration_as_total_time {
            map.serialize_entry("durationAsTotalTime", &self.duration_as_total_time)?;
        }
        if let Some(is_gallery) = self.is_gallery {
            map.serialize_entry("isGallery", &is_gallery)?;
        }
        if self.presence.steps {
            map.serialize_entry("steps", &MinimalF64(self.steps))?;
        }
        for (key, value) in &self.extra {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

/// Raw shape for deserialization: every known key is optional so the presence
/// flags (and therefore the emitted key set) survive a round trip.
#[derive(Deserialize)]
struct TweenDescriptorRepr {
    id: String,
    #[serde(default)]
    data: Option<Vec<Vec<f64>>>,
    #[serde(default)]
    duration: Option<f64>,
    #[serde(default)]
    direction: Option<String>,
    #[serde(default)]
    easing: Option<String>,
    #[serde(default, rename = "loop")]
    looping: Option<bool>,
    #[serde(default, rename = "useBpm")]
    use_bpm: Option<bool>,
    #[serde(default, rename = "bpmDivision")]
    bpm_division: Option<f64>,
    #[serde(default, rename = "durationAsTotalTime")]
    duration_as_total_time: Option<bool>,
    #[serde(default)]
    steps: Option<f64>,
    #[serde(default, rename = "isGallery")]
    is_gallery: Option<bool>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

impl<'de> Deserialize<'de> for TweenDescriptor {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let repr = TweenDescriptorRepr::deserialize(deserializer)?;
        let presence = Presence {
            data: repr.data.is_some(),
            duration: repr.duration.is_some(),
            direction: repr.direction.is_some(),
            easing: repr.easing.is_some(),
            looping: repr.looping.is_some(),
            use_bpm: repr.use_bpm.is_some(),
            bpm_division: repr.bpm_division.is_some(),
            duration_as_total_time: repr.duration_as_total_time.is_some(),
            steps: repr.steps.is_some(),
        };

        Ok(TweenDescriptor {
            id: repr.id,
            data: repr.data.unwrap_or_default(),
            duration: repr.duration.unwrap_or(1000.0),
            direction: repr.direction.unwrap_or_else(|| "alternate".to_string()),
            easing: repr.easing.unwrap_or_else(|| "linear".to_string()),
            looping: repr.looping.unwrap_or(true),
            use_bpm: repr.use_bpm.unwrap_or(true),
            bpm_division: repr.bpm_division.unwrap_or(16.0),
            duration_as_total_time: repr.duration_as_total_time.unwrap_or(false),
            steps: repr.steps.unwrap_or(0.0),
            is_gallery: repr.is_gallery,
            extra: repr.extra,
            presence,
        })
    }
}

// ————————————————— easings —————————————————

/// The anime.js `penner` easing catalogue, in `Object.keys` order.
///
/// The store exposes this list to the UI (`tweens.js:18-24`), which builds the
/// labels by splitting camel case, so the order is part of the port.
pub const PENNER_EASING_NAMES: &[&str] = &[
    "linear",
    "easeInSine",
    "easeOutSine",
    "easeInOutSine",
    "easeOutInSine",
    "easeInExpo",
    "easeOutExpo",
    "easeInOutExpo",
    "easeOutInExpo",
    "easeInCirc",
    "easeOutCirc",
    "easeInOutCirc",
    "easeOutInCirc",
    "easeInBack",
    "easeOutBack",
    "easeInOutBack",
    "easeOutInBack",
    "easeInBounce",
    "easeOutBounce",
    "easeInOutBounce",
    "easeOutInBounce",
    "easeInElastic",
    "easeOutElastic",
    "easeInOutElastic",
    "easeOutInElastic",
    "easeInQuad",
    "easeOutQuad",
    "easeInOutQuad",
    "easeOutInQuad",
    "easeInCubic",
    "easeOutCubic",
    "easeInOutCubic",
    "easeOutInCubic",
    "easeInQuart",
    "easeOutQuart",
    "easeInOutQuart",
    "easeOutInQuart",
    "easeInQuint",
    "easeOutQuint",
    "easeInOutQuint",
    "easeOutInQuint",
];

/// `minMax` (`anime.es.js:34`).
fn min_max(value: f64, min: f64, max: f64) -> f64 {
    value.max(min).min(max)
}

/// anime's `steps(n)` easing (`anime.es.js:133-137`): progress is quantised
/// upwards to `1/n` steps, with the CSS `1e-6` lower clamp.
fn steps_ease(t: f64, steps: f64) -> f64 {
    (min_max(t, 0.000_001, 1.0) * steps).ceil() * (1.0 / steps)
}

/// The anime.js "ease in" half of a Penner family (`anime.es.js:227-242`).
fn ease_in_family(family: &str, t: f64) -> Option<f64> {
    match family {
        "Sine" => Some(1.0 - (t * std::f64::consts::FRAC_PI_2).cos()),
        "Expo" => Some(if t == 0.0 {
            0.0
        } else {
            2f64.powf(10.0 * t - 10.0)
        }),
        "Circ" => Some(1.0 - (1.0 - t * t).sqrt()),
        "Back" => Some(t * t * (3.0 * t - 2.0)),
        "Bounce" => Some(ease_in_bounce(t)),
        "Elastic" => Some(ease_in_elastic(t)),
        "Quad" => Some(t * t),
        "Cubic" => Some(t * t * t),
        "Quart" => Some(t * t * t * t),
        "Quint" => Some(t * t * t * t * t),
        _ => None,
    }
}

/// `easeInBounce` (`anime.es.js:233-236`).
fn ease_in_bounce(t: f64) -> f64 {
    // `while (t < ((pow2 = 2^--b) - 1) / 11) {}` — b ends on the first value
    // (3, 2, 1 or 0) whose threshold is at or below `t`.
    let mut b: i32 = 4;
    let mut pow2: f64;
    loop {
        b -= 1;
        pow2 = 2f64.powi(b);
        if t >= (pow2 - 1.0) / 11.0 {
            break;
        }
    }
    1.0 / 4f64.powi(3 - b) - 7.5625 * ((pow2 * 3.0 - 2.0) / 22.0 - t).powi(2)
}

/// `easeInElastic` with anime's default parameters (`amplitude = 1`,
/// `period = 0.5`), `anime.es.js:238-247`.
fn ease_in_elastic(t: f64) -> f64 {
    if t == 0.0 || t == 1.0 {
        return t;
    }
    let amplitude: f64 = 1.0;
    let period: f64 = 0.5;
    let offset = period / (std::f64::consts::TAU) * (1.0 / amplitude).asin();
    -amplitude
        * 2f64.powf(10.0 * (t - 1.0))
        * (((t - 1.0) - offset) * std::f64::consts::TAU / period).sin()
}

/// The four directions anime derives a Penner family into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EaseForm {
    /// `easeIn<Family>`.
    In,
    /// `easeOut<Family>`.
    Out,
    /// `easeInOut<Family>`.
    InOut,
    /// `easeOutIn<Family>`.
    OutIn,
}

/// Evaluate an anime.js easing name at progress `t`.
///
/// Covers the whole `penner` catalogue ([`PENNER_EASING_NAMES`]) plus
/// parameterised `steps(n)` strings. Unknown names fall back to `linear`
/// (`t`) — the JS would throw here, but a preset from a newer build must not
/// take the engine down, and `linear` is the value anime uses for a missing
/// easing (`tweens.js:89`).
pub fn ease(name: &str, t: f64) -> f64 {
    if let Some(argument) = name
        .strip_prefix("steps(")
        .and_then(|rest| rest.strip_suffix(')'))
    {
        // `applyArguments(steps, args)` with no argument defaults to 10.
        return match argument.trim().parse::<f64>() {
            Ok(steps) => steps_ease(t, steps),
            Err(_) => steps_ease(t, 10.0),
        };
    }

    if name == "linear" {
        return t;
    }

    // `easeOutIn` / `easeInOut` are longer prefixes than `easeIn`/`easeOut`.
    let (form, family) = if let Some(family) = name.strip_prefix("easeInOut") {
        (EaseForm::InOut, family)
    } else if let Some(family) = name.strip_prefix("easeOutIn") {
        (EaseForm::OutIn, family)
    } else if let Some(family) = name.strip_prefix("easeIn") {
        (EaseForm::In, family)
    } else if let Some(family) = name.strip_prefix("easeOut") {
        (EaseForm::Out, family)
    } else {
        return t;
    };

    // Unknown families (and unknown names) fall back to linear.
    let ease_in = |value: f64| match ease_in_family(family, value) {
        Some(eased) => eased,
        None => value,
    };

    if ease_in_family(family, 0.0).is_none() {
        return t;
    }

    match form {
        EaseForm::In => ease_in(t),
        EaseForm::Out => 1.0 - ease_in(1.0 - t),
        EaseForm::InOut => {
            if t < 0.5 {
                ease_in(t * 2.0) / 2.0
            } else {
                1.0 - ease_in(t * -2.0 + 2.0) / 2.0
            }
        }
        EaseForm::OutIn => {
            if t < 0.5 {
                (1.0 - ease_in(1.0 - t * 2.0)) / 2.0
            } else {
                (ease_in(t * 2.0 - 1.0) + 1.0) / 2.0
            }
        }
    }
}

// ————————————————— frame pre-renderer —————————————————

/// JS `Math.round`: ties go up, and non-finite input stays non-finite.
fn js_round(value: f64) -> f64 {
    if value.is_finite() {
        (value + 0.5).floor()
    } else {
        value
    }
}

/// `Math.round(newDuration / (1000 / fps)) || 1`, clamped to
/// [`MAX_TWEEN_FRAMES`].
fn frames_required(new_duration: f64, fps: f64) -> usize {
    let required = js_round(new_duration / (1000.0 / fps));

    if !required.is_finite() || required < 1.0 {
        return 1;
    }

    (required.min(MAX_TWEEN_FRAMES as f64)) as usize
}

/// The start/end value of every channel.
struct Channels {
    from: Vec<f64>,
    to: Vec<f64>,
}

impl Channels {
    /// Transpose the `data` rows into per-channel start/end values.
    ///
    /// `tweens.js:68-83` builds `mapped[index]` from the value at `index` of
    /// every row; anime then animates a two-entry array from its first to its
    /// last element. Rows are rectangular in every modV tween, so channel `i`
    /// runs from `data[0][i]` to `data[last][i]`.
    fn from_data(data: &[Vec<f64>]) -> Self {
        let channel_count = data.iter().map(Vec::len).max().unwrap_or(0);
        let mut from = Vec::with_capacity(channel_count);
        let mut to = Vec::with_capacity(channel_count);

        for channel in 0..channel_count {
            let values: Vec<f64> = data
                .iter()
                .filter_map(|row| row.get(channel).copied())
                .collect();

            from.push(values.first().copied().unwrap_or(0.0));
            to.push(values.last().copied().unwrap_or(0.0));
        }

        Self { from, to }
    }

    /// The channel values at a share `progress` (0..1) of the animation.
    ///
    /// This is anime's `setAnimationsProgress`: `elapsed` is clamped to
    /// `0..1`, a `NaN` elapsed (zero duration) counts as complete, and each
    /// channel is `from + eased * (to - from)`.
    fn sample(&self, tween: &TweenDescriptor, progress: f64) -> Vec<f64> {
        let progress = if progress.is_nan() {
            1.0
        } else {
            progress.clamp(0.0, 1.0)
        };

        let eased = if tween.steps != 0.0 && !tween.steps.is_nan() {
            steps_ease(progress, tween.steps)
        } else {
            ease(&tween.easing, progress)
        };

        self.from
            .iter()
            .zip(&self.to)
            .map(|(from, to)| from + eased * (to - from))
            .collect()
    }
}

/// Pre-render the tween's frames, starting at the beginning.
///
/// A line-for-line port of `buildFrames` (`tweens.js:31-133`) without the
/// `anime` dependency. `fps` is the store's target frame rate and `bpm` the
/// current tempo (`0`/`NaN` falls back to 120 for BPM tweens).
///
/// The returned vector holds one frame per stored value row:
///
/// - `loop`: `2 × framesRequired` frames — `0..=framesRequired` then
///   `framesRequired - 1 ..= 1` (frame 0 is not repeated at the end).
/// - no loop: `framesRequired` frames, `0..framesRequired`.
///
/// A tween without `data` has no channels and cannot animate (the JS would
/// divide by zero and loop forever), so it returns a single empty frame.
pub fn build_tween_frames(tween: &TweenDescriptor, fps: f64, bpm: f64) -> Vec<Vec<f64>> {
    build_frames_with_seek(tween, fps, bpm, 0.0).0
}

/// [`build_tween_frames`] plus the `seek` position restore.
///
/// Returns the new frames and the frame index that preserves `seek`
/// (`frame = Math.floor(newFrames.len() * seek)`, `tweens.js:129`).
fn build_frames_with_seek(
    tween: &TweenDescriptor,
    fps: f64,
    bpm: f64,
    seek: f64,
) -> (Vec<Vec<f64>>, usize) {
    if tween.data.is_empty() {
        return (vec![Vec::new()], 0);
    }

    let new_duration = tween.effective_duration(bpm);
    let frames_required = frames_required(new_duration, fps);
    let channels = Channels::from_data(&tween.data);

    // anime computes `elapsed = clamp(insTime, 0, duration) / duration`, so a
    // zero duration yields `NaN`, which anime treats as "complete" (the end
    // value).
    let progress_of = |time: f64| {
        if new_duration == 0.0 {
            f64::NAN
        } else {
            time / new_duration
        }
    };

    // The value anime installs during `reset()`, before the first `seek(0)`
    // runs (which does not update the target): `ease(0)` normally, `ease(1)`
    // when the instance was created reversed.
    let reset_frame = channels.sample(
        tween,
        progress_of(if tween.direction == "reverse" {
            new_duration
        } else {
            0.0
        }),
    );

    let sample = |frame: usize| {
        if frame == 0 {
            reset_frame.clone()
        } else {
            let time = (frame as f64 / frames_required as f64) * new_duration;
            channels.sample(tween, progress_of(time))
        }
    };

    if !tween.looping {
        let mut frames = Vec::with_capacity(frames_required);
        frames.push(reset_frame.clone());
        for frame in 1..frames_required {
            frames.push(sample(frame));
        }
        return (frames, 0);
    }

    let mut frames = Vec::with_capacity(frames_required * 2);
    let mut frame = 0usize;
    let mut step: i64 = 1;

    loop {
        frames.push(sample(frame));

        if frame == frames_required {
            step = -1;
        }

        let next = frame as i64 + step;
        frame = next.max(0) as usize;

        if frame == 0 {
            break;
        }
    }

    let restored = restored_frame(frames.len(), seek);
    (frames, restored)
}

/// `Math.floor(len * seek)`, clamped into the frame vector.
fn restored_frame(len: usize, seek: f64) -> usize {
    if len == 0 {
        return 0;
    }

    let restored = (len as f64 * seek).floor();

    if !restored.is_finite() || restored < 0.0 {
        return 0;
    }

    (restored.min((len - 1) as f64)) as usize
}

// ————————————————— runtime —————————————————

/// A live tween: its descriptor, the pre-rendered frames and the play head.
///
/// Mirrors the pair `state.tweens.tweens[id]` (descriptor + `frames`) and the
/// module-level `frames[id]` counter in `tweens.js:13,231-254`.
#[derive(Clone, Debug)]
pub struct TweenRuntime {
    descriptor: TweenDescriptor,
    frames: Vec<Vec<f64>>,
    frame: usize,
    fps: f64,
    bpm: f64,
}

impl TweenRuntime {
    /// Build the frames for `descriptor` at the current `fps`/`bpm`.
    pub fn new(descriptor: TweenDescriptor, fps: f64, bpm: f64) -> Self {
        let (frames, frame) = build_frames_with_seek(&descriptor, fps, bpm, 0.0);

        Self {
            descriptor,
            frames,
            frame,
            fps,
            bpm,
        }
    }

    /// The descriptor (including unknown keys).
    pub fn descriptor(&self) -> &TweenDescriptor {
        &self.descriptor
    }

    /// The descriptor, mutably. Call [`TweenRuntime::rebuild`] afterwards to
    /// re-render the frames for the edited settings.
    pub fn descriptor_mut(&mut self) -> &mut TweenDescriptor {
        &mut self.descriptor
    }

    /// The tween id.
    pub fn id(&self) -> &str {
        &self.descriptor.id
    }

    /// The pre-rendered frames.
    pub fn frames(&self) -> &[Vec<f64>] {
        &self.frames
    }

    /// The current frame index.
    pub fn frame(&self) -> usize {
        self.frame
    }

    /// The current frame's values without advancing.
    pub fn current_value(&self) -> Option<&[f64]> {
        self.frames.get(self.frame).map(Vec::as_slice)
    }

    /// Move the play head, clamped into the recorded frames.
    pub fn set_frame(&mut self, frame: usize) {
        if self.frames.is_empty() {
            self.frame = 0;
            return;
        }
        self.frame = frame.min(self.frames.len() - 1);
    }

    /// The frame rate the frames were rendered at.
    pub fn fps(&self) -> f64 {
        self.fps
    }

    /// The tempo the frames were rendered at.
    pub fn bpm(&self) -> f64 {
        self.bpm
    }

    /// Advance one frame, wrapping to 0 at the end (`advanceFrame`,
    /// `tweens.js:248-254`).
    pub fn advance(&mut self) {
        if self.frames.is_empty() {
            self.frame = 0;
        } else if self.frame + 1 < self.frames.len() {
            self.frame += 1;
        } else {
            self.frame = 0;
        }
    }

    /// Read the current frame and advance (`dataTypes.tween.get`,
    /// `dataTypes.js:122-127`).
    ///
    /// The advance happens even when there is no frame to read, matching the
    /// JS getter.
    pub fn value(&mut self) -> Option<Vec<f64>> {
        let value = self.frames.get(self.frame).cloned();
        self.advance();
        value
    }

    /// Re-render the frames for a new `fps`/`bpm`, keeping the position.
    ///
    /// The seek fraction is `frame / frames.len()` of the *old* frames, and
    /// the new frame is `floor(newLen * seek)` (`buildFrames`' `seek`,
    /// `tweens.js:56-60,129`).
    pub fn rebuild(&mut self, fps: f64, bpm: f64) {
        let seek = if self.frames.is_empty() {
            0.0
        } else {
            self.frame as f64 / self.frames.len() as f64
        };
        let (frames, frame) = build_frames_with_seek(&self.descriptor, fps, bpm, seek);

        self.frames = frames;
        self.frame = frame;
        self.fps = fps;
        self.bpm = bpm;
    }

    /// Follow a tempo change the way `updateBpm` does
    /// (`tweens.js:185-212`).
    ///
    /// Only `useBpm` tweens are rebuilt. Two legacy quirks are reproduced so a
    /// preset saved by the engine matches the worker:
    ///
    /// - the stored `duration` becomes `calculatedBpm` (or
    ///   `calculatedBpm × data.length`), which is *not* what the frames use;
    ///   a BPM tween's frames are computed from the tempo alone
    ///   ([`TweenDescriptor::effective_duration`]), so the field only becomes
    ///   observable if `useBpm` is later cleared;
    /// - `buildFrames` then divides that duration by `data.length` again, so
    ///   a hand-written descriptor whose `duration` a caller set to the
    ///   "total" time still plays as a two-row tween.
    pub fn update_bpm(&mut self, bpm: f64) {
        if !self.descriptor.use_bpm {
            return;
        }

        let calculated_bpm = ((60.0 * 60.0) / bpm) * self.descriptor.bpm_division;
        self.descriptor.duration = if self.descriptor.duration_as_total_time {
            calculated_bpm
        } else {
            calculated_bpm * self.descriptor.data.len() as f64
        };

        self.rebuild(self.fps, bpm);
    }
}

// ————————————————— set —————————————————

/// A set of live tweens keyed by id, in document order.
///
/// Replaces `state.tweens.tweens` (id-keyed map) plus the module-level
/// `frames`/`progress` maps of the JS store. Iteration order is the order the
/// descriptors appeared in the preset, so [`TweenSet::to_json`] writes back the
/// same array.
///
/// # Advancing
///
/// In the worker a tween advances **when it is read**: `dataTypes.tween.get`
/// calls `advanceFrame` on every read, and one link read is one advance
/// (`dataTypes.js:122-127`). Keep that behaviour by resolving tween links with
/// [`resolve_tween_link`]; use [`TweenSet::advance_all`] only for tweens no
/// link read during the frame (otherwise they advance twice).
#[derive(Clone, Debug, Default)]
pub struct TweenSet {
    tweens: IndexMap<String, TweenRuntime>,
}

/// Errors decoding a tween document.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum TweenError {
    /// `tweens` was neither an array, an id-keyed map, nor a
    /// `{ tweens: ... }` wrapper.
    #[error("tween document is not an array or an id-keyed object")]
    InvalidShape,
    /// A descriptor did not decode.
    #[error("failed to decode tween `{id}`: {message}")]
    Decode {
        /// The descriptor id (or its array index / map key).
        id: String,
        /// The serde error message.
        message: String,
    },
}

impl TweenSet {
    /// An empty set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of tweens.
    pub fn len(&self) -> usize {
        self.tweens.len()
    }

    /// Whether the set is empty.
    pub fn is_empty(&self) -> bool {
        self.tweens.is_empty()
    }

    /// Ids in document order.
    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.tweens.keys().map(String::as_str)
    }

    /// Iterate over `(id, runtime)` in document order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &TweenRuntime)> {
        self.tweens.iter().map(|(id, tween)| (id.as_str(), tween))
    }

    /// Iterate mutably over `(id, runtime)` in document order.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&str, &mut TweenRuntime)> {
        self.tweens
            .iter_mut()
            .map(|(id, tween)| (id.as_str(), tween))
    }

    /// The tween with `id`.
    pub fn get(&self, id: &str) -> Option<&TweenRuntime> {
        self.tweens.get(id)
    }

    /// The tween with `id`, mutably.
    pub fn get_mut(&mut self, id: &str) -> Option<&mut TweenRuntime> {
        self.tweens.get_mut(id)
    }

    /// Whether `id` is present.
    pub fn contains(&self, id: &str) -> bool {
        self.tweens.contains_key(id)
    }

    /// Insert or replace a runtime (`ADD_TWEEN`). The key is the tween id.
    pub fn upsert(&mut self, tween: TweenRuntime) {
        let id = tween.id().to_string();
        self.tweens.insert(id, tween);
    }

    /// Insert a descriptor, rendering its frames (`ADD_TWEEN`).
    pub fn insert_descriptor(&mut self, descriptor: TweenDescriptor, fps: f64, bpm: f64) {
        self.upsert(TweenRuntime::new(descriptor, fps, bpm));
    }

    /// Decode and insert a single descriptor (a `tweens/ADD_TWEEN` payload).
    pub fn upsert_json(
        &mut self,
        value: &Value,
        fps: f64,
        bpm: f64,
    ) -> Result<&mut TweenRuntime, TweenError> {
        let descriptor = decode_descriptor(value, value.get("id").and_then(Value::as_str))
            .map_err(|(id, message)| TweenError::Decode { id, message })?;
        self.insert_descriptor(descriptor, fps, bpm);
        let id = value
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        self.tweens.get_mut(&id).ok_or(TweenError::InvalidShape)
    }

    /// Remove a tween (`REMOVE_TWEEN`). Returns whether it existed.
    pub fn remove(&mut self, id: &str) -> bool {
        self.tweens.shift_remove(id).is_some()
    }

    /// Advance every tween once.
    ///
    /// This is *not* what the worker does on its own — see the type docs and
    /// only call it for tweens no Input Link read this frame.
    pub fn advance_all(&mut self) {
        for tween in self.tweens.values_mut() {
            tween.advance();
        }
    }

    /// Every tween's current frame values, without advancing.
    pub fn values_snapshot(&self) -> IndexMap<String, Vec<f64>> {
        self.tweens
            .iter()
            .filter_map(|(id, tween)| {
                tween
                    .current_value()
                    .map(|value| (id.clone(), value.to_vec()))
            })
            .collect()
    }

    /// Replace the set with the tweens in `value`, rendering frames at
    /// `fps`/`bpm`.
    ///
    /// Accepts the shapes `state.tweens` can have:
    ///
    /// - the preset array of descriptors (`[{ id, data, ... }]`);
    /// - the live worker map (`{ "<id>": { id, data, ... } }`);
    /// - the live worker root (`{ tweens: { "<id>": ... } }`).
    ///
    /// A `null` document clears the set.
    pub fn load_from_json(value: &Value, fps: f64, bpm: f64) -> Result<Self, TweenError> {
        let mut set = Self::new();
        set.replace_from_json(value, fps, bpm)?;
        Ok(set)
    }

    /// [`TweenSet::load_from_json`] into an existing set.
    pub fn replace_from_json(
        &mut self,
        value: &Value,
        fps: f64,
        bpm: f64,
    ) -> Result<(), TweenError> {
        self.tweens.clear();

        let entries = tween_entries(value)?;
        for (key, entry) in entries {
            let descriptor = decode_descriptor(&entry, key.as_deref())
                .map_err(|(id, message)| TweenError::Decode { id, message })?;
            self.insert_descriptor(descriptor, fps, bpm);
        }

        Ok(())
    }

    /// Serialize the set back to the preset array of descriptors.
    pub fn to_json(&self) -> Value {
        Value::Array(
            self.tweens
                .values()
                .map(|tween| tween.descriptor().to_json())
                .collect(),
        )
    }
}

/// Split a tween document into `(key, descriptor)` pairs.
///
/// The key is `None` for array entries (a preset descriptor must carry its
/// own `id`) and `Some(key)` for the live worker map, which is keyed by id.
fn tween_entries(value: &Value) -> Result<Vec<(Option<String>, Value)>, TweenError> {
    match value {
        Value::Null => Ok(Vec::new()),
        Value::Array(entries) => Ok(entries.iter().map(|entry| (None, entry.clone())).collect()),
        Value::Object(map) => {
            // The live worker root is `{ tweens: { ... } }`.
            if let Some(inner) = map.get("tweens") {
                return tween_entries(inner);
            }

            Ok(map
                .iter()
                .map(|(key, entry)| (Some(key.clone()), entry.clone()))
                .collect())
        }
        _ => Err(TweenError::InvalidShape),
    }
}

/// Decode one descriptor, falling back to `key` when the entry has no `id`
/// (the live worker map is keyed by id).
fn decode_descriptor(
    value: &Value,
    key: Option<&str>,
) -> Result<TweenDescriptor, (String, String)> {
    if !value.is_object() {
        return Err((
            key.unwrap_or_default().to_string(),
            "tween descriptor is not an object".to_string(),
        ));
    }

    let mut value = value.clone();
    if value.get("id").and_then(Value::as_str).is_none()
        && let (Some(key), Some(object)) = (key, value.as_object_mut())
    {
        object.insert("id".to_string(), Value::String(key.to_string()));
    }

    let id = value
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    serde_json::from_value(value).map_err(|error| (id, error.to_string()))
}

// ————————————————— app-facing helpers —————————————————

/// Resolve one Input Link read of a tween, mirroring
/// `dataTypes.tween.get` + `value[channel]`.
///
/// The read advances the tween exactly once (even when the channel is
/// missing, like the JS getter), which is what makes a BPM tween breathe at
/// the link evaluation rate.
pub fn resolve_tween_link(tweens: &mut TweenSet, id: &str, channel: usize) -> Option<f64> {
    let frame = tweens.get_mut(id)?.value()?;
    frame.get(channel).copied()
}

/// Split a tween link `location` into `(tween id, channel)`.
///
/// Handles the shape the Scene link builder writes,
/// `tweens.tweens['<input id>'].value[0]`, plus the shorter
/// `tweens['<id>'].value[0]` and unquoted `tweens.tweens[<id>].value[0]`.
/// Anything else is `None` (the link is not a tween link).
pub fn parse_tween_link_location(location: &str) -> Option<(String, usize)> {
    let rest = location
        .strip_prefix("tweens.tweens[")
        .or_else(|| location.strip_prefix("tweens["))?;

    let (id, rest) = match rest.strip_prefix('\'') {
        Some(quoted) => {
            let end = quoted.find('\'')?;
            let id = quoted[..end].to_string();
            let tail = quoted[end + 1..].strip_prefix(']')?;
            (id, tail)
        }
        None => {
            let end = rest.find(']')?;
            (rest[..end].to_string(), &rest[end + 1..])
        }
    };

    let channel = rest.strip_prefix(".value[")?.strip_suffix(']')?;

    Some((id, channel.parse::<usize>().ok()?))
}

#[cfg(test)]
mod tests {
    //! Reference tests against anime.js 3.2.2 itself
    //! (`crates/modv-engine/data/tween-reference.json`, generated by running
    //! `buildFrames` from `store/modules/tweens.js` in Node 24 — see the
    //! module docs) plus the runtime/link semantics from `dataTypes.js`.
    use super::*;
    use crate::scene_link_defaults::default_link_tween;
    use serde_json::json;

    /// The anime.js reference: frame dumps for five tween configs and the
    /// whole `penner` easing catalogue.
    const REFERENCE: &str = include_str!("../data/tween-reference.json");

    fn reference() -> Value {
        serde_json::from_str(REFERENCE).expect("tween reference fixture parses")
    }

    fn descriptor_from(config: &Value) -> TweenDescriptor {
        serde_json::from_value(config.clone()).expect("reference config decodes")
    }

    /// A plain, non-BPM tween at 10 fps (5 frames over 500 ms per row).
    fn linear_tween() -> TweenDescriptor {
        let mut tween = TweenDescriptor::new("t1", vec![vec![0.0], vec![1.0]]);
        tween.duration = 1000.0;
        tween.direction = "normal".to_string();
        tween.easing = "linear".to_string();
        tween.looping = false;
        tween.use_bpm = false;
        tween
    }

    // ————————————————— anime.js reference —————————————————

    #[test]
    fn reference_frames_match_anime() {
        let reference = reference();

        for entry in reference["configs"].as_array().expect("configs array") {
            let config = &entry["config"];
            let id = config["id"].as_str().expect("config id");
            let fps = config["fps"].as_f64().expect("fps");
            let bpm = config["bpm"].as_f64().expect("bpm");

            let frames = build_tween_frames(&descriptor_from(config), fps, bpm);
            let expected = entry["frames"].as_array().expect("frame array");

            assert_eq!(frames.len(), expected.len(), "{id}: frame count");

            for (index, expected_frame) in expected.iter().enumerate() {
                let expected_channels = expected_frame.as_object().expect("frame object");
                assert_eq!(
                    frames[index].len(),
                    expected_channels.len(),
                    "{id}: channel count at frame {index}"
                );

                for (channel, value) in frames[index].iter().enumerate() {
                    let expected_value = expected_channels[&channel.to_string()]
                        .as_f64()
                        .expect("numeric channel");
                    assert!(
                        (value - expected_value).abs() < 1e-9,
                        "{id}: frame {index} channel {channel}: {value} != {expected_value}"
                    );
                }
            }
        }
    }

    #[test]
    fn every_penner_easing_matches_anime() {
        let reference = reference();
        let times: Vec<f64> = reference["easingTimes"]
            .as_array()
            .expect("easing times")
            .iter()
            .map(|time| time.as_f64().expect("numeric time"))
            .collect();
        let easings = reference["easings"].as_object().expect("easings map");

        for (name, values) in easings {
            let values = values.as_array().expect("easing samples");
            assert_eq!(values.len(), times.len(), "{name}: sample count");

            for (time, expected) in times.iter().zip(values) {
                let expected = expected.as_f64().expect("numeric sample");
                let actual = ease(name, *time);
                assert!(
                    (actual - expected).abs() < 1e-12,
                    "{name}({time}): {actual} != {expected}"
                );
            }
        }
    }

    #[test]
    fn penner_easing_names_match_the_catalogue() {
        let reference = reference();
        let fixture: Vec<&str> = reference["easings"]
            .as_object()
            .expect("easings map")
            .keys()
            .map(String::as_str)
            .collect();

        assert_eq!(PENNER_EASING_NAMES.len(), 41);
        let mut ours = PENNER_EASING_NAMES.to_vec();
        let mut theirs = fixture.clone();
        ours.sort_unstable();
        theirs.sort_unstable();
        assert_eq!(ours, theirs);
        // `linear` is first in anime's `Object.keys` order.
        assert_eq!(PENNER_EASING_NAMES[0], "linear");
        assert_eq!(ANIME_REFERENCE_VERSION, "3.2.2");
    }

    #[test]
    fn unknown_easings_fall_back_to_linear() {
        assert_eq!(ease("notAnEasing", 0.25), 0.25);
        assert_eq!(ease("", 0.75), 0.75);
        // `spring` is a separate anime case the penner catalogue does not own.
        assert_eq!(ease("spring(1, 100, 10, 0)", 0.4), 0.4);
    }

    #[test]
    fn steps_easing_quantises_progress() {
        // anime's steps(): ceil(clamp(t, 1e-6, 1) * n) / n.
        assert_eq!(ease("steps(4)", 0.0), 0.25);
        assert_eq!(ease("steps(4)", 0.25), 0.25);
        assert_eq!(ease("steps(4)", 0.3), 0.5);
        assert_eq!(ease("steps(4)", 1.0), 1.0);
        // No argument defaults to 10 steps.
        assert!((ease("steps()", 0.0) - 0.1).abs() < 1e-12);

        // The `steps` descriptor field wins over the easing name, like
        // `easing: steps ? \`steps(${steps})\` : easing` in buildFrames.
        let mut tween = linear_tween();
        tween.easing = "easeInOutSine".to_string();
        tween.steps = 4.0;
        let frames = build_tween_frames(&tween, 10.0, 120.0);
        assert!((frames[0][0] - 0.25).abs() < 1e-12);
        assert!((frames[2][0] - 0.5).abs() < 1e-12);
    }

    // ————————————————— frame recording —————————————————

    #[test]
    fn loop_records_a_ping_pong_and_no_loop_records_one_leg() {
        // duration 1000 / 2 rows = 500 ms; at 10 fps that is 5 frames a leg.
        let mut tween = linear_tween();
        let frames = build_tween_frames(&tween, 10.0, 120.0);
        assert_eq!(frames.len(), 5);
        let values: Vec<f64> = frames.iter().map(|frame| frame[0]).collect();
        assert_eq!(values, vec![0.0, 0.2, 0.4, 0.6, 0.8]);

        tween.looping = true;
        let frames = build_tween_frames(&tween, 10.0, 120.0);
        assert_eq!(frames.len(), 10);
        let values: Vec<f64> = frames.iter().map(|frame| frame[0]).collect();
        // 0..=framesRequired then framesRequired - 1 ..= 1: frame 0 and the
        // top frame appear once.
        assert_eq!(
            values,
            vec![0.0, 0.2, 0.4, 0.6, 0.8, 1.0, 0.8, 0.6, 0.4, 0.2]
        );
        assert_eq!(frames[10 - 1][0], 0.2);
    }

    #[test]
    fn frames_required_rounds_like_javascript() {
        // 1000 ms / 2 rows = 500 ms; 500 / (1000/30) = 14.999...
        let tween = linear_tween();
        let frames = build_tween_frames(&tween, 30.0, 120.0);
        assert_eq!(frames.len(), 15);

        // 500 / (1000/20) = 10.000000000000002 → 10.
        let frames = build_tween_frames(&tween, 20.0, 120.0);
        assert_eq!(frames.len(), 10);

        // Degenerate fps values collapse to a single frame (`|| 1`).
        assert_eq!(build_tween_frames(&tween, 0.0, 120.0).len(), 1);
        assert_eq!(build_tween_frames(&tween, f64::NAN, 120.0).len(), 1);
    }

    #[test]
    fn two_channel_data_interpolates_each_channel() {
        let mut tween = TweenDescriptor::new("two", vec![vec![0.0, 10.0], vec![1.0, 20.0]]);
        tween.duration = 500.0;
        tween.direction = "normal".to_string();
        tween.easing = "linear".to_string();
        tween.looping = false;
        tween.use_bpm = false;

        let frames = build_tween_frames(&tween, 10.0, 120.0);
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[0], vec![0.0, 10.0]);
        assert!((frames[1][0] - 1.0 / 3.0).abs() < 1e-12);
        assert!((frames[1][1] - (10.0 + 10.0 / 3.0)).abs() < 1e-12);
        assert!((frames[2][0] - 2.0 / 3.0).abs() < 1e-12);
    }

    #[test]
    fn empty_data_is_a_single_empty_frame() {
        let mut tween = TweenDescriptor::new("empty", Vec::new());
        tween.use_bpm = false;
        let frames = build_tween_frames(&tween, 60.0, 120.0);
        assert_eq!(frames, vec![Vec::<f64>::new()]);
    }

    #[test]
    fn zero_duration_samples_the_end_value() {
        let mut tween = linear_tween();
        tween.duration = 0.0;
        let frames = build_tween_frames(&tween, 60.0, 120.0);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0], vec![1.0]);
    }

    #[test]
    fn bpm_zero_or_nan_falls_back_to_120() {
        let tween = descriptor_from(&default_link_tween("in-1", None));
        assert_eq!(
            tween.effective_duration(0.0),
            tween.effective_duration(120.0)
        );
        assert_eq!(
            tween.effective_duration(f64::NAN),
            tween.effective_duration(120.0)
        );
        assert_eq!(
            build_tween_frames(&tween, 60.0, 0.0),
            build_tween_frames(&tween, 60.0, 120.0)
        );
    }

    // ————————————————— runtime —————————————————

    #[test]
    fn value_reads_then_advances_and_wraps() {
        let mut runtime = TweenRuntime::new(linear_tween(), 10.0, 120.0);
        assert_eq!(runtime.frames().len(), 5);
        assert_eq!(runtime.frame(), 0);

        let read: Vec<f64> = (0..5).map(|_| runtime.value().expect("frame")[0]).collect();
        assert_eq!(read, vec![0.0, 0.2, 0.4, 0.6, 0.8]);
        // `advanceFrame` wraps to 0 at the last frame.
        assert_eq!(runtime.frame(), 0);
        assert_eq!(runtime.value().expect("frame")[0], 0.0);

        runtime.set_frame(100);
        assert_eq!(runtime.frame(), 4);
        runtime.advance();
        assert_eq!(runtime.frame(), 0);
    }

    #[test]
    fn set_frame_keeps_the_current_value() {
        let mut runtime = TweenRuntime::new(linear_tween(), 10.0, 120.0);
        runtime.set_frame(3);
        assert_eq!(runtime.frame(), 3);
        assert_eq!(runtime.current_value(), Some(&[0.6][..]));
    }

    #[test]
    fn update_bpm_recomputes_duration_and_frames() {
        let mut tween = TweenDescriptor::new("bpm", vec![vec![0.0], vec![1.0]]);
        tween.bpm_division = 32.0;
        let mut runtime = TweenRuntime::new(tween, 60.0, 120.0);

        // `createTween` leaves duration at 1000, but a BPM tween ignores it:
        // ((3600 / 120) * 32) / 2 rows = 480 ms → round(28.8) = 29 a leg.
        assert_eq!(runtime.descriptor().duration, 1000.0);
        assert_eq!(runtime.frames().len(), 58);

        // `updateBpm` stores `calculatedBpm * data.length` (960 * 2) and
        // rebuilds; at the same tempo the frames are unchanged because they
        // follow the tempo, not the stored duration.
        runtime.update_bpm(120.0);
        assert_eq!(runtime.descriptor().duration, 1920.0);
        assert_eq!(runtime.frames().len(), 58);
        assert_eq!(runtime.bpm(), 120.0);

        // 60 BPM doubles the leg: ((3600 / 60) * 32) / 2 = 960 ms → 58 a leg.
        runtime.update_bpm(60.0);
        assert_eq!(runtime.descriptor().duration, 3840.0);
        assert_eq!(runtime.frames().len(), 116);
        assert_eq!(runtime.bpm(), 60.0);
    }

    #[test]
    fn update_bpm_leaves_wall_clock_tweens_alone() {
        let mut runtime = TweenRuntime::new(linear_tween(), 10.0, 120.0);
        let frames = runtime.frames().to_vec();
        runtime.update_bpm(60.0);
        assert_eq!(runtime.descriptor().duration, 1000.0);
        assert_eq!(runtime.frames(), frames.as_slice());
        assert_eq!(runtime.bpm(), 120.0);
    }

    #[test]
    fn rebuild_preserves_the_position_fraction() {
        let mut tween = linear_tween();
        tween.looping = true;
        let mut runtime = TweenRuntime::new(tween, 10.0, 120.0);
        assert_eq!(runtime.frames().len(), 10);

        runtime.set_frame(2);
        let before = runtime.frame();
        runtime.rebuild(20.0, 120.0);

        // floor(20 frames * (2 / 10)) = 4.
        assert_eq!(runtime.frames().len(), 20);
        assert_eq!(runtime.frame(), 4);
        assert_eq!(before, 2);
        assert_eq!(runtime.fps(), 20.0);
    }

    // ————————————————— descriptors —————————————————

    #[test]
    fn descriptor_new_uses_the_create_tween_defaults() {
        let tween = TweenDescriptor::new("id-1", vec![vec![0.0], vec![1.0]]);
        assert_eq!(tween.duration, 1000.0);
        assert_eq!(tween.direction, "alternate");
        assert_eq!(tween.easing, "linear");
        assert!(tween.looping);
        assert!(tween.use_bpm);
        assert_eq!(tween.bpm_division, 16.0);
        assert!(!tween.duration_as_total_time);
        assert_eq!(tween.steps, 0.0);
        assert_eq!(tween.is_gallery, None);
        assert_eq!(tween.channel_count(), 1);
    }

    #[test]
    fn descriptor_round_trip_preserves_unknown_keys() {
        let mut config = default_link_tween("in-9", Some(&json!(8)));
        config["isGallery"] = json!(false);
        config["frames"] = json!([[0.0], [1.0]]);
        config["mysteryKey"] = json!({"deep": [true, null]});

        let descriptor: TweenDescriptor =
            serde_json::from_value(config.clone()).expect("descriptor");
        assert_eq!(descriptor.bpm_division, 8.0);
        assert_eq!(descriptor.is_gallery, Some(false));
        assert_eq!(descriptor.to_json(), config);

        // A second cycle is stable.
        let reparsed: TweenDescriptor =
            serde_json::from_value(descriptor.to_json()).expect("reparse");
        assert_eq!(reparsed, descriptor);
        assert_eq!(reparsed.to_json(), config);
    }

    #[test]
    fn descriptor_omits_keys_the_document_did_not_have() {
        let raw = json!({"id": "t1", "data": [[0], [1]]});
        let descriptor: TweenDescriptor = serde_json::from_value(raw.clone()).expect("descriptor");
        assert_eq!(descriptor.duration, 1000.0);
        assert!(descriptor.looping);
        assert_eq!(descriptor.to_json(), raw);

        // `isGallery` is never invented, only carried.
        let raw = json!({"id": "t2"});
        let descriptor: TweenDescriptor = serde_json::from_value(raw.clone()).expect("descriptor");
        assert_eq!(descriptor.to_json(), raw);
    }

    #[test]
    fn descriptor_requires_an_id() {
        assert!(serde_json::from_value::<TweenDescriptor>(json!({"data": [[0], [1]]})).is_err());
    }

    // ————————————————— links / the default Scene tween —————————————————

    #[test]
    fn default_link_tween_breathes_between_zero_and_one() {
        // The automatic Scene link tween (`scene-link-defaults.js:7-10`):
        // easeInOutSine, division 32, alternating, looped. At 128 BPM the leg
        // is `(3600 / 128) * 32 / 2 = 450 ms` → round(27) frames each way.
        let descriptor = descriptor_from(&default_link_tween("in-1", None));
        let frames = build_tween_frames(&descriptor, 60.0, 128.0);

        assert_eq!(frames.len(), 54);
        let values: Vec<f64> = frames.iter().map(|frame| frame[0]).collect();

        let min = values.iter().copied().fold(f64::INFINITY, f64::min);
        let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        assert_eq!(min, 0.0);
        assert_eq!(max, 1.0);

        // One full breath: up for 27 frames, back down for 27.
        assert_eq!(values[0], 0.0);
        assert_eq!(values[27], 1.0);
        assert!(values[1] > values[0] && values[26] < values[27]);
        assert!(values[28] < values[27] && values[53] > 0.0);
        // It never leaves the 0..1 range.
        assert!(values.iter().all(|value| (0.0..=1.0).contains(value)));
    }

    #[test]
    fn resolve_tween_link_advances_once_per_read() {
        let mut set = TweenSet::new();
        set.insert_descriptor(linear_tween(), 10.0, 120.0);

        let values: Vec<f64> = (0..7)
            .map(|_| resolve_tween_link(&mut set, "t1", 0).expect("link value"))
            .collect();
        assert_eq!(values, vec![0.0, 0.2, 0.4, 0.6, 0.8, 0.0, 0.2]);

        // A missing channel advances the tween (like the JS getter) but
        // resolves to nothing, so the link keeps its saved value.
        let before = set.get("t1").expect("tween").frame();
        assert_eq!(resolve_tween_link(&mut set, "t1", 3), None);
        assert_eq!(set.get("t1").expect("tween").frame(), (before + 1) % 5);
        assert_eq!(resolve_tween_link(&mut set, "missing", 0), None);
    }

    #[test]
    fn parse_tween_link_location_handles_the_stored_forms() {
        assert_eq!(
            parse_tween_link_location("tweens.tweens['in-1'].value[0]"),
            Some(("in-1".to_string(), 0))
        );
        assert_eq!(
            parse_tween_link_location("tweens.tweens[in-2].value[1]"),
            Some(("in-2".to_string(), 1))
        );
        assert_eq!(
            parse_tween_link_location("tweens['in-3'].value[2]"),
            Some(("in-3".to_string(), 2))
        );

        assert_eq!(parse_tween_link_location("meyda/getFeature"), None);
        assert_eq!(
            parse_tween_link_location("tweens.tweens['in-1'].value"),
            None
        );
        assert_eq!(parse_tween_link_location("tweens.tweens['in-1']"), None);
        assert_eq!(
            parse_tween_link_location("tweens.tweens['in-1'].value[x]"),
            None
        );
        assert_eq!(parse_tween_link_location("tweens.tweens['in-1']"), None);
    }

    // ————————————————— the set —————————————————

    #[test]
    fn tween_set_loads_preset_arrays_and_round_trips() {
        let first = default_link_tween("in-1", None);
        let mut second = default_link_tween("in-2", Some(&json!(16)));
        second["isGallery"] = json!(true);
        let document = json!([first, second]);

        let set = TweenSet::load_from_json(&document, 60.0, 128.0).expect("load");
        assert_eq!(set.len(), 2);
        assert_eq!(set.ids().collect::<Vec<_>>(), vec!["in-1", "in-2"]);
        assert!(set.contains("in-2"));

        // Gallery tweens stay in the document — filtering is the caller's job.
        assert_eq!(set.to_json(), document);
    }

    #[test]
    fn tween_set_loads_the_live_worker_shapes() {
        // `state.tweens` root (`{ tweens: { id: descriptor } }`).
        let document = json!({
            "tweens": {
                "a": {"id": "a", "data": [[0], [1]]},
                "b": {"data": [[0], [2]], "easing": "easeInOutSine"}
            }
        });
        let set = TweenSet::load_from_json(&document, 60.0, 120.0).expect("load");
        assert_eq!(set.len(), 2);
        assert_eq!(set.ids().collect::<Vec<_>>(), vec!["a", "b"]);
        // A descriptor without an id takes the map key (`getSceneTweenEntries`
        // / `scene_ops::restore_scene_inputs` behaviour).
        assert_eq!(set.get("b").expect("b").id(), "b");
        assert_eq!(
            set.to_json(),
            json!([
                {"id": "a", "data": [[0], [1]]},
                {"id": "b", "data": [[0], [2]], "easing": "easeInOutSine"}
            ])
        );

        // A bare id-keyed map works too.
        let set =
            TweenSet::load_from_json(&json!({"c": {"id": "c", "data": [[0], [1]]}}), 60.0, 120.0)
                .expect("load");
        assert_eq!(set.ids().collect::<Vec<_>>(), vec!["c"]);

        // null clears; a non-container is an error.
        assert!(
            TweenSet::load_from_json(&Value::Null, 60.0, 120.0)
                .expect("load")
                .is_empty()
        );
        assert_eq!(
            TweenSet::load_from_json(&json!("nope"), 60.0, 120.0).unwrap_err(),
            TweenError::InvalidShape
        );
        assert!(matches!(
            TweenSet::load_from_json(&json!([{"data": [[0], [1]]}]), 60.0, 120.0),
            Err(TweenError::Decode { .. })
        ));
    }

    #[test]
    fn tween_set_upserts_removes_and_snapshots() {
        let mut set = TweenSet::new();
        set.insert_descriptor(linear_tween(), 10.0, 120.0);
        assert_eq!(set.len(), 1);

        // The same id replaces in place, keeping document order.
        set.insert_descriptor(linear_tween(), 10.0, 120.0);
        assert_eq!(set.len(), 1);

        // `tweens/ADD_TWEEN` payloads decode straight in.
        let runtime = set
            .upsert_json(
                &json!({"id": "added", "data": [[0], [1]], "useBpm": false}),
                10.0,
                120.0,
            )
            .expect("upsert");
        assert_eq!(runtime.id(), "added");
        assert_eq!(set.ids().collect::<Vec<_>>(), vec!["t1", "added"]);

        set.get_mut("t1").expect("t1").set_frame(2);
        let snapshot = set.values_snapshot();
        assert_eq!(snapshot["t1"], vec![0.4]);
        assert_eq!(snapshot["added"], vec![0.0]);

        set.advance_all();
        assert_eq!(set.get("t1").expect("t1").frame(), 3);
        assert_eq!(set.get("added").expect("added").frame(), 1);

        assert!(set.remove("t1"));
        assert!(!set.remove("t1"));
        assert_eq!(set.ids().collect::<Vec<_>>(), vec!["added"]);
    }

    #[test]
    fn tween_set_replace_clears_the_previous_document() {
        let mut set =
            TweenSet::load_from_json(&json!([default_link_tween("old", None)]), 60.0, 120.0)
                .expect("load");
        assert!(set.contains("old"));

        set.replace_from_json(&json!([default_link_tween("new", None)]), 60.0, 120.0)
            .expect("replace");
        assert!(!set.contains("old"));
        assert!(set.contains("new"));
        assert_eq!(set.len(), 1);
    }
}
