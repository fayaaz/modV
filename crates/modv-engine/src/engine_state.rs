//! Authoritative state container for the M2 slice of the engine.
//!
//! Ported from `src/renderer/src/application/worker/store/modules/groups.js`
//! (Group typedef, `groupIndexRenderOrder` getter, `createPresetData`),
//! `store/modules/modules.js` (ModuleInstance shape, `createPresetData`),
//! `worker/group-inherit.js` and the inherit handling in `worker/loop.js`.
//!
//! Line numbers refer to the tree at commit `8a512b6e` (branch `rust-hybrid`).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Serialize an `f64` the way JS `JSON.stringify` writes a number:
/// integral values within the safe-integer range become integers, everything
/// else stays a float.
///
/// modV persists numbers without int/float distinction — a group alpha of
/// `1` is written as `1`, not `1.0`. Keeping this behaviour makes preset
/// round trips byte-comparable with documents the JS store writes, and
/// avoids a spurious representation drift for integer-valued props.
pub(crate) fn serialize_minimal_f64<S>(value: &f64, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    const SAFE_INTEGER: f64 = 9_007_199_254_740_991.0; // 2^53 - 1
    if value.fract() == 0.0 && value.abs() <= SAFE_INTEGER {
        serializer.serialize_i64(*value as i64)
    } else {
        serializer.serialize_f64(*value)
    }
}

/// `serialize_minimal_f64` for an `Option<f64>` (absent bounds are skipped on
/// their fields anyway; `None` maps to `null` for completeness).
pub(crate) fn serialize_minimal_option_f64<S>(
    value: &Option<f64>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    match value {
        Some(n) => serialize_minimal_f64(n, serializer),
        None => serializer.serialize_none(),
    }
}

/// Group is not drawn at all (and its modules are not rendered).
/// Ported from `src/renderer/src/application/constants.js` (`GROUP_DISABLED = 0`).
pub const GROUP_DISABLED: i64 = 0;
/// Group is drawn.
/// Ported from `src/renderer/src/application/constants.js` (`GROUP_ENABLED = 1`).
pub const GROUP_ENABLED: i64 = 1;
/// Group is drawn but not composited to the output canvas.
/// Ported from `src/renderer/src/application/constants.js` (`GROUP_DRAW_TO_OUTPUT = 2`).
pub const GROUP_DRAW_TO_OUTPUT: i64 = 2;
/// The internal group the Gallery panel draws into.
/// Ported from `src/renderer/src/application/constants.js`
/// (`GALLERY_GROUP_NAME`, used by `groups.js:261` and `loop.js:251`).
pub const GALLERY_GROUP_NAME: &str = "VmodV internal Gallery Group";

