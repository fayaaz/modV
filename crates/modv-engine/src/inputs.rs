//! Input links: the typed link model and the value-resolution pipeline.
//!
//! Ported from `store/modules/inputs.js` (state shape, link typedefs),
//! `worker/input-link-value.js` (`resolveInputLinkValue`) and
//! `utils/map.js` (`map`).
//!
//! A saved input value is only overwritten when the link resolves to a
//! finite number; `null`/`undefined`/`NaN` sources (e.g. audio features before
//! the first audio frame) must keep the saved value — this is the bug class
//! that produced `NaN` props.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The source kind of an input link.
///
/// `action`/`commit` are *bind-side* (which store mutation/action the input
/// fires), not source kinds, so they do not appear here — see [`Input`] and
/// [`LinkBind`]. Serialised lowercase, matching `inputs.js:10-13`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InputLinkKind {
    /// Reads a store getter, possibly with arguments; updates every frame.
    Getter,
    /// Reads a location in the store; updates every frame.
    State,
    /// Waits for a mutation type (optionally with a payload matcher) and
    /// applies a store value; fires on demand.
    Mutation,
}

/// The bind side of an [`Input`]: which store action/mutation it dispatches
/// and where it lives.
///
/// Mirrors the `type`/`location` pair on inputs created with
/// `type: "action"` (e.g. `modules/updateProp`) and `type: "commit"`
/// (e.g. `groups/UPDATE_GROUP_BY_KEY`). Plain action/commit targets are
/// looked up by name at bind time; they are not link sources.
#[derive(Clone, PartialEq, Debug)]
pub enum LinkBind {
    /// Vuex-style action dispatch (`type: "action"`), e.g. `modules/updateProp`.
    Action { location: String },
    /// Vuex-style commit (`type: "commit"`), e.g. `groups/UPDATE_GROUP_BY_KEY`.
    Commit { location: String },
}

/// A bound input: the write target of an [`InputLink`].
///
/// Serialises with the plain-object shape from `inputs.js`
/// (`{ type, getLocation, location, data, id }`). `getLocation` is the
/// store path the action reads `value` from (`modules.active[...].props.x`);
/// it is absent for commits (their payload carries `data`).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Input {
    /// UUID of the input.
    pub id: String,
    /// Bind-side kind: `"action"` or `"commit"`.
    #[serde(rename = "type")]
    pub type_: String,
    /// Opaque payload dispatched with the action/commit
    /// (e.g. `{ moduleId, prop }`).
    #[serde(default)]
    pub data: Value,
    /// Store path the action reads from (`getLocation` in JS).
    #[serde(
        rename = "getLocation",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub get_location: Option<String>,
    /// Bind-side location: the action/mutation type,
    /// e.g. `modules/updateProp`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
}

impl Input {
    /// The bind side of this input, if it declares one.
    pub fn bind(&self) -> Option<LinkBind> {
        let location = self.location.clone()?;
        match self.type_.as_str() {
            "action" => Some(LinkBind::Action { location }),
            "commit" => Some(LinkBind::Commit { location }),
            _ => None,
        }
    }
}

/// An input link: binds an input to a store source and maps the raw source
/// value into the input's range.
///
/// Serialises with the plain-object shape from `inputs.js`:
/// `{ id, type, location, args, min, max, source, match }`. `id` is the id of
/// the input this link writes to. `min`/`max` scale the incoming value from
/// `[0, 1]` into `[min, max]` (JS default `0`/`1`); absent bounds are not
/// written back on save.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct InputLink {
    /// Id of the input this link writes to.
    pub id: String,
    /// Source kind: `getter` | `state` | `mutation`.
    #[serde(rename = "type")]
    pub kind: InputLinkKind,
    /// Store path of the source value. Required by every source kind in
    /// practice, but a link *without* one is legal in JS
    /// (`createInputLink` only requires `source`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    /// Arguments for a getter that is a function, e.g. `["energy"]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<Value>>,
    /// Identifies what created and/or manages this link (`"meyda"`,
    /// `"tween"`, `"midiscript"`, ...). Required by `createInputLink`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Minimum expected value; scales the incoming `[0, 1]` value into
    /// `[min, max]` when present (with or without `max`). Serialised the way
    /// JS writes numbers (integral bounds become integers).
    #[serde(
        default,
        serialize_with = "crate::engine_state::serialize_minimal_option_f64",
        skip_serializing_if = "Option::is_none"
    )]
    pub min: Option<f64>,
    /// Maximum expected value; see `min`. Serialised the way JS writes
    /// numbers (integral bounds become integers).
    #[serde(
        default,
        serialize_with = "crate::engine_state::serialize_minimal_option_f64",
        skip_serializing_if = "Option::is_none"
    )]
    pub max: Option<f64>,
    /// Payload matcher for `mutation` links: the mutation type
    /// (`match.type`, e.g. `midi/WRITE_DATA`) and optional key-value pairs to
    /// match in the payload (`match.payload`). Kept as raw JSON.
    #[serde(rename = "match", default, skip_serializing_if = "Option::is_none")]
    pub matcher: Option<Value>,
    /// Unknown keys, kept verbatim for round-trip fidelity.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl InputLink {
    /// `true` for a `getter` link.
    pub fn is_getter(&self) -> bool {
        self.kind == InputLinkKind::Getter
    }

    /// `true` for a `state` link.
    pub fn is_state(&self) -> bool {
        self.kind == InputLinkKind::State
    }

    /// `true` for a `mutation` link.
    pub fn is_mutation(&self) -> bool {
        self.kind == InputLinkKind::Mutation
    }
}

