//! Input Link evaluation for the Rust engine (WS-9 audio feed / WS-1b links).
//!
//! Port of the per-frame link loop in `worker/loop.js:134-199` plus the
//! mutation-link path in `worker/index.worker.js:80-128` and the value mapping
//! in `worker/input-link-value.js`. The engine evaluates every link from its
//! own state (the renderer only forwards store changes), so the music-reactive
//! props keep animating while the UI is idle.
//!
//! # Supported sources
//!
//! | link kind | `location` | value |
//! |-----------|------------|-------|
//! | `getter` | `meyda/getFeature`, `args = [feature, smoothingId?, smoothingValue?]` | the analyser feature, optionally smoothed |
//! | `state` | lodash path into `{ beats, tweens, midi }` | the current store value |
//! | `mutation` | `match.type = "beats/SET_KICK"` | [`BeatsState::kick`] |
//! | `mutation` | any other `match.type` | resolved from `location` like a `state` link |
//!
//! `beats.kick`, `beats.bpm` and `tweens.tweens['<id>'].value[0]` are resolved
//! against the engine state rather than a cloned JSON document. The engine
//! keeps `tweens` as the descriptor array (not the worker's live
//! `{ tweens: { id: { value } } }` map), so a tween link only produces a value
//! once WS-12 animates tween values. MIDI links read `state.midi`, which the
//! renderer forwards verbatim; OSC has no engine state slot yet, so an OSC
//! link is skipped until its state exists.
//!
//! # Feature names
//!
//! `bass`, `mid`, `treble`, the Meyda scalars (`rms`, `zcr`, `energy`,
//! `spectralCentroid`, `spectralFlatness`, `spectralSlope`, `spectralRolloff`,
//! `spectralSpread`, `spectralSkewness`, `spectralKurtosis`,
//! `perceptualSpread`, `perceptualSharpness`), and the Point10 grids as
//! `lane0`..`lane11`, `lanes/<i>`, `cell0`..`cell15`, `cells/<i>`.
//! Unknown features resolve to `None` and never write `NaN`.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::time::Duration;

use serde_json::Value;

use modv_engine::engine_state::EngineState;
use modv_engine::inputs::{InputLink, InputLinkKind, resolve_input_link_value};
use modv_engine::tweens::{TweenSet, parse_tween_link_location, resolve_tween_link};
use modv_renderers::two_d::AudioFrame;

/// Minimum spacing between Input Link deltas for one input.
///
/// The engine evaluates links every frame (60 Hz), but the UI mirror only needs
/// the slider position; forwarding every frame would flood the control socket.
/// One delta per [`LINK_DELTA_INTERVAL`] keeps the sliders alive at ~10 Hz.
pub const LINK_DELTA_INTERVAL: Duration = Duration::from_millis(100);

/// A change smaller than this is not worth a delta (and skips the prop write).
pub const LINK_DELTA_EPSILON: f64 = 1e-3;

/// Beat/tempo state an Input Link can read (`beats.kick`, `beats.bpm`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BeatsState {
    /// Current tempo in BPM (`beats/SET_BPM`; 0 until the detector reports).
    pub bpm: f64,
    /// Whether the current frame is a kick (`beats/SET_KICK`).
    pub kick: bool,
}

/// The target an input link writes to.
#[derive(Clone, Debug, PartialEq)]
pub enum LinkTarget {
    /// A Module prop (`data = { moduleId, prop }`), optionally at a sub-path.
    Module {
        /// Module id the input belongs to.
        module_id: String,
        /// Prop name.
        prop: String,
        /// Sub-prop path (`data.path`, e.g. `[0]`).
        path: Option<String>,
    },
    /// A Module meta input (`data = { id|moduleId, metaKey }`), e.g. the
    /// alpha/enabled/compositeOperation inputs every module gets
    /// (`modules.js:292-318`).
    ModuleMeta {
        /// Module id the input belongs to.
        module_id: String,
        /// Meta key (`alpha`, `enabled`, `compositeOperation`).
        meta_key: String,
    },
    /// A Group control (`data = { groupId, key }`), e.g. the automatic
    /// Group-alpha fallback link.
    Group {
        /// Group id the input belongs to.
        group_id: String,
        /// Group key (`alpha`, `enabled`, ...).
        key: String,
    },
}