/// A Group: the render-graph row in the layer stack.
///
/// Mirrors the typedef at `store/modules/groups.js:8-81`. Group objects are
/// plain JSON in the store; unknown keys are preserved verbatim in
/// [`Group::extra`] so a load → save round trip loses nothing that the JS
/// shallow-spread clone kept (e.g. keys written by a newer build).
///
/// `context` (the auxiliary output record), `writeToSwap`/`readFromSwap`
/// (action arguments wrongly persistable in JS) and `drawToCanvasId` are
/// dropped on preset save — see [`Group::for_preset_save`].
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Group {
    /// UUID of the Group.
    pub id: String,
    /// Name of the Group.
    pub name: String,
    /// Whether this group is hidden.
    pub hidden: bool,
    /// Draw order of the Module ids contained within the Group.
    pub modules: Vec<String>,
    /// 0 = not drawn, 1 = drawn, 2 = drawn but not to the output canvas.
    pub enabled: i64,
    /// Opacity between 0 and 1.
    #[serde(serialize_with = "serialize_minimal_f64")]
    pub alpha: f64,
    /// Whether the Group should inherit from another Group at redraw.
    pub inherit: bool,
    /// Which Group to inherit from. `-1` = the previous Group in
    /// `groupIndexRenderOrder`; a UUID string = that specific Group.
    /// See [`InheritFrom`].
    #[serde(rename = "inheritFrom", default)]
    pub inherit_from: InheritFrom,
    /// Whether the Group renders using pipeline mode.
    pub pipeline: bool,
    /// Whether the Group clears its target before redraw.
    pub clearing: bool,
    /// The blend mode token the Group composites with
    /// (see [`crate::blend::BlendMode`]).
    #[serde(rename = "compositeOperation")]
    pub composite_operation: String,
    /// The id of the Scene this Group belongs to, stamped by the scene
    /// machinery at runtime (`scenes.js:1994`). Presets carry it too.
    /// Serialised as `sceneId` (the JS key). Omitted when absent.
    #[serde(rename = "sceneId", default, skip_serializing_if = "Option::is_none")]
    pub scene_id: Option<String>,
    /// The row of the scene grid this Group renders in (`scenes.js:1995`).
    /// Serialised as `sceneRow` (the JS key). Omitted when absent.
    #[serde(rename = "sceneRow", default, skip_serializing_if = "Option::is_none")]
    pub scene_row: Option<usize>,
    /// Set on the outgoing Group kept alive during a scene row fade
    /// (`groups.js:261`). Groups with this flag are dropped from preset
    /// saves entirely. Serialised as `transitionGhost` (the JS key).
    /// Omitted when absent.
    #[serde(
        rename = "transitionGhost",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub transition_ghost: Option<bool>,
    /// The id of the auxiliary canvas the Group draws to; `null`/absent draws
    /// to the main output. Present on live groups but dropped on preset save.
    /// Serialised as `drawToCanvasId` (the JS key).
    #[serde(
        rename = "drawToCanvasId",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub draw_to_canvas_id: Option<String>,
    /// Input id bound to the Alpha control. Serialised as `alphaInputId`.
    #[serde(
        rename = "alphaInputId",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub alpha_input_id: Option<String>,
    /// Input id bound to the Enabled control. Serialised as `enabledInputId`.
    #[serde(
        rename = "enabledInputId",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub enabled_input_id: Option<String>,
    /// Input id bound to the Clearing control. Serialised as `clearingInputId`.
    #[serde(
        rename = "clearingInputId",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub clearing_input_id: Option<String>,
    /// Input id bound to the Inherit control. Serialised as `inheritInputId`.
    #[serde(
        rename = "inheritInputId",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub inherit_input_id: Option<String>,
    /// Input id bound to the Composite Operation control.
    /// Serialised as `compositeOperationInputId`.
    #[serde(
        rename = "compositeOperationInputId",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub composite_operation_input_id: Option<String>,
    /// Input id bound to the Pipeline control.
    /// Serialised as `pipelineInputId`.
    #[serde(
        rename = "pipelineInputId",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub pipeline_input_id: Option<String>,
    /// Unknown keys, kept verbatim for round-trip fidelity. `context`,
    /// `writeToSwap`, `readFromSwap` and `drawToCanvasId` never reach this
    /// map via [`Group::for_preset_save`]; they land here on load only if a
    /// document carried them.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Group {
    /// Serialise the Group the way `groups.js:263-269` does for a preset:
    /// everything except the non-serialisable `context` record and the
    /// `writeToSwap`/`readFromSwap` action arguments. `drawToCanvasId` is
    /// also dropped (it may be present on live groups but must not persist).
    ///
    /// Note: the Gallery-group and `transitionGhost` filters are applied by
    /// the caller (`crate::preset::create_preset_from_state`), matching
    /// `groups.js:257-262`.
    pub fn for_preset_save(&self) -> Value {
        match serde_json::to_value(self) {
            Ok(Value::Object(mut map)) => {
                map.remove("context");
                map.remove("writeToSwap");
                map.remove("readFromSwap");
                map.remove("drawToCanvasId");
                Value::Object(map)
            }
            // A Group always serialises to an object; fall back defensively.
            Ok(other) => other,
            Err(_) => Value::Object(Map::new()),
        }
    }
}