/// Map `value` from `[from_min, from_max]` to `[to_min, to_max]`.
///
/// Port of `utils/map.js`:
/// `low2 + (high2 - low2) * (value - low1) / (high1 - low1)`.
pub fn map_range(value: f64, from_min: f64, from_max: f64, to_min: f64, to_max: f64) -> f64 {
    to_min + (value - from_min) * (to_max - to_min) / (from_max - from_min)
}

/// Resolve an input link's raw source value into the value written to the
/// linked input.
///
/// Port of `worker/input-link-value.js:9-22`, exactly:
///
/// - `value` undefined/null → `None`.
/// - If `min != null || max != null` →
///   `map_range(value, 0, 1, min, max)`. A missing bound becomes `NaN`
///   (JS passes `undefined` through `map`), so e.g. a `min`-only link
///   resolves to a non-finite number and is filtered to `None`.
/// - A non-finite resolved number → `None` (JS `Number.isFinite` check).
/// - Otherwise the resolved value is `Some(resolved)`.
///
/// A finite `0` is a valid value — `None` must never be produced for it.
/// Callers must treat `None` as "keep the saved value".
pub fn resolve_input_link_value(
    value: Option<f64>,
    min: Option<f64>,
    max: Option<f64>,
) -> Option<f64> {
    let value = value?;

    let resolved = if min.is_some() || max.is_some() {
        map_range(
            value,
            0.0,
            1.0,
            min.unwrap_or(f64::NAN),
            max.unwrap_or(f64::NAN),
        )
    } else {
        value
    };

    if !resolved.is_finite() {
        return None;
    }

    Some(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn map_range_scales() {
        assert_eq!(map_range(0.5, 0.0, 1.0, 10.0, 20.0), 15.0);
        assert_eq!(map_range(0.0, 0.0, 1.0, 10.0, 20.0), 10.0);
        assert_eq!(map_range(1.0, 0.0, 1.0, 10.0, 20.0), 20.0);
        assert_eq!(map_range(2.0, 0.0, 1.0, 0.0, 100.0), 200.0);
    }

    #[test]
    fn resolve_undefined_is_none() {
        assert_eq!(resolve_input_link_value(None, Some(0.0), Some(1.0)), None);
        assert_eq!(resolve_input_link_value(None, None, None), None);
    }

    #[test]
    fn resolve_null_is_none() {
        // null arrives from JSON as `null`; the Option wraps it.
        assert_eq!(resolve_input_link_value(None, Some(10.0), Some(20.0)), None);
    }

    #[test]
    fn resolve_midpoint_with_range() {
        assert_eq!(
            resolve_input_link_value(Some(0.5), Some(10.0), Some(20.0)),
            Some(15.0)
        );
    }

    #[test]
    fn resolve_min_only_maps_and_filters_to_none() {
        // JS: map(value, 0, 1, min, undefined) => NaN => !isFinite => undefined.
        // This mirrors input-link-value.js exactly.
        assert_eq!(resolve_input_link_value(Some(0.5), Some(10.0), None), None);
    }

    #[test]
    fn resolve_max_only_maps_and_filters_to_none() {
        // Same reasoning, missing lower bound.
        assert_eq!(resolve_input_link_value(Some(0.5), None, Some(20.0)), None);
    }

    #[test]
    fn resolve_nan_source_is_none() {
        assert_eq!(
            resolve_input_link_value(Some(f64::NAN), Some(10.0), Some(20.0)),
            None
        );
        assert_eq!(resolve_input_link_value(Some(f64::NAN), None, None), None);
    }

    #[test]
    fn resolve_infinite_source_is_none() {
        assert_eq!(
            resolve_input_link_value(Some(f64::INFINITY), None, None),
            None
        );
        assert_eq!(
            resolve_input_link_value(Some(f64::NEG_INFINITY), None, None),
            None
        );
        assert_eq!(
            resolve_input_link_value(Some(f64::INFINITY), Some(10.0), Some(20.0)),
            None
        );
    }

    #[test]
    fn resolve_finite_without_range_passthrough() {
        assert_eq!(resolve_input_link_value(Some(0.75), None, None), Some(0.75));
        assert_eq!(
            resolve_input_link_value(Some(123.5), None, None),
            Some(123.5)
        );
    }

    #[test]
    fn resolve_zero_is_a_valid_value() {
        // Falsy-but-valid: `0` must not be treated as missing.
        assert_eq!(resolve_input_link_value(Some(0.0), None, None), Some(0.0));
        // With a range, 0 maps to min.
        assert_eq!(
            resolve_input_link_value(Some(0.0), Some(10.0), Some(20.0)),
            Some(10.0)
        );
    }

    #[test]
    fn resolve_integer_source_with_range() {
        assert_eq!(
            resolve_input_link_value(Some(1.0), Some(0.0), Some(100.0)),
            Some(100.0)
        );
    }

    #[test]
    fn input_link_kind_serializes_lowercase() {
        assert_eq!(
            serde_json::to_value(InputLinkKind::Getter).unwrap(),
            json!("getter")
        );
        assert_eq!(
            serde_json::to_value(InputLinkKind::State).unwrap(),
            json!("state")
        );
        assert_eq!(
            serde_json::to_value(InputLinkKind::Mutation).unwrap(),
            json!("mutation")
        );
    }

    #[test]
    fn input_link_round_trip_verbatim_shape() {
        let raw = json!({
            "id": "in-1",
            "type": "getter",
            "location": "meyda/getFeature",
            "args": ["energy"],
            "min": 0,
            "max": 1,
            "source": "meyda",
            "futureKey": {"nested": true}
        });
        let link: InputLink = serde_json::from_value(raw.clone()).unwrap();
        assert_eq!(link.kind, InputLinkKind::Getter);
        assert_eq!(link.args.as_ref().unwrap()[0], json!("energy"));
        assert!(link.is_getter());
        assert!(!link.is_mutation());
        let reserialized = serde_json::to_value(&link).unwrap();
        assert_eq!(reserialized, raw);
    }

    #[test]
    fn input_link_mutation_matcher_round_trip() {
        let raw = json!({
            "id": "in-2",
            "type": "mutation",
            "location": "midi.devices[0].channelData",
            "source": "midi",
            "match": {"type": "midi/WRITE_DATA", "payload": {"channel": 1, "type": 144}}
        });
        let link: InputLink = serde_json::from_value(raw.clone()).unwrap();
        assert!(link.is_mutation());
        assert_eq!(
            link.matcher.as_ref().unwrap()["type"],
            json!("midi/WRITE_DATA")
        );
        assert_eq!(serde_json::to_value(&link).unwrap(), raw);
    }

    #[test]
    fn input_link_without_optional_fields_omits_them() {
        let link = InputLink {
            id: "in-3".to_string(),
            kind: InputLinkKind::State,
            location: None,
            args: None,
            source: Some("tween".to_string()),
            min: None,
            max: None,
            matcher: None,
            extra: Map::new(),
        };
        let value = serde_json::to_value(&link).unwrap();
        let obj = value.as_object().unwrap();
        assert!(!obj.contains_key("location"));
        assert!(!obj.contains_key("args"));
        assert!(!obj.contains_key("min"));
        assert!(!obj.contains_key("max"));
        assert!(!obj.contains_key("match"));
        assert_eq!(obj.get("type").unwrap(), &json!("state"));
    }

    #[test]
    fn input_bind_side_parses_action_and_commit() {
        let action = Input {
            id: "a".to_string(),
            type_: "action".to_string(),
            data: json!({"moduleId": "m1", "prop": "x"}),
            get_location: Some("modules.active[\"m1\"].props[\"x\"]".to_string()),
            location: Some("modules/updateProp".to_string()),
        };
        assert_eq!(
            action.bind(),
            Some(LinkBind::Action {
                location: "modules/updateProp".to_string()
            })
        );

        let commit = Input {
            id: "c".to_string(),
            type_: "commit".to_string(),
            data: json!({"groupId": "g1", "key": "alpha"}),
            get_location: None,
            location: Some("groups/UPDATE_GROUP_BY_KEY".to_string()),
        };
        assert_eq!(
            commit.bind(),
            Some(LinkBind::Commit {
                location: "groups/UPDATE_GROUP_BY_KEY".to_string()
            })
        );

        let unknown = Input {
            id: "u".to_string(),
            type_: "getter".to_string(),
            data: Value::Null,
            get_location: None,
            location: Some("whatever".to_string()),
        };
        assert_eq!(unknown.bind(), None);
    }

    #[test]
    fn input_round_trip_verbatim_shape() {
        let raw = json!({
            "id": "in-1",
            "type": "action",
            "getLocation": "modules.active[\"m1\"].props[\"bandAmount\"]",
            "location": "modules/updateProp",
            "data": {"moduleId": "m1", "prop": "bandAmount"}
        });
        let input: Input = serde_json::from_value(raw.clone()).unwrap();
        assert_eq!(serde_json::to_value(&input).unwrap(), raw);
    }
}