/// One resolved link value ready to be applied to the store.
#[derive(Clone, Debug, PartialEq)]
pub struct LinkWrite {
    /// Id of the input the link belongs to (the delta-throttle key).
    pub input_id: String,
    /// The Module target, when the input binds a Module prop.
    pub module_id: Option<String>,
    /// The Group target, when the input binds a Group control.
    pub group_id: Option<String>,
    /// Prop name (`module_id`) or Group key (`group_id`).
    pub prop: String,
    /// Sub-prop path (`module_id` targets only).
    pub path: Option<String>,
    /// The mapped value to write.
    pub value: f64,
    /// The resolved target, used by the store when applying the write.
    pub target: LinkTarget,
}

/// Evaluate every Input Link in `state` against the current audio/beat state.
///
/// This is the pure half of the evaluator: it never touches the store, so the
/// link semantics are unit-testable without a GPU or a socket. The caller
/// applies the returned writes through [`crate::store_ops::Store`], which owns
/// the coercion and delta throttling.
///
/// `smoothing` is the per-smoothing-id peak state (`audio-features.js`
/// `smoothedFeatures`). It is keyed by `args[1]` (the uuid
/// `meyda/getSmoothingId` minted for the input); a link without a smoothing id
/// reads the raw feature.
pub fn evaluate_input_links(
    state: &EngineState,
    audio: &AudioFrame,
    beats: &BeatsState,
    tweens: &mut TweenSet,
    smoothing: &mut HashMap<String, f64>,
    writes: &mut Vec<LinkWrite>,
) {
    for (input_id, link) in &state.input_links {
        let Some(input) = state.inputs.get(input_id) else {
            continue;
        };
        let Some(target) = link_target(state, input) else {
            continue;
        };

        // `loop.js:164-170`: the JS loop skips a **meyda** link when its
        // Module is disabled (tween/state links keep updating so a re-enabled
        // module resumes at the current tween value), and skips mutation links
        // entirely (the mutation subscriber owns them). The engine evaluates
        // mutation links here because it has no Vuex subscription: they are
        // gated on the matcher below.
        // `createInputLink` always stores a source, but older/synthesised
        // documents may omit it: a sourceless `meyda/*` getter is still an
        // audio link and keeps the JS disabled-module gate.
        let meyda_link = link.source.as_deref() == Some("meyda")
            || (link.source.is_none()
                && link
                    .location
                    .as_deref()
                    .is_some_and(|location| location.starts_with("meyda/")));
        if let LinkTarget::Module { module_id, .. } | LinkTarget::ModuleMeta { module_id, .. } =
            &target
        {
            match state.modules.get(module_id) {
                Some(module) => {
                    if meyda_link
                        && module.meta.get("enabled").and_then(Value::as_bool) != Some(true)
                    {
                        continue;
                    }
                }
                None => continue,
            }
        } else if let LinkTarget::Group { group_id, .. } = &target {
            if state.find_group(group_id).is_none() {
                continue;
            }
        }

        let raw = match link.kind {
            InputLinkKind::Getter => resolve_getter(link, audio, smoothing),
            InputLinkKind::State => link.location.as_deref().and_then(|path| {
                // Tween links read (and advance) the live tween first; a
                // snapshot that carries a literal `value` still resolves
                // through the generic path walker below.
                if let Some((id, channel)) = parse_tween_link_location(path) {
                    resolve_tween_link(tweens, &id, channel)
                        .or_else(|| resolve_state_path(state, beats, path))
                } else {
                    resolve_state_path(state, beats, path)
                }
            }),
            InputLinkKind::Mutation => resolve_mutation(link, state, beats),
        };

        // A missing feature (no analyser frame) and a non-finite result keep
        // the input's saved value (`input-link-value.js`).
        let Some(value) =
            raw.and_then(|raw| resolve_input_link_value(Some(raw), link.min, link.max))
        else {
            continue;
        };

        let (module_id, group_id, prop, path) = match &target {
            LinkTarget::Module {
                module_id,
                prop,
                path,
            } => (Some(module_id.clone()), None, prop.clone(), path.clone()),
            LinkTarget::ModuleMeta {
                module_id,
                meta_key,
            } => (Some(module_id.clone()), None, meta_key.clone(), None),
            LinkTarget::Group { group_id, key } => {
                (None, Some(group_id.clone()), key.clone(), None)
            }
        };

        writes.push(LinkWrite {
            input_id: input_id.clone(),
            module_id,
            group_id,
            prop,
            path,
            value,
            target,
        });
    }
}