/// The `inheritFrom` target of a Group.
///
/// JSON carries either a number (`-1` for "the previous Group in
/// `groupIndexRenderOrder`") or a string (the UUID of another Group).
/// Any other number is accepted as `Previous(n)` and behaves like `Previous`
/// with the given index offset — the JS only distinguishes `-1` from other
/// values by accident of its `=== -1` comparison in `group-inherit.js`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum InheritFrom {
    /// Numeric inherit target. `-1` means the previous Group in render order.
    Previous(i64),
    /// UUID of a specific Group in the state.
    Group(String),
}

impl Default for InheritFrom {
    fn default() -> Self {
        InheritFrom::Previous(-1)
    }
}

impl InheritFrom {
    /// `true` iff this is the `-1` ("previous in render order") sentinel.
    pub fn is_previous(&self) -> bool {
        matches!(self, InheritFrom::Previous(-1))
    }
}

/// A live module instance in the active modules map.
///
/// Mirrors `store/modules/modules.js:660-683` (`createPresetData`): the
/// instance is a shallow clone of the runtime module object with `$status`
/// dropped on save and gallery modules filtered out. Unknown keys are kept
/// verbatim in [`ModuleInstance::extra`] for round-trip fidelity.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ModuleInstance {
    /// UUID of the module instance (`$id`).
    #[serde(rename = "$id")]
    pub id: String,
    /// Registry key of the module definition, e.g. `"Bands"` (`$moduleName`).
    #[serde(rename = "$moduleName")]
    pub module_name: String,
    /// Module metadata (`name`, `author`, `version`, `type`, `alpha`,
    /// `enabled`, `compositeOperation`, `isGallery`, ...). Kept as raw JSON:
    /// the typed model lands with WS-12.
    #[serde(default)]
    pub meta: Value,
    /// Raw prop values keyed by prop name (`{ propName: rawValue }`).
    /// Values stay raw JSON until the typed prop model lands (WS-12).
    #[serde(default)]
    pub props: Value,
    /// Renderer-specific, JSON-safe data blob.
    #[serde(default)]
    pub data: Value,
    /// Bound meta inputs: `{ prop: { id, type, ... } }` (`$props`).
    // `$props` is not a valid Rust identifier; the serde name is verbatim.
    #[serde(default, rename = "$props")]
    pub dollar_props: Value,
    /// Live module error/warning status entries. Dropped on preset save
    /// (`modules.js:672`). Omitted from serialisation when empty; even when
    /// non-empty, [`ModuleInstance::for_preset_save`] removes it.
    #[serde(default, rename = "$status", skip_serializing_if = "Vec::is_empty")]
    pub status: Vec<Value>,
    /// Unknown keys, kept verbatim for round-trip fidelity.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl ModuleInstance {
    /// Serialise the module the way `modules.js:671-681` does for a preset:
    /// a shallow clone without `$status` (gallery modules are filtered by the
    /// caller — see [`crate::preset::create_preset_from_state`]).
    pub fn for_preset_save(&self) -> Value {
        match serde_json::to_value(self) {
            Ok(Value::Object(mut map)) => {
                map.remove("$status");
                Value::Object(map)
            }
            Ok(other) => other,
            Err(_) => Value::Object(Map::new()),
        }
    }
}

/// The authoritative M2 state container.
///
/// `inputs` holds the bound [`crate::inputs::Input`] records keyed by input
/// id; `input_links` the [`crate::inputs::InputLink`] records keyed by the
/// same input id. The on-disk preset shape is
/// `inputs: { focusedInput, inputs, inputLinks }`; the focused input is kept
/// as an opaque `{ id, title }` pair in `focused_input` so the round trip is
/// lossless. `tweens` and `midi` are kept verbatim as raw JSON (WS-12 ports
/// them for real).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct EngineState {
    /// Groups in definition order (render order differs — see
    /// [`EngineState::group_index_render_order`]).
    pub groups: Vec<Group>,
    /// Active module instances keyed by `$id`.
    pub modules: indexmap::IndexMap<String, ModuleInstance>,
    /// Bound inputs keyed by input id.
    pub inputs: indexmap::IndexMap<String, crate::inputs::Input>,
    /// Input links keyed by the id of the input they write to.
    pub input_links: indexmap::IndexMap<String, crate::inputs::InputLink>,
    /// Expression assignments keyed by expression id
    /// (`expressions.js` state shape: `{ assignments: {...} }`).
    pub expressions: indexmap::IndexMap<String, crate::preset::ExpressionAssignment>,
    /// The focused input (`{ id, title }`), kept opaque.
    pub focused_input: Value,
    /// Tween array, verbatim. Ported properly with WS-12.
    pub tweens: Value,
    /// MIDI state object, verbatim. The UI owns the device model;
    /// porting lands with WS-11.
    pub midi: Value,
}

impl EngineState {
    /// An empty state with empty opaque slots: tweens default to `[]` and
    /// midi to `{}` (not `{"devices": {}, "learning": false}`), matching the
    /// preset loader's missing-key defaults.
    pub fn new() -> Self {
        Self {
            groups: Vec::new(),
            modules: indexmap::IndexMap::new(),
            inputs: indexmap::IndexMap::new(),
            input_links: indexmap::IndexMap::new(),
            expressions: indexmap::IndexMap::new(),
            focused_input: Value::Null,
            tweens: Value::Array(Vec::new()),
            midi: Value::Object(Map::new()),
        }
    }

    /// Index of the Gallery Group in `groups`, if present.
    /// Port of `groups.js:112-115` (`findIndex` on the name).
    pub fn find_gallery_index(&self) -> Option<usize> {
        self.groups
            .iter()
            .position(|group| group.name == GALLERY_GROUP_NAME)
    }

    /// Find a Group by id.
    /// Port of `groups.js` `getById` getter (`groups.js:125-128`).
    pub fn find_group(&self, id: &str) -> Option<&Group> {
        self.groups.iter().find(|group| group.id == id)
    }

    /// Find a Group by id, mutably.
    pub fn find_group_mut(&mut self, id: &str) -> Option<&mut Group> {
        self.groups.iter_mut().find(|group| group.id == id)
    }

    /// Group indexes in render order. The Gallery Group, when present, is
    /// moved to the end.
    ///
    /// Port of the `groupIndexRenderOrder` getter at `groups.js:111-123`:
    /// callers must not assume the last entry is the Gallery Group
    /// (`loop.js:630` skips it by name for gallery-less layouts).
    pub fn group_index_render_order(&self) -> Vec<usize> {
        let mut indexes: Vec<usize> = (0..self.groups.len()).collect();

        if let Some(gallery_index) = self.find_gallery_index() {
            indexes.splice(gallery_index..gallery_index + 1, []);
            indexes.push(gallery_index);
        }

        indexes
    }