/// Resolve an input's write target from its bind-side `data`
/// (`inputs.js` `action`/`commit` records).
fn link_target(state: &EngineState, input: &modv_engine::inputs::Input) -> Option<LinkTarget> {
    let data = input.data.as_object()?;

    if let (Some(module_id), Some(prop)) = (
        data.get("moduleId").and_then(Value::as_str),
        data.get("prop").and_then(Value::as_str),
    ) {
        return Some(LinkTarget::Module {
            module_id: module_id.to_string(),
            prop: prop.to_string(),
            path: data
                .get("path")
                .and_then(Value::as_str)
                .filter(|path| !path.is_empty())
                .map(str::to_string),
        });
    }

    if let Some(meta_key) = data.get("metaKey").and_then(Value::as_str) {
        let module_id = data
            .get("moduleId")
            .or_else(|| data.get("id"))
            .and_then(Value::as_str)?;
        if state.modules.contains_key(module_id) {
            return Some(LinkTarget::ModuleMeta {
                module_id: module_id.to_string(),
                meta_key: meta_key.to_string(),
            });
        }
    }

    if let (Some(group_id), Some(key)) = (
        data.get("groupId").and_then(Value::as_str),
        data.get("key").and_then(Value::as_str),
    ) {
        if state.find_group(group_id).is_some() {
            return Some(LinkTarget::Group {
                group_id: group_id.to_string(),
                key: key.to_string(),
            });
        }
    }

    None
}

/// Resolve a `getter` link (`meyda/getFeature`), applying the optional
/// smoothing arguments exactly like `meyda.js`'s getter:
///
/// ```text
/// if (smoothingId && smoothingValue) {
///   smoothed[id] = lerp(smoothed[id] || 0, features[feature], smoothingValue)
///   return smoothed[id]
/// }
/// return features[feature]
/// ```
fn resolve_getter(
    link: &InputLink,
    audio: &AudioFrame,
    smoothing: &mut HashMap<String, f64>,
) -> Option<f64> {
    if link.location.as_deref() != Some("meyda/getFeature") {
        return None;
    }

    let args = link.args.as_deref()?;
    let feature = args.first()?.as_str()?;
    let value = feature_value(feature, audio)?;

    let smoothing_id = args.get(1).and_then(value_key);
    let smoothing_value = args.get(2).and_then(Value::as_f64);

    match (smoothing_id, smoothing_value) {
        (Some(id), Some(amount)) if amount != 0.0 && amount.is_finite() => {
            let previous = smoothing.get(&id).copied().unwrap_or(0.0);
            let smoothed = previous + (value - previous) * amount;
            smoothing.insert(id, smoothed);
            Some(smoothed)
        }
        _ => Some(value),
    }
}

/// The key a smoothing argument is stored under (string or numeric JSON).
fn value_key(value: &Value) -> Option<String> {
    match value {
        Value::String(text) if !text.is_empty() => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

/// Map a `meyda/getFeature` feature name onto the analyser payload.
fn feature_value(feature: &str, audio: &AudioFrame) -> Option<f64> {
    let features = &audio.features;
    match feature {
        "bass" => Some(f64::from(audio.bass)),
        "mid" => Some(f64::from(audio.mid)),
        "treble" => Some(f64::from(audio.treble)),
        "rms" => Some(f64::from(features.rms)),
        "zcr" => Some(f64::from(features.zcr)),
        "energy" => Some(f64::from(features.energy)),
        "spectralCentroid" => Some(f64::from(features.spectral_centroid)),
        "spectralFlatness" => Some(f64::from(features.spectral_flatness)),
        "spectralSlope" => Some(f64::from(features.spectral_slope)),
        "spectralRolloff" => Some(f64::from(features.spectral_rolloff)),
        "spectralSpread" => Some(f64::from(features.spectral_spread)),
        "spectralSkewness" => Some(f64::from(features.spectral_skewness)),
        "spectralKurtosis" => Some(f64::from(features.spectral_kurtosis)),
        "perceptualSpread" => Some(f64::from(features.perceptual_spread)),
        "perceptualSharpness" => Some(f64::from(features.perceptual_sharpness)),
        _ => feature_grid_value(feature, audio).or_else(|| {
            let index = feature.strip_prefix("band")?.parse::<usize>().ok()?;
            match index {
                0 => Some(f64::from(audio.bass)),
                1 => Some(f64::from(audio.mid)),
                2 => Some(f64::from(audio.treble)),
                _ => None,
            }
        }),
    }
}

/// `lane0`/`lanes/0` and `cell0`/`cells/0` grid features.
fn feature_grid_value(feature: &str, audio: &AudioFrame) -> Option<f64> {
    if let Some(index) = feature.strip_prefix("lanes/") {
        return index
            .parse::<usize>()
            .ok()
            .and_then(|index| audio.lanes.get(index))
            .map(|value| f64::from(*value));
    }
    if let Some(index) = feature.strip_prefix("lane") {
        return index
            .parse::<usize>()
            .ok()
            .and_then(|index| audio.lanes.get(index))
            .map(|value| f64::from(*value));
    }
    if let Some(index) = feature.strip_prefix("cells/") {
        return index
            .parse::<usize>()
            .ok()
            .and_then(|index| audio.cells.get(index))
            .map(|value| f64::from(*value));
    }
    if let Some(index) = feature.strip_prefix("cell") {
        return index
            .parse::<usize>()
            .ok()
            .and_then(|index| audio.cells.get(index))
            .map(|value| f64::from(*value));
    }
    None
}

/// Resolve a `mutation` link.
///
/// `beats/SET_KICK` reads the current kick state; every other mutation type
/// falls back to its `location` (the renderer forwards MIDI/OSC data into the
/// store, so the last written value is the current state).
fn resolve_mutation(link: &InputLink, state: &EngineState, beats: &BeatsState) -> Option<f64> {
    let mutation_type = link
        .matcher
        .as_ref()
        .and_then(|matcher| matcher.get("type"))
        .and_then(Value::as_str)?;

    if mutation_type == "beats/SET_KICK" {
        return Some(if beats.kick { 1.0 } else { 0.0 });
    }

    link.location
        .as_deref()
        .and_then(|path| resolve_state_path(state, beats, path))
}

/// One parsed lodash-style path segment.
#[derive(Clone, Debug, PartialEq, Eq)]
enum PathSegment {
    /// Object key.
    Key(String),
    /// Array index.
    Index(usize),
}

/// Parse `a.b`, `['key']`, `[0]`, `a['b-c'].value[0]` into segments.
fn parse_path(path: &str) -> Vec<PathSegment> {
    let mut segments = Vec::new();
    let mut key = String::new();
    let mut chars = path.chars().peekable();

    while let Some(character) = chars.next() {
        match character {
            '.' => {
                if !key.is_empty() {
                    segments.push(PathSegment::Key(std::mem::take(&mut key)));
                }
            }
            '[' => {
                if !key.is_empty() {
                    segments.push(PathSegment::Key(std::mem::take(&mut key)));
                }
                let mut inside = String::new();
                let quote = chars.peek().copied().filter(|c| *c == '\'' || *c == '"');
                let quoted = quote.is_some();
                if let Some(quote) = quote {
                    chars.next();
                    for next in chars.by_ref() {
                        if next == quote {
                            break;
                        }
                        inside.push(next);
                    }
                    // Consume the closing `]`.
                    for next in chars.by_ref() {
                        if next == ']' {
                            break;
                        }
                    }
                } else {
                    for next in chars.by_ref() {
                        if next == ']' {
                            break;
                        }
                        inside.push(next);
                    }
                }

                // `['0']` is an object key (MIDI channel/type maps use string
                // keys); only an unquoted `[0]` is an array index.
                if !quoted {
                    if let Ok(index) = inside.parse::<usize>() {
                        segments.push(PathSegment::Index(index));
                        continue;
                    }
                }
                if !inside.is_empty() {
                    segments.push(PathSegment::Key(inside));
                }
            }
            other => key.push(other),
        }
    }
    if !key.is_empty() {
        segments.push(PathSegment::Key(key));
    }
    segments
}

/// Resolve a store path against the engine's link-visible state.
///
/// Only `beats`, `tweens` and `midi` exist engine-side; a path into any other
/// root is missing (the JS store has more roots, but nothing the engine
/// mirrors today produces them).
fn resolve_state_path(state: &EngineState, beats: &BeatsState, path: &str) -> Option<f64> {
    let segments = parse_path(path);
    let (first, rest) = segments.split_first()?;
    let PathSegment::Key(root) = first else {
        return None;
    };

    match root.as_str() {
        "beats" => match rest {
            [PathSegment::Key(key)] if key == "kick" => Some(if beats.kick { 1.0 } else { 0.0 }),
            [PathSegment::Key(key)] if key == "bpm" => Some(beats.bpm),
            _ => None,
        },
        "tweens" => resolve_tween(&state.tweens, rest),
        "midi" => resolve_value(&state.midi, rest).and_then(value_as_f64),
        // No engine `osc` state yet (WS-9 deviation): the renderer forwards
        // MIDI but not the OSC receive buffer, so the link stays at its saved
        // value until that state lands.
        "osc" => None,
        _ => None,
    }
}

/// `tweens.tweens['<id>'].value[0]` against the engine's tween descriptor
/// array.
///
/// The engine keeps `tweens` as the preset array; the worker's live
/// `state.tweens.tweens` is an id-keyed map. Accept both shapes: a map-like
/// object keyed by id or an array searched by `id`.
fn resolve_tween(tweens: &Value, rest: &[PathSegment]) -> Option<f64> {
    let mut rest = rest;
    if let Some((PathSegment::Key(key), tail)) = rest.split_first() {
        if key == "tweens" {
            rest = tail;
        }
    }
    let (PathSegment::Key(id), tail) = rest.split_first()? else {
        return None;
    };

    let tween = match tweens {
        Value::Array(entries) => entries
            .iter()
            .find(|entry| entry.get("id").and_then(Value::as_str) == Some(id.as_str())),
        Value::Object(map) => map.get(id),
        _ => None,
    }?;

    resolve_value(tween, tail).and_then(value_as_f64)
}

/// Walk `segments` into a JSON value.
fn resolve_value<'a>(value: &'a Value, segments: &[PathSegment]) -> Option<&'a Value> {
    let mut current = value;
    for segment in segments {
        current = match segment {
            PathSegment::Key(key) => current.get(key)?,
            PathSegment::Index(index) => current.get(*index)?,
        };
    }
    Some(current)
}