    /// Resolve which Group a `inheritFrom` target reads its canvas from.
    ///
    /// Port of `worker/group-inherit.js:10-28` adapted to records:
    ///
    /// - `InheritFrom::Previous(_)` → the previous Group in render order.
    ///   When `rendered` carries the ids of the Groups that drew this frame
    ///   (insertion order = draw order, as in `loop.js:241`/`renderedCanvasByGroupId`),
    ///   the most recent *drawn* Group before `current_render_pos` is
    ///   returned — the JS `lastCanvas` starts as `null` and is only set by
    ///   groups that actually draw, so culled groups never become the inherit
    ///   source. When `rendered` is empty (no frame context — e.g. unit
    ///   tests or save-time calls) it falls back to the positional previous
    ///   Group. At the start of render order the result is always `None`:
    ///   the first Group of a frame inherits nothing, never the previous
    ///   frame's root canvas (`loop.js:239-241,333-374`).
    /// - `InheritFrom::Group(id)` → the Group with that id, regardless of
    ///   `rendered` — the JS falls back to the Group's persistent auxiliary
    ///   canvas when it did not draw this frame
    ///   (`renderedCanvasByGroupId[id] || inheritedGroup.context.context.canvas`).
    ///
    /// `rendered` exists so the caller (M2b, the render loop) can distinguish
    /// "drew this frame" from "persistent record only"; M2b decides the
    /// canvas fallback. For M2a the returned record is the persistent one.
    pub fn resolve_inherit_source<'a>(
        &'a self,
        inherit_from: &InheritFrom,
        current_render_pos: usize,
        rendered: &'a indexmap::IndexMap<String, ()>,
    ) -> Option<&'a Group> {
        let order = self.group_index_render_order();

        match inherit_from {
            InheritFrom::Previous(_) => {
                if current_render_pos == 0 {
                    return None;
                }
                if rendered.is_empty() {
                    // No frame context: positional previous in render order.
                    let &index = order.get(current_render_pos - 1)?;
                    return Some(&self.groups[index]);
                }
                // Frame context: last drawn Group before this one in render order.
                for &index in order[..current_render_pos].iter().rev() {
                    let group = &self.groups[index];
                    if rendered.contains_key(&group.id) {
                        return Some(group);
                    }
                }
                None
            }
            InheritFrom::Group(id) => self.find_group(id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn group(id: &str, name: &str) -> Group {
        Group {
            id: id.to_string(),
            name: name.to_string(),
            hidden: false,
            modules: Vec::new(),
            enabled: GROUP_ENABLED,
            alpha: 1.0,
            inherit: true,
            inherit_from: InheritFrom::Previous(-1),
            pipeline: false,
            clearing: false,
            composite_operation: "normal".to_string(),
            scene_id: None,
            scene_row: None,
            transition_ghost: None,
            draw_to_canvas_id: None,
            alpha_input_id: None,
            enabled_input_id: None,
            clearing_input_id: None,
            inherit_input_id: None,
            composite_operation_input_id: None,
            pipeline_input_id: None,
            extra: Map::new(),
        }
    }

    #[test]
    fn inherit_from_round_trips_number_and_string() {
        let previous = InheritFrom::Previous(-1);
        let uuid = InheritFrom::Group("5d9f4b36-8fd0-4a5e-bc1b-db5d0d8a0a01".to_string());
        assert_eq!(serde_json::to_value(&previous).unwrap(), json!(-1));
        assert_eq!(
            serde_json::to_value(&uuid).unwrap(),
            json!("5d9f4b36-8fd0-4a5e-bc1b-db5d0d8a0a01")
        );
        assert_eq!(
            serde_json::from_value::<InheritFrom>(json!(-1)).unwrap(),
            previous
        );
        assert_eq!(
            serde_json::from_value::<InheritFrom>(json!("5d9f4b36-8fd0-4a5e-bc1b-db5d0d8a0a01"))
                .unwrap(),
            uuid
        );
        assert_eq!(InheritFrom::default(), InheritFrom::Previous(-1));
        assert!(InheritFrom::Previous(-1).is_previous());
        assert!(!InheritFrom::Group("x".to_string()).is_previous());
    }

    #[test]
    fn group_omits_optional_scene_fields_when_absent() {
        let value = serde_json::to_value(group("g1", "Layer 1")).unwrap();
        let obj = value.as_object().unwrap();
        assert!(!obj.contains_key("scene_id"));
        assert!(!obj.contains_key("sceneRow"));
        assert!(!obj.contains_key("transitionGhost"));
        assert!(!obj.contains_key("context"));
        assert_eq!(obj.get("inheritFrom").unwrap(), &json!(-1));
        assert_eq!(obj.get("compositeOperation").unwrap(), &json!("normal"));
    }

    #[test]
    fn group_for_preset_save_drops_runtime_keys() {
        let mut g = group("g1", "Effects");
        g.extra
            .insert("context".to_string(), json!({"id": "aux-1"}));
        g.extra.insert("writeToSwap".to_string(), json!(true));
        g.extra.insert("readFromSwap".to_string(), json!(false));
        g.draw_to_canvas_id = Some("aux-b".to_string());
        g.extra
            .insert("futureProp".to_string(), json!({"nested": [1, 2, 3]}));

        let saved = g.for_preset_save();
        let obj = saved.as_object().unwrap();
        assert!(!obj.contains_key("context"));
        assert!(!obj.contains_key("writeToSwap"));
        assert!(!obj.contains_key("readFromSwap"));
        assert!(!obj.contains_key("drawToCanvasId"));
        // Unknown keys are preserved (JS shallow-spread behaviour).
        assert_eq!(
            obj.get("futureProp").unwrap(),
            &json!({"nested": [1, 2, 3]})
        );
    }

    #[test]
    fn group_round_trip_preserves_unknown_keys() {
        let original = json!({
            "id": "g1",
            "name": "Foreground",
            "hidden": false,
            "modules": ["m1", "m2"],
            "enabled": 2,
            "alpha": 0.5,
            "inherit": false,
            "inheritFrom": "g0",
            "pipeline": true,
            "clearing": true,
            "compositeOperation": "source-over",
            "alphaInputId": "in-alpha",
            "pipelineInputId": "in-pipeline",
            "sceneId": "scene-9",
            "sceneRow": 3,
            "mysteryKey": {"deep": [true, null]}
        });
        let parsed: Group = serde_json::from_value(original.clone()).unwrap();
        assert_eq!(parsed.inherit_from, InheritFrom::Group("g0".to_string()));
        assert_eq!(parsed.scene_row, Some(3));
        let reserialized = serde_json::to_value(&parsed).unwrap();
        assert_eq!(reserialized, original);
    }

    #[test]
    fn render_order_moves_gallery_to_end() {
        let mut state = EngineState::new();
        state.groups = vec![
            group("a", "Layer A"),
            group("gal", GALLERY_GROUP_NAME),
            group("b", "Layer B"),
        ];
        assert_eq!(state.group_index_render_order(), vec![0, 2, 1]);
        assert_eq!(state.find_gallery_index(), Some(1));
    }

    #[test]
    fn render_order_without_gallery_is_identity() {
        let mut state = EngineState::new();
        state.groups = vec![group("a", "A"), group("b", "B")];
        assert_eq!(state.group_index_render_order(), vec![0, 1]);
        assert_eq!(state.find_gallery_index(), None);
    }

    #[test]
    fn render_order_gallery_already_last() {
        let mut state = EngineState::new();
        state.groups = vec![group("a", "A"), group("gal", GALLERY_GROUP_NAME)];
        assert_eq!(state.group_index_render_order(), vec![0, 1]);
    }

    #[test]
    fn find_group_matches_by_id() {
        let mut state = EngineState::new();
        state.groups = vec![group("a", "A"), group("b", "B")];
        assert_eq!(state.find_group("b").map(|g| g.name.as_str()), Some("B"));
        assert!(state.find_group("missing").is_none());
        state.find_group_mut("b").unwrap().hidden = true;
        assert!(state.find_group("b").unwrap().hidden);
    }

    #[test]
    fn resolve_inherit_source_first_group_inherits_nothing() {
        let mut state = EngineState::new();
        state.groups = vec![group("a", "A"), group("b", "B")];
        let rendered = indexmap::IndexMap::new();
        let source = state.resolve_inherit_source(&InheritFrom::Previous(-1), 0, &rendered);
        assert!(source.is_none());
    }

    #[test]
    fn resolve_inherit_source_previous_uses_render_order() {
        let mut state = EngineState::new();
        state.groups = vec![
            group("a", "A"),
            group("gal", GALLERY_GROUP_NAME),
            group("b", "B"),
        ];
        let rendered = indexmap::IndexMap::new();
        // Render order is [a(0), b(2), gal(1)]: positions 0, 1, 2 map to
        // groups a, b, gal. Position 2's previous in render order is b —
        // not the gallery at raw index 1.
        let source = state.resolve_inherit_source(&InheritFrom::Previous(-1), 2, &rendered);
        assert_eq!(source.map(|g| g.id.as_str()), Some("b"));
        let source = state.resolve_inherit_source(&InheritFrom::Previous(-1), 1, &rendered);
        assert_eq!(source.map(|g| g.id.as_str()), Some("a"));
    }

    #[test]
    fn resolve_inherit_source_group_by_id() {
        let mut state = EngineState::new();
        state.groups = vec![group("a", "A"), group("b", "B")];
        let rendered = indexmap::IndexMap::new();
        let source =
            state.resolve_inherit_source(&InheritFrom::Group("a".to_string()), 1, &rendered);
        assert_eq!(source.map(|g| g.name.as_str()), Some("A"));
        let missing =
            state.resolve_inherit_source(&InheritFrom::Group("nope".to_string()), 1, &rendered);
        assert!(missing.is_none());
    }

    #[test]
    fn resolve_inherit_source_prefers_drawn_group_when_frame_context_provided() {
        let mut state = EngineState::new();
        state.groups = vec![group("a", "A"), group("b", "B"), group("c", "C")];
        let mut rendered = indexmap::IndexMap::new();
        rendered.insert("a".to_string(), ());
        // b did NOT draw this frame; c inherits from a, not b.
        let source = state.resolve_inherit_source(&InheritFrom::Previous(-1), 2, &rendered);
        assert_eq!(source.map(|g| g.id.as_str()), Some("a"));
    }

    #[test]
    fn resolve_inherit_source_frame_context_with_only_undrawn_predecessors_is_none() {
        let mut state = EngineState::new();
        state.groups = vec![group("a", "A"), group("b", "B")];
        let mut rendered = indexmap::IndexMap::new();
        rendered.insert("c-unrelated".to_string(), ());
        let source = state.resolve_inherit_source(&InheritFrom::Previous(-1), 1, &rendered);
        assert!(source.is_none());
    }

    #[test]
    fn module_instance_omits_empty_status_and_round_trips() {
        let raw = json!({
            "$id": "m1",
            "$moduleName": "Bands",
            "meta": {"name": "Bands", "type": "shader", "alpha": 1, "enabled": true},
            "props": {"bandAmount": 3},
            "data": {"seed": 42},
            "$props": {"bandAmount": {"id": "in-1", "type": "float"}},
            "customField": "keep-me"
        });
        let module: ModuleInstance = serde_json::from_value(raw.clone()).unwrap();
        assert!(module.status.is_empty());
        assert_eq!(module.dollar_props["bandAmount"]["id"], json!("in-1"));
        let reserialized = serde_json::to_value(&module).unwrap();
        assert_eq!(reserialized, raw);
    }

    #[test]
    fn module_instance_for_preset_save_drops_status() {
        let raw = json!({
            "$id": "m1",
            "$moduleName": "Bands",
            "meta": {"name": "Bands"},
            "$status": [{"type": "error", "message": "boom"}]
        });
        let module: ModuleInstance = serde_json::from_value(raw).unwrap();
        let saved = module.for_preset_save();
        assert!(!saved.as_object().unwrap().contains_key("$status"));
    }

    #[test]
    fn engine_state_new_has_empty_opaque_slots() {
        let state = EngineState::new();
        assert!(state.groups.is_empty());
        assert!(state.modules.is_empty());
        assert_eq!(state.tweens, json!([]));
        assert_eq!(state.midi, json!({}));
    }
}