/// Numbers and booleans (as `0`/`1`) are link values; anything else is missing.
fn value_as_f64(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_bool().map(|value| if value { 1.0 } else { 0.0 }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use modv_engine::engine_state::ModuleInstance;
    use serde_json::{Map, json};

    fn module(id: &str, enabled: bool, props: Value) -> ModuleInstance {
        ModuleInstance {
            id: id.to_string(),
            module_name: "Bands".to_string(),
            meta: json!({ "name": "Bands", "type": "shader", "enabled": enabled }),
            props,
            data: json!({}),
            dollar_props: json!({}),
            status: Vec::new(),
            extra: Map::new(),
        }
    }

    /// A module at `id` plus an action input binding `prop`, and a link.
    fn state_with_link(
        id: &str,
        module_id: &str,
        prop: &str,
        path: Option<&str>,
        link: Value,
    ) -> (EngineState, String) {
        let mut state = EngineState::new();
        state.modules.insert(
            module_id.to_string(),
            module(module_id, true, json!({ prop: 0.0 })),
        );
        let input_id = format!("input-{id}");
        let mut data = json!({ "moduleId": module_id, "prop": prop });
        if let Some(path) = path {
            data["path"] = json!(path);
        }
        state.inputs.insert(
            input_id.clone(),
            serde_json::from_value(json!({
                "id": input_id,
                "type": "action",
                "data": data,
                "location": "modules/updateProp",
            }))
            .expect("input parses"),
        );
        let mut link = link;
        link["id"] = json!(input_id);
        state.input_links.insert(
            input_id.clone(),
            serde_json::from_value(link).expect("link parses"),
        );
        (state, input_id)
    }

    fn audio() -> AudioFrame {
        AudioFrame {
            bass: 0.25,
            mid: 0.5,
            treble: 0.75,
            beat: 0.6,
            lanes: [
                0.11, 0.12, 0.13, 0.21, 0.22, 0.23, 0.31, 0.32, 0.33, 0.41, 0.42, 0.43,
            ],
            cells: [
                0.01, 0.02, 0.03, 0.04, 0.05, 0.06, 0.07, 0.08, 0.09, 0.1, 0.11, 0.12, 0.13, 0.14,
                0.15, 0.16,
            ],
            features: modv_renderers::two_d::AudioFeatures {
                rms: 0.4,
                zcr: 0.05,
                energy: 0.9,
                ..Default::default()
            },
            ..AudioFrame::default()
        }
    }

    fn evaluate(state: &EngineState) -> Vec<LinkWrite> {
        evaluate_with_tweens(state, &mut TweenSet::new())
    }

    fn evaluate_with_tweens(state: &EngineState, tweens: &mut TweenSet) -> Vec<LinkWrite> {
        let mut smoothing = HashMap::new();
        let mut writes = Vec::new();
        evaluate_input_links(
            state,
            &audio(),
            &BeatsState::default(),
            tweens,
            &mut smoothing,
            &mut writes,
        );
        writes
    }

    #[test]
    fn meta_inputs_resolve_to_module_meta_targets() {
        // `modules.js:292-318`: alpha/enabled/compositeOperation inputs carry
        // `data = { id|moduleId, metaKey }` and bind `modules/updateMeta`.
        let mut state = EngineState::new();
        state
            .modules
            .insert("m-meta".to_string(), module("m-meta", true, json!({})));
        state.inputs.insert(
            "input-meta".to_string(),
            serde_json::from_value(json!({
                "id": "input-meta",
                "type": "action",
                "location": "modules/updateMeta",
                "getLocation": "modules.active[\"m-meta\"].meta.alpha",
                "data": { "id": "m-meta", "metaKey": "alpha" },
            }))
            .expect("meta input parses"),
        );
        state.input_links.insert(
            "input-meta".to_string(),
            serde_json::from_value(json!({
                "id": "input-meta",
                "type": "getter",
                "location": "meyda/getFeature",
                "source": "meyda",
                "args": ["bass"],
                "min": 0.2,
                "max": 0.8,
            }))
            .expect("meta link parses"),
        );

        let writes = evaluate(&state);
        assert_eq!(writes.len(), 1);
        assert_eq!(
            writes[0].target,
            LinkTarget::ModuleMeta {
                module_id: "m-meta".to_string(),
                meta_key: "alpha".to_string(),
            }
        );
        assert!((writes[0].value - 0.35).abs() < 1e-6);

        // A disabled module is skipped for meta links too (JS loop gate).
        state.modules.get_mut("m-meta").expect("module").meta = json!({ "enabled": false });
        assert!(evaluate(&state).is_empty());
    }

    #[test]
    fn only_meyda_links_skip_a_disabled_module() {
        let (mut state, _) = state_with_link(
            "tween",
            "m1",
            "amount",
            None,
            json!({
                "type": "state",
                "location": "tweens.tweens['t1'].value[0]",
                "source": "tween",
            }),
        );
        state.tweens = json!([{ "id": "t1", "value": [0.4] }]);
        state.modules.get_mut("m1").expect("module").meta = json!({ "enabled": false });
        let writes = evaluate(&state);
        assert_eq!(writes.len(), 1, "tween links update disabled modules");
        assert!((writes[0].value - 0.4).abs() < 1e-6);

        let (mut state, _) = state_with_link(
            "meyda",
            "m2",
            "amount",
            None,
            json!({ "type": "getter", "location": "meyda/getFeature", "args": ["bass"], "source": "meyda" }),
        );
        state.modules.get_mut("m2").expect("module").meta = json!({ "enabled": false });
        assert!(
            evaluate(&state).is_empty(),
            "meyda links skip disabled modules"
        );
    }

    #[test]
    fn getter_maps_bands_and_scalars() {
        for (feature, expected) in [
            ("bass", 0.25),
            ("mid", 0.5),
            ("treble", 0.75),
            ("rms", 0.4),
            ("zcr", 0.05),
            ("energy", 0.9),
        ] {
            let (state, _) = state_with_link(
                feature,
                "m1",
                "amount",
                None,
                json!({ "type": "getter", "location": "meyda/getFeature", "args": [feature] }),
            );
            let writes = evaluate(&state);
            assert_eq!(writes.len(), 1, "{feature}");
            assert!((writes[0].value - expected).abs() < 1e-6, "{feature}");
        }
    }

    #[test]
    fn getter_maps_lanes_and_cells() {
        for (feature, expected) in [
            ("lane0", 0.11),
            ("lane11", 0.43),
            ("lanes/3", 0.21),
            ("cell0", 0.01),
            ("cell15", 0.16),
            ("cells/9", 0.1),
        ] {
            let (state, _) = state_with_link(
                "grid",
                "m1",
                "amount",
                None,
                json!({ "type": "getter", "location": "meyda/getFeature", "args": [feature] }),
            );
            let writes = evaluate(&state);
            assert_eq!(writes.len(), 1, "{feature}");
            assert!((writes[0].value - expected).abs() < 1e-6, "{feature}");
        }
    }

    #[test]
    fn unknown_feature_is_skipped() {
        let (state, _) = state_with_link(
            "unknown",
            "m1",
            "amount",
            None,
            json!({ "type": "getter", "location": "meyda/getFeature", "args": ["nope"] }),
        );
        assert!(evaluate(&state).is_empty());
    }

    #[test]
    fn getter_smoothing_lerps_towards_the_feature() {
        let (state, _) = state_with_link(
            "smooth",
            "m1",
            "amount",
            None,
            json!({
                "type": "getter",
                "location": "meyda/getFeature",
                "args": ["bass", "smoothing-id", 0.5]
            }),
        );
        let mut smoothing = HashMap::new();
        let mut writes = Vec::new();

        evaluate_input_links(
            &state,
            &audio(),
            &BeatsState::default(),
            &mut TweenSet::new(),
            &mut smoothing,
            &mut writes,
        );
        // First frame: lerp(0, 0.25, 0.5) = 0.125.
        assert!((writes.last().unwrap().value - 0.125).abs() < 1e-6);

        writes.clear();
        evaluate_input_links(
            &state,
            &audio(),
            &BeatsState::default(),
            &mut TweenSet::new(),
            &mut smoothing,
            &mut writes,
        );
        // Second frame: lerp(0.125, 0.25, 0.5) = 0.1875.
        assert!((writes.last().unwrap().value - 0.1875).abs() < 1e-6);
    }

    #[test]
    fn min_max_maps_the_getter_value() {
        let (state, _) = state_with_link(
            "range",
            "m1",
            "amount",
            None,
            json!({
                "type": "getter",
                "location": "meyda/getFeature",
                "args": ["bass"],
                "min": 10,
                "max": 20
            }),
        );
        let writes = evaluate(&state);
        assert_eq!(writes.len(), 1);
        assert!((writes[0].value - 12.5).abs() < 1e-6);
    }

    #[test]
    fn null_range_passes_the_raw_value() {
        let (state, _) = state_with_link(
            "passthrough",
            "m1",
            "amount",
            None,
            json!({
                "type": "getter",
                "location": "meyda/getFeature",
                "args": ["bass"],
                "min": null,
                "max": null
            }),
        );
        let writes = evaluate(&state);
        assert_eq!(writes.len(), 1);
        assert!((writes[0].value - 0.25).abs() < 1e-6);
    }

    #[test]
    fn state_link_reads_beats_paths() {
        let (mut state, _) = state_with_link(
            "kick",
            "m1",
            "amount",
            None,
            json!({ "type": "state", "location": "beats.kick" }),
        );
        let beats = BeatsState {
            bpm: 128.0,
            kick: true,
        };
        let mut smoothing = HashMap::new();
        let mut writes = Vec::new();
        evaluate_input_links(
            &state,
            &audio(),
            &beats,
            &mut TweenSet::new(),
            &mut smoothing,
            &mut writes,
        );
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].value, 1.0);

        // bpm path
        state.input_links.get_mut("input-kick").unwrap().location = Some("beats.bpm".to_string());
        writes.clear();
        evaluate_input_links(
            &state,
            &audio(),
            &beats,
            &mut TweenSet::new(),
            &mut smoothing,
            &mut writes,
        );
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].value, 128.0);
    }

    #[test]
    fn state_link_reads_nested_tween_path() {
        let (mut state, _) = state_with_link(
            "tween",
            "m1",
            "amount",
            None,
            json!({ "type": "state", "location": "tweens.tweens['tween-1'].value[0]" }),
        );
        state.tweens = json!([{ "id": "tween-1", "value": [0.42] }]);
        let writes = evaluate(&state);
        assert_eq!(writes.len(), 1);
        assert!((writes[0].value - 0.42).abs() < 1e-6);
    }

    #[test]
    fn state_link_reads_midi_dot_and_bracket_paths() {
        let (mut state, _) = state_with_link(
            "midi",
            "m1",
            "amount",
            None,
            json!({
                "type": "state",
                "location": "midi.devices['dev-a'].channelData['1']['144']"
            }),
        );
        state.midi = json!({
            "devices": {
                "dev-a": { "channelData": { "1": { "144": 0.7 } } }
            }
        });
        let writes = evaluate(&state);
        assert_eq!(writes.len(), 1);
        assert!((writes[0].value - 0.7).abs() < 1e-6);
    }

    #[test]
    fn missing_state_path_is_skipped() {
        let (state, _) = state_with_link(
            "missing",
            "m1",
            "amount",
            None,
            json!({ "type": "state", "location": "midi.devices['nope'].value" }),
        );
        assert!(evaluate(&state).is_empty());
    }

    #[test]
    fn osc_state_links_are_skipped_until_their_state_exists() {
        let (state, _) = state_with_link(
            "osc",
            "m1",
            "amount",
            None,
            json!({ "type": "state", "location": "osc.data.foo" }),
        );
        assert!(evaluate(&state).is_empty());
    }

    #[test]
    fn mutation_kick_link_follows_the_beat_state() {
        let (state, _) = state_with_link(
            "mutation-kick",
            "m1",
            "amount",
            None,
            json!({
                "type": "mutation",
                "location": "beats.kick",
                "match": { "type": "beats/SET_KICK" }
            }),
        );
        let mut smoothing = HashMap::new();
        let mut writes = Vec::new();
        evaluate_input_links(
            &state,
            &audio(),
            &BeatsState {
                bpm: 0.0,
                kick: true,
            },
            &mut TweenSet::new(),
            &mut smoothing,
            &mut writes,
        );
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].value, 1.0);

        writes.clear();
        evaluate_input_links(
            &state,
            &audio(),
            &BeatsState::default(),
            &mut TweenSet::new(),
            &mut smoothing,
            &mut writes,
        );
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].value, 0.0);
    }

    #[test]
    fn mutation_midi_link_resolves_its_location() {
        let (mut state, _) = state_with_link(
            "mutation-midi",
            "m1",
            "amount",
            None,
            json!({
                "type": "mutation",
                "location": "midi.devices['dev-a'].channelData['0']['144']",
                "match": { "type": "midi/WRITE_DATA" }
            }),
        );
        state.midi = json!({
            "devices": { "dev-a": { "channelData": { "0": { "144": 0.33 } } } }
        });
        let writes = evaluate(&state);
        assert_eq!(writes.len(), 1);
        assert!((writes[0].value - 0.33).abs() < 1e-6);
    }

    #[test]
    fn disabled_module_is_skipped() {
        let (mut state, _) = state_with_link(
            "disabled",
            "m1",
            "amount",
            None,
            json!({ "type": "getter", "location": "meyda/getFeature", "args": ["bass"] }),
        );
        state
            .modules
            .get_mut("m1")
            .unwrap()
            .meta
            .as_object_mut()
            .unwrap()
            .insert("enabled".to_string(), json!(false));
        assert!(evaluate(&state).is_empty());
    }

    #[test]
    fn missing_input_and_missing_module_are_skipped() {
        let (mut state, _) = state_with_link(
            "missing-module",
            "m1",
            "amount",
            None,
            json!({ "type": "getter", "location": "meyda/getFeature", "args": ["bass"] }),
        );
        state.modules.shift_remove("m1");
        assert!(evaluate(&state).is_empty());

        state.inputs.shift_remove("input-missing-module");
        assert!(evaluate(&state).is_empty());
    }

    #[test]
    fn sub_prop_path_is_carried_to_the_write() {
        let (state, _) = state_with_link(
            "path",
            "m1",
            "size",
            Some("[0]"),
            json!({ "type": "getter", "location": "meyda/getFeature", "args": ["bass"] }),
        );
        let writes = evaluate(&state);
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].path.as_deref(), Some("[0]"));
        assert_eq!(writes[0].prop, "size");
    }

    #[test]
    fn group_control_links_target_the_group() {
        let mut state = EngineState::new();
        let group: modv_engine::engine_state::Group = serde_json::from_value(json!({
            "id": "g1",
            "name": "Effects",
            "hidden": false,
            "modules": [],
            "enabled": 1,
            "alpha": 1.0,
            "inherit": true,
            "inheritFrom": -1,
            "pipeline": false,
            "clearing": false,
            "compositeOperation": "normal",
        }))
        .expect("group parses");
        state.groups.push(group);
        state.inputs.insert(
            "input-group".to_string(),
            serde_json::from_value(json!({
                "id": "input-group",
                "type": "commit",
                "data": { "groupId": "g1", "key": "alpha" },
                "location": "groups/UPDATE_GROUP_BY_KEY",
            }))
            .unwrap(),
        );
        state.input_links.insert(
            "input-group".to_string(),
            serde_json::from_value(json!({
                "id": "input-group",
                "type": "getter",
                "location": "meyda/getFeature",
                "args": ["bass"],
                "min": 0.55,
                "max": 1.0
            }))
            .unwrap(),
        );

        let writes = evaluate(&state);
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].group_id.as_deref(), Some("g1"));
        assert_eq!(writes[0].prop, "alpha");
        assert!((writes[0].value - 0.6625).abs() < 1e-6);
    }

    #[test]
    fn non_finite_feature_is_skipped() {
        let (state, _) = state_with_link(
            "nan",
            "m1",
            "amount",
            None,
            json!({ "type": "getter", "location": "meyda/getFeature", "args": ["bass"] }),
        );
        let mut audio = audio();
        audio.bass = f32::NAN;
        let mut smoothing = HashMap::new();
        let mut writes = Vec::new();
        evaluate_input_links(
            &state,
            &audio,
            &BeatsState::default(),
            &mut TweenSet::new(),
            &mut smoothing,
            &mut writes,
        );
        assert!(writes.is_empty());
    }
}
