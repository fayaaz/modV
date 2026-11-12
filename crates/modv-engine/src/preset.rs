//! Preset / Scene / playlist (de)serialisation.
//!
//! Ported from `worker/preset-serializer.js` (110 lines): the six-key preset
//! document, `isSceneDocument` / `isScenesPlaylist` / the playlist type
//! token, and the round trip through [`EngineState`].
//!
//! ## The six-key format
//!
//! ```json
//! {
//!   "groups": [ ...Group ],
//!   "modules": { "<$id>": { "$id", "$moduleName", "meta", "props", "data", "$props" } },
//!   "inputs": { "focusedInput": { "id", "title" }, "inputs": {...}, "inputLinks": {...} },
//!   "expressions": { "assignments": {...} },
//!   "tweens": [ ... ],
//!   "midi": { "devices": {...}, "learning": false }
//! }
//! ```
//!
//! Key order is not required (deep equality only). The format is unchanged —
//! this is a hard compatibility requirement (WS-13).
//!
//! ## Behaviour notes
//!
//! - `load_preset_into_state` **replaces** the six keys in the target state.
//!   The live-vs-swap orchestration (gallery preservation via
//!   [`crate::swap`]) happens in the caller (M2b); loading into a scratch
//!   state and swapping is the JS flow (`preset-serializer.js:76-83`).
//! - Scene metadata (`id`, `name`, `enabled`, `minSeconds`, `maxSeconds`,
//!   `timingOverride`, ...) is **not** part of the preset format: it is kept
//!   verbatim in [`PresetLoadInfo::metadata`] on load. Re-attaching it when
//!   saving a Scene is the scenes store's job (M5) — `create_preset_from_state`
//!   produces the six-key document only, exactly like the JS
//!   `createPresetData`.
//! - Scene documents may carry `null` holes in `groups` (rows the Scene does
//!   not define). Those are rejected loudly ([`PresetError::Decode`]) rather
//!   than silently mangled — the JS loader turned them into empty
//!   "New Group"s. Row-faithful scene loading is M5 scope.
//! - `groups` drop `context`/`writeToSwap`/`readFromSwap`/`drawToCanvasId`,
//!   the Gallery group and `transitionGhost` groups on save
//!   (`groups.js:257-269`, `modules.js:663-672`).
//! - Textures in prop values are raw descriptors
//!   (`{ type, options, ... }`); nothing here resolves them.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::engine_state::{
    EngineState, GALLERY_GROUP_NAME, Group, ModuleInstance, normalise_module_meta_value,
};
use crate::inputs::{Input, InputLink};

/// The playlist document type token (`preset-serializer.js:3`).
pub const SCENES_PLAYLIST_TYPE: &str = "modv-scenes-playlist";

/// A preset document broken into its typed pieces plus out-of-band metadata.
///
/// Intermediate representation used by [`load_preset_into_state`]; exported
/// so M2b can build a scratch state (or inspect a document) without
/// mutating anything.
#[derive(Clone, Debug, Default)]
pub struct PresetDocument {
    /// The `groups` array. Scene documents may carry `null` row holes — those
    /// are **rejected** with a decode error here (JS silently turned them into
    /// empty "New Group"s); supporting scene application lands with M5.
    pub groups: Vec<Group>,
    /// The `modules` object keyed by module `$id`.
    pub modules: IndexMap<String, ModuleInstance>,
    /// The `inputs.inputs` object keyed by input id.
    pub inputs: IndexMap<String, Input>,
    /// The `inputs.inputLinks` object keyed by input id.
    pub input_links: IndexMap<String, InputLink>,
    /// The `inputs.focusedInput` object, kept opaque (`{ id, title }`).
    pub focused_input: Value,
    /// The `expressions.assignments` object keyed by expression id.
    pub expressions: IndexMap<String, ExpressionAssignment>,
    /// The `tweens` array, kept verbatim.
    pub tweens: Value,
    /// The `midi` object, kept verbatim.
    pub midi: Value,
    /// Top-level keys that are not part of the six-key format (Scene id,
    /// name, timing, ...). Kept verbatim; see module docs.
    pub metadata: Map<String, Value>,
}

/// What `load_preset_into_state` learned about the document besides the
/// six-key payload.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PresetLoadInfo {
    /// Top-level keys outside the six-key format, verbatim. Empty for a
    /// plain preset; carries `id`/`name`/timing for a Scene document.
    pub metadata: Map<String, Value>,
    /// `preset-serializer.js:88` — an `id` non-empty string, a `name`
    /// string, and `groups` or `layers` an array.
    pub is_scene_document: bool,
    /// `preset-serializer.js:103` — `type === "modv-scenes-playlist"` and
    /// `scenes` is an array.
    pub is_scenes_playlist: bool,
}

/// Errors loading a preset document.
#[derive(Clone, Debug, thiserror::Error, PartialEq)]
pub enum PresetError {
    /// The document itself is not a JSON object.
    #[error("preset document is not an object")]
    NotAnObject,
    /// A known key has a shape incompatible with the format
    /// (e.g. `groups` present but not an array). Absent keys are fine.
    #[error("preset key `{0}` has the wrong shape")]
    InvalidShape(&'static str),
    /// A known key could not be decoded into its typed representation.
    #[error("failed to decode preset key `{key}`: {message}")]
    Decode {
        /// Which key failed.
        key: &'static str,
        /// The serde error message.
        message: String,
    },
}

impl PresetError {
    fn decode(key: &'static str, error: serde_json::Error) -> Self {
        PresetError::Decode {
            key,
            message: error.to_string(),
        }
    }
}

/// An expression assignment (`expressions.js` `{ assignment }` object).
///
/// `func` (the compiled JS function) never survives the JSON round trip in
/// the legacy format either — `createPresetData`'s `JSON.parse(JSON.stringify)`
/// clone drops it — so only JSON-safe fields are modelled. Unknown keys are
/// kept in `extra`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ExpressionAssignment {
    /// Expression id.
    pub id: String,
    /// Id of the input the expression writes to.
    #[serde(rename = "inputId")]
    pub input_id: String,
    /// The expression source.
    #[serde(default)]
    pub expression: String,
    /// Whether the expression re-evaluates every frame.
    #[serde(default, rename = "evaluateEveryFrame")]
    pub evaluate_every_frame: bool,
    /// Unknown keys, kept verbatim.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// `preset-serializer.js:88-97` — a Scene document is a preset document plus
/// `id` (non-empty string), `name` (string) and `groups` or `layers` (array).
///
/// Legacy `layers` documents are normalised later (M5); here we only detect.
pub fn is_scene_document(value: &Value) -> bool {
    let Some(obj) = value.as_object() else {
        return false;
    };

    match (obj.get("id"), obj.get("name")) {
        (Some(Value::String(id)), Some(Value::String(_))) if !id.is_empty() => {}
        _ => return false,
    }

    obj.get("groups").is_some_and(Value::is_array) || obj.get("layers").is_some_and(Value::is_array)
}

/// The playlist type check from `preset-serializer.js:103-110`
/// (`value.type === SCENES_PLAYLIST_TYPE`), without the `scenes` array check.
pub fn is_scenes_playlist_type(value: &Value) -> bool {
    value
        .as_object()
        .and_then(|obj| obj.get("type"))
        .is_some_and(|t| t == SCENES_PLAYLIST_TYPE)
}

/// `preset-serializer.js:103-110` — a Scenes playlist is a document with
/// `type === "modv-scenes-playlist"` and `scenes` an array.
pub fn is_scenes_playlist(value: &Value) -> bool {
    is_scenes_playlist_type(value) && value.get("scenes").is_some_and(Value::is_array)
}

fn decode_optional_object<T>(
    key: &'static str,
    target: Option<&Value>,
    collection: &mut IndexMap<String, T>,
    collect_by: impl Fn(&T) -> String,
) -> Result<(), PresetError>
where
    T: for<'de> Deserialize<'de>,
{
    let Some(value) = target else { return Ok(()) };
    if value.is_null() {
        return Ok(());
    }
    let Some(object) = value.as_object() else {
        return Err(PresetError::InvalidShape(key));
    };

    for (_map_key, raw) in object {
        let decoded: T =
            serde_json::from_value(raw.clone()).map_err(|e| PresetError::decode(key, e))?;
        collection.insert(collect_by(&decoded), decoded);
    }
    Ok(())
}

/// Parse a preset document into its typed pieces, keeping out-of-band
/// metadata aside. Missing keys default to empty collections; a present but
/// wrong-shaped key is a [`PresetError`].
pub fn parse_preset(preset: &Value) -> Result<PresetDocument, PresetError> {
    let Some(obj) = preset.as_object() else {
        return Err(PresetError::NotAnObject);
    };

    let mut document = PresetDocument::default();

    // groups: array of Group (absent/null = empty).
    match obj.get("groups") {
        None | Some(Value::Null) => {}
        Some(Value::Array(entries)) => {
            for entry in entries {
                let group: Group = serde_json::from_value(entry.clone())
                    .map_err(|e| PresetError::decode("groups", e))?;
                document.groups.push(group);
            }
        }
        Some(_) => return Err(PresetError::InvalidShape("groups")),
    }

    // modules: object keyed by module $id (absent/null = empty).
    match obj.get("modules") {
        None | Some(Value::Null) => {}
        Some(Value::Object(modules)) => {
            for raw in modules.values() {
                let mut module: ModuleInstance = serde_json::from_value(raw.clone())
                    .map_err(|e| PresetError::decode("modules", e))?;
                normalise_module_meta_value(&mut module.meta);
                document.modules.insert(module.id.clone(), module);
            }
        }
        Some(_) => return Err(PresetError::InvalidShape("modules")),
    }

    // inputs: { focusedInput, inputs, inputLinks } (absent/null = empty).
    match obj.get("inputs") {
        None | Some(Value::Null) => {}
        Some(Value::Object(inputs)) => {
            document.focused_input = inputs
                .get("focusedInput")
                .cloned()
                .unwrap_or_else(|| Value::Object(Map::new()));

            decode_optional_object(
                "inputs",
                inputs.get("inputs"),
                &mut document.inputs,
                |input: &Input| input.id.clone(),
            )?;
            decode_optional_object(
                "inputLinks",
                inputs.get("inputLinks"),
                &mut document.input_links,
                |link: &InputLink| link.id.clone(),
            )?;
        }
        Some(_) => return Err(PresetError::InvalidShape("inputs")),
    }

    // expressions: { assignments } (absent/null = empty).
    match obj.get("expressions") {
        None | Some(Value::Null) => {}
        Some(Value::Object(expressions)) => {
            decode_optional_object(
                "expressions",
                expressions.get("assignments"),
                &mut document.expressions,
                |assignment: &ExpressionAssignment| assignment.id.clone(),
            )?;
        }
        Some(_) => return Err(PresetError::InvalidShape("expressions")),
    }

    // tweens: array, verbatim (absent/null = empty array).
    match obj.get("tweens") {
        None | Some(Value::Null) => document.tweens = Value::Array(Vec::new()),
        Some(array @ Value::Array(_)) => document.tweens = array.clone(),
        Some(_) => return Err(PresetError::InvalidShape("tweens")),
    }

    // midi: object, verbatim (absent/null = empty object).
    match obj.get("midi") {
        None | Some(Value::Null) => document.midi = Value::Object(Map::new()),
        Some(object @ Value::Object(_)) => document.midi = object.clone(),
        Some(_) => return Err(PresetError::InvalidShape("midi")),
    }

    // Everything else is Scene metadata (or a future key) — kept verbatim.
    for (key, value) in obj {
        if !matches!(
            key.as_str(),
            "groups" | "modules" | "inputs" | "expressions" | "tweens" | "midi"
        ) {
            document.metadata.insert(key.clone(), value.clone());
        }
    }

    Ok(document)
}

/// Load a preset document into `state`, replacing its six-key contents.
///
/// Scene/playlist metadata is returned in [`PresetLoadInfo`] and left out of
/// `state` (see module docs). This does **not** perform the live gallery
/// preservation — call it on a scratch state and `swap` (see
/// [`crate::swap`]), mirroring the JS preset-load flow.
pub fn load_preset_into_state(
    state: &mut EngineState,
    preset: &Value,
) -> Result<PresetLoadInfo, PresetError> {
    let document = parse_preset(preset)?;

    let info = PresetLoadInfo {
        metadata: document.metadata,
        is_scene_document: is_scene_document(preset),
        is_scenes_playlist: is_scenes_playlist(preset),
    };

    state.groups = document.groups;
    state.modules = document.modules;
    state.inputs = document.inputs;
    state.input_links = document.input_links;
    state.focused_input = document.focused_input;
    state.expressions = document.expressions;
    state.tweens = document.tweens;
    state.midi = document.midi;

    Ok(info)
}

/// Assemble the six-key preset document for `state`.
///
/// Filters and drops, mirroring the JS `createPresetData` actions:
///
/// - `groups`: drops the Gallery Group and `transitionGhost === true`
///   groups, then drops `context`/`writeToSwap`/`readFromSwap`/
///   `drawToCanvasId` per group (`groups.js:257-269`).
/// - `modules`: drops modules with `meta.isGallery === true`, then drops
///   `$status` per module (`modules.js:663-672`). Renderer `createPresetData`
///   merges (ISF/texture bookkeeping) are **not** ported in M2a.
/// - `inputs`/`expressions`: verbatim (`focusedInput`/`inputs`/`inputLinks`;
///   `assignments`).
/// - `tweens`/`midi`: verbatim.
///
/// Scene metadata is *not* attached here — saving a Scene re-adds `id`,
/// `name` and timing on top of this document (M5), as the JS scenes store
/// does.
pub fn create_preset_from_state(state: &EngineState) -> Value {
    let groups: Vec<Value> = state
        .groups
        .iter()
        .filter(|group| group.name != GALLERY_GROUP_NAME && group.transition_ghost != Some(true))
        .map(Group::for_preset_save)
        .collect();

    let mut modules = Map::new();
    for (id, module) in &state.modules {
        if module.meta.get("isGallery").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        modules.insert(id.clone(), module.for_preset_save());
    }

    let mut inputs = Map::new();
    inputs.insert("focusedInput".to_string(), state.focused_input.clone());
    inputs.insert(
        "inputs".to_string(),
        serde_json::to_value(&state.inputs).unwrap_or_else(|_| Value::Object(Map::new())),
    );
    inputs.insert(
        "inputLinks".to_string(),
        serde_json::to_value(&state.input_links).unwrap_or_else(|_| Value::Object(Map::new())),
    );

    let mut expressions = Map::new();
    expressions.insert(
        "assignments".to_string(),
        serde_json::to_value(&state.expressions).unwrap_or_else(|_| Value::Object(Map::new())),
    );

    let mut preset = Map::new();
    preset.insert("groups".to_string(), Value::Array(groups));
    preset.insert("modules".to_string(), Value::Object(modules));
    preset.insert("inputs".to_string(), Value::Object(inputs));
    preset.insert("expressions".to_string(), Value::Object(expressions));
    preset.insert("tweens".to_string(), state.tweens.clone());
    preset.insert("midi".to_string(), state.midi.clone());

    Value::Object(preset)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ————————————————— isSceneDocument / isScenesPlaylist —————————————————

    #[test]
    fn scene_document_detects_scene_shape() {
        // Ported from tests/unit/preset-serializer.test.js isSceneDocument.
        assert!(is_scene_document(&json!({
            "id": "scene-1",
            "name": "A Scene",
            "groups": [],
            "modules": {},
        })));
    }

    #[test]
    fn scene_document_rejects_plain_preset() {
        assert!(!is_scene_document(&json!({
            "groups": [],
            "modules": {},
            "inputs": {},
            "expressions": {},
            "tweens": [],
            "midi": {},
        })));
    }

    #[test]
    fn scene_document_rejects_empty_and_partial_values() {
        assert!(!is_scene_document(&Value::Null));
        assert!(!is_scene_document(&json!({})));
        assert!(!is_scene_document(
            &json!({"id": "scene-1", "name": "A Scene"})
        ));
    }

    #[test]
    fn scene_document_rejects_non_string_id_and_empty_id() {
        assert!(!is_scene_document(&json!({
            "id": 42,
            "name": "A Scene",
            "groups": [],
        })));
        assert!(!is_scene_document(&json!({
            "id": "",
            "name": "A Scene",
            "groups": [],
        })));
        assert!(!is_scene_document(&json!({
            "id": "scene-1",
            "groups": [],
        })));
        assert!(!is_scene_document(&json!("string")));
    }

    #[test]
    fn scene_document_accepts_legacy_layers_array() {
        assert!(is_scene_document(&json!({
            "id": "scene-legacy",
            "name": "Legacy Scene",
            "layers": [],
        })));
    }

    #[test]
    fn scenes_playlist_detects_playlist_shape() {
        // Ported from tests/unit/preset-serializer.test.js isScenesPlaylist.
        assert!(is_scenes_playlist(&json!({
            "type": SCENES_PLAYLIST_TYPE,
            "version": 1,
            "scenes": [],
        })));
        assert_eq!(SCENES_PLAYLIST_TYPE, "modv-scenes-playlist");
    }

    #[test]
    fn scenes_playlist_rejects_scene_documents() {
        assert!(!is_scenes_playlist(&json!({
            "id": "scene-1",
            "name": "A Scene",
            "groups": [],
        })));
    }

    #[test]
    fn scenes_playlist_rejects_malformed_playlists() {
        assert!(!is_scenes_playlist(&Value::Null));
        assert!(!is_scenes_playlist(&json!({})));
        assert!(!is_scenes_playlist(&json!({
            "type": SCENES_PLAYLIST_TYPE,
            "scenes": "nope",
        })));
        // Type token alone is not a playlist — the type-only check still passes.
        assert!(is_scenes_playlist_type(
            &json!({"type": SCENES_PLAYLIST_TYPE})
        ));
        assert!(!is_scenes_playlist_type(&json!({"type": "other"})));
        assert!(!is_scenes_playlist_type(&Value::Null));
    }

    // ————————————————— fixture + round-trip —————————————————

    /// A realistic Scene document: Gallery + transitionGhost groups, a gallery
    /// module, `$status`, unknown keys, scene metadata.
    fn fixture_preset() -> Value {
        json!({
            "groups": [
                {
                    "id": "gallery-1",
                    "name": GALLERY_GROUP_NAME,
                    "hidden": false,
                    "modules": ["mgallery"],
                    "enabled": 1,
                    "alpha": 1,
                    "inherit": true,
                    "inheritFrom": -1,
                    "pipeline": false,
                    "clearing": false,
                    "compositeOperation": "normal",
                    "context": {"id": "aux-gallery", "canvas": {"width": 640, "height": 480}},
                    "writeToSwap": false
                },
                {
                    "id": "group-a",
                    "name": "Effects",
                    "hidden": false,
                    "modules": ["m1"],
                    "enabled": 1,
                    "alpha": 0.75,
                    "inherit": true,
                    "inheritFrom": -1,
                    "pipeline": false,
                    "clearing": true,
                    "compositeOperation": "multiply",
                    "sceneId": "scene-1",
                    "sceneRow": 0,
                    "alphaInputId": "in-a",
                    "enabledInputId": "in-b",
                    "context": {"id": "aux-a", "canvas": {"width": 1920, "height": 1080}},
                    "futureProp": {"nested": [1, 2, 3]}
                },
                {
                    "id": "group-ghost",
                    "name": "Effects (ghost)",
                    "hidden": false,
                    "modules": ["m1"],
                    "enabled": 1,
                    "alpha": 0.5,
                    "inherit": true,
                    "inheritFrom": -1,
                    "pipeline": false,
                    "clearing": false,
                    "compositeOperation": "normal",
                    "transitionGhost": true
                },
                {
                    "id": "group-b",
                    "name": "Foreground",
                    "hidden": false,
                    "modules": ["m1"],
                    "enabled": 2,
                    "alpha": 1,
                    "inherit": false,
                    "inheritFrom": "group-a",
                    "pipeline": true,
                    "clearing": false,
                    "compositeOperation": "source-over",
                    "drawToCanvasId": "aux-b",
                    "sceneRow": 1
                }
            ],
            "modules": {
                "m1": {
                    "$id": "m1",
                    "$moduleName": "Bands",
                    "meta": {
                        "name": "Bands",
                        "author": "modV",
                        "version": "1.0.0",
                        "type": "shader",
                        "alpha": 1,
                        "enabled": true,
                        "compositeOperation": "normal"
                    },
                    "props": {"bandAmount": 3},
                    "data": {"seed": 42},
                    "$props": {"bandAmount": {"id": "in-1", "type": "float"}},
                    "$status": [{"type": "warn", "message": "heads up"}],
                    "customField": "keep-me"
                },
                "mgallery": {
                    "$id": "mgallery",
                    "$moduleName": "GallerySphere",
                    "meta": {"name": "GallerySphere", "type": "three", "isGallery": true},
                    "props": {},
                    "data": {},
                    "$props": {}
                }
            },
            "inputs": {
                "focusedInput": {"id": "in-1", "title": "Bands.bandAmount"},
                "inputs": {
                    "in-1": {
                        "id": "in-1",
                        "type": "action",
                        "getLocation": "modules.active[\"m1\"].props[\"bandAmount\"]",
                        "location": "modules/updateProp",
                        "data": {"moduleId": "m1", "prop": "bandAmount"}
                    },
                    "in-a": {
                        "id": "in-a",
                        "type": "commit",
                        "location": "groups/UPDATE_GROUP_BY_KEY",
                        "data": {"groupId": "group-a", "key": "alpha"}
                    }
                },
                "inputLinks": {
                    "link-1": {
                        "id": "link-1",
                        "type": "getter",
                        "location": "meyda/getFeature",
                        "args": ["energy"],
                        "min": 0,
                        "max": 1,
                        "source": "meyda"
                    }
                }
            },
            "expressions": {
                "assignments": {
                    "ex-1": {
                        "id": "ex-1",
                        "inputId": "in-1",
                        "expression": "value * 2",
                        "evaluateEveryFrame": false,
                        "futureAssignment": [1, {"a": "b"}]
                    }
                }
            },
            "tweens": [
                {"id": "tw-1", "inputId": "in-1", "frames": [0, 0.5, 1], "loop": true}
            ],
            "midi": {"devices": {"dev-1": {"name": "Launchpad Mini"}}, "learning": false},
            "id": "scene-1",
            "name": "Scene 1",
            "enabled": true,
            "minSeconds": 8,
            "maxSeconds": 24,
            "timingOverride": false
        })
    }

    /// The expected preset document after a load → save round trip: the
    /// fixture above minus the dropped groups/module/fields and the scene
    /// metadata (which is not part of the preset format).
    fn expected_round_tripped() -> Value {
        json!({
            "groups": [
                {
                    "id": "group-a",
                    "name": "Effects",
                    "hidden": false,
                    "modules": ["m1"],
                    "enabled": 1,
                    "alpha": 0.75,
                    "inherit": true,
                    "inheritFrom": -1,
                    "pipeline": false,
                    "clearing": true,
                    "compositeOperation": "multiply",
                    "sceneId": "scene-1",
                    "sceneRow": 0,
                    "alphaInputId": "in-a",
                    "enabledInputId": "in-b",
                    "futureProp": {"nested": [1, 2, 3]}
                },
                {
                    "id": "group-b",
                    "name": "Foreground",
                    "hidden": false,
                    "modules": ["m1"],
                    "enabled": 2,
                    "alpha": 1,
                    "inherit": false,
                    "inheritFrom": "group-a",
                    "pipeline": true,
                    "clearing": false,
                    "compositeOperation": "source-over",
                    "sceneRow": 1
                }
            ],
            "modules": {
                "m1": {
                    "$id": "m1",
                    "$moduleName": "Bands",
                    "meta": {
                        "name": "Bands",
                        "author": "modV",
                        "version": "1.0.0",
                        "type": "shader",
                        "alpha": 1,
                        "enabled": true,
                        "compositeOperation": "normal"
                    },
                    "props": {"bandAmount": 3},
                    "data": {"seed": 42},
                    "$props": {"bandAmount": {"id": "in-1", "type": "float"}},
                    "customField": "keep-me"
                }
            },
            "inputs": {
                "focusedInput": {"id": "in-1", "title": "Bands.bandAmount"},
                "inputs": {
                    "in-1": {
                        "id": "in-1",
                        "type": "action",
                        "getLocation": "modules.active[\"m1\"].props[\"bandAmount\"]",
                        "location": "modules/updateProp",
                        "data": {"moduleId": "m1", "prop": "bandAmount"}
                    },
                    "in-a": {
                        "id": "in-a",
                        "type": "commit",
                        "location": "groups/UPDATE_GROUP_BY_KEY",
                        "data": {"groupId": "group-a", "key": "alpha"}
                    }
                },
                "inputLinks": {
                    "link-1": {
                        "id": "link-1",
                        "type": "getter",
                        "location": "meyda/getFeature",
                        "args": ["energy"],
                        "min": 0,
                        "max": 1,
                        "source": "meyda"
                    }
                }
            },
            "expressions": {
                "assignments": {
                    "ex-1": {
                        "id": "ex-1",
                        "inputId": "in-1",
                        "expression": "value * 2",
                        "evaluateEveryFrame": false,
                        "futureAssignment": [1, {"a": "b"}]
                    }
                }
            },
            "tweens": [
                {"id": "tw-1", "inputId": "in-1", "frames": [0, 0.5, 1], "loop": true}
            ],
            "midi": {"devices": {"dev-1": {"name": "Launchpad Mini"}}, "learning": false}
        })
    }

    #[test]
    fn presets_normalise_numeric_enabled_flags() {
        let preset = json!({
            "groups": [],
            "modules": {
                "a": {"$id": "a", "$moduleName": "Webcam", "meta": {"enabled": 1}},
                "b": {"$id": "b", "$moduleName": "Webcam", "meta": {"enabled": 0}},
                "c": {"$id": "c", "$moduleName": "Webcam", "meta": {"enabled": "yes"}},
                "d": {"$id": "d", "$moduleName": "Webcam", "meta": {"enabled": false}}
            },
            "inputs": {},
            "expressions": {},
            "tweens": [],
            "midi": {}
        });

        let document = parse_preset(&preset).expect("preset parses");
        assert_eq!(document.modules["a"].meta["enabled"], json!(true));
        assert_eq!(document.modules["b"].meta["enabled"], json!(false));
        // JS truthiness: a non-empty string (even "false") is truthy.
        assert_eq!(document.modules["c"].meta["enabled"], json!(true));
        assert_eq!(document.modules["d"].meta["enabled"], json!(false));
    }

    #[test]
    fn load_preset_splits_six_keys_and_keeps_metadata_aside() {
        let mut state = EngineState::new();
        let info = load_preset_into_state(&mut state, &fixture_preset()).unwrap();

        // Six keys land in state.
        assert_eq!(state.groups.len(), 4);
        assert_eq!(state.modules.len(), 2);
        assert_eq!(state.inputs.len(), 2);
        assert_eq!(state.input_links.len(), 1);
        assert_eq!(state.expressions.len(), 1);
        assert_eq!(state.tweens, fixture_preset()["tweens"]);
        assert_eq!(state.midi, fixture_preset()["midi"]);
        assert_eq!(
            state.focused_input,
            json!({"id": "in-1", "title": "Bands.bandAmount"})
        );

        // Scene metadata is kept aside, verbatim.
        assert_eq!(
            Value::Object(info.metadata.clone()),
            json!({
                "id": "scene-1",
                "name": "Scene 1",
                "enabled": true,
                "minSeconds": 8,
                "maxSeconds": 24,
                "timingOverride": false
            })
        );
        assert!(info.is_scene_document);
        assert!(!info.is_scenes_playlist);

        // Spot-check the typed pieces.
        assert_eq!(state.groups[0].name, GALLERY_GROUP_NAME);
        let ghost = state.groups.iter().find(|g| g.id == "group-ghost").unwrap();
        assert_eq!(ghost.transition_ghost, Some(true));
        assert_eq!(state.modules["m1"].module_name, "Bands");
        assert_eq!(state.modules["m1"].status.len(), 1);
        assert_eq!(
            state.input_links["link-1"].kind,
            crate::inputs::InputLinkKind::Getter
        );
        assert_eq!(state.expressions["ex-1"].input_id, "in-1");
    }

    #[test]
    fn round_trip_load_save_matches_expected_masked_fixture() {
        let mut state = EngineState::new();
        load_preset_into_state(&mut state, &fixture_preset()).unwrap();
        let saved = create_preset_from_state(&state);
        assert_eq!(saved, expected_round_tripped());
    }

    #[test]
    fn round_trip_drops_gallery_group_transition_ghost_and_gallery_module() {
        let mut state = EngineState::new();
        load_preset_into_state(&mut state, &fixture_preset()).unwrap();
        let saved = create_preset_from_state(&state);
        let groups = saved["groups"].as_array().unwrap();
        assert_eq!(groups.len(), 2);
        assert!(
            groups
                .iter()
                .all(|g| g["name"] != json!(GALLERY_GROUP_NAME))
        );
        assert!(groups.iter().all(|g| g.get("transitionGhost").is_none()));
        assert!(
            !saved["modules"]
                .as_object()
                .unwrap()
                .contains_key("mgallery")
        );
        assert!(
            !saved["modules"]["m1"]
                .as_object()
                .unwrap()
                .contains_key("$status")
        );
    }

    #[test]
    fn round_trip_drops_context_and_draw_to_canvas_id() {
        let mut state = EngineState::new();
        load_preset_into_state(&mut state, &fixture_preset()).unwrap();
        let saved = create_preset_from_state(&state);
        let groups = saved["groups"].as_array().unwrap();
        for group in groups {
            assert!(group.get("context").is_none());
            assert!(group.get("drawToCanvasId").is_none());
        }
    }

    #[test]
    fn round_trip_preserves_unknown_group_and_module_keys() {
        let mut state = EngineState::new();
        load_preset_into_state(&mut state, &fixture_preset()).unwrap();
        let saved = create_preset_from_state(&state);
        let groups = saved["groups"].as_array().unwrap();
        let group_a = groups.iter().find(|g| g["id"] == json!("group-a")).unwrap();
        assert_eq!(group_a["futureProp"], json!({"nested": [1, 2, 3]}));
        assert_eq!(saved["modules"]["m1"]["customField"], json!("keep-me"));
        // Expression assignment extras survive too.
        assert_eq!(
            saved["expressions"]["assignments"]["ex-1"]["futureAssignment"],
            json!([1, {"a": "b"}])
        );
    }

    #[test]
    fn round_trip_is_stable_across_load_save_cycles() {
        let mut state = EngineState::new();
        load_preset_into_state(&mut state, &fixture_preset()).unwrap();
        let first = create_preset_from_state(&state);

        let mut state2 = EngineState::new();
        load_preset_into_state(&mut state2, &first).unwrap();
        let second = create_preset_from_state(&state2);

        assert_eq!(first, second);
    }

    #[test]
    fn load_preset_ignores_unknown_top_level_keys() {
        let mut state = EngineState::new();
        let preset = json!({
            "groups": [],
            "beats": {"bpm": 120},
            "scenes": {"scenes": []},
            "customThing": 42,
        });
        let info = load_preset_into_state(&mut state, &preset).unwrap();
        assert_eq!(
            Value::Object(info.metadata.clone()),
            json!({"beats": {"bpm": 120}, "scenes": {"scenes": []}, "customThing": 42})
        );
        assert!(state.groups.is_empty());
    }

    #[test]
    fn load_preset_missing_keys_default_to_empty() {
        let mut state = EngineState::new();
        let info = load_preset_into_state(&mut state, &json!({})).unwrap();
        assert!(!info.is_scene_document);
        assert_eq!(state.groups.len(), 0);
        assert_eq!(state.modules.len(), 0);
        assert_eq!(state.tweens, json!([]));
        assert_eq!(state.midi, json!({}));
        assert!(info.metadata.is_empty());
    }

    #[test]
    fn load_preset_rejects_non_object_document() {
        let mut state = EngineState::new();
        assert_eq!(
            load_preset_into_state(&mut state, &json!([1, 2, 3])).unwrap_err(),
            PresetError::NotAnObject
        );
        assert_eq!(
            load_preset_into_state(&mut state, &Value::Null).unwrap_err(),
            PresetError::NotAnObject
        );
    }

    #[test]
    fn load_preset_rejects_wrong_shapes() {
        let mut state = EngineState::new();
        assert_eq!(
            load_preset_into_state(&mut state, &json!({"groups": {"not": "array"}})).unwrap_err(),
            PresetError::InvalidShape("groups")
        );
        assert_eq!(
            load_preset_into_state(&mut state, &json!({"modules": [1]})).unwrap_err(),
            PresetError::InvalidShape("modules")
        );
        assert_eq!(
            load_preset_into_state(&mut state, &json!({"tweens": "nope"})).unwrap_err(),
            PresetError::InvalidShape("tweens")
        );
        assert_eq!(
            load_preset_into_state(&mut state, &json!({"midi": []})).unwrap_err(),
            PresetError::InvalidShape("midi")
        );
        assert_eq!(
            load_preset_into_state(&mut state, &json!({"inputs": []})).unwrap_err(),
            PresetError::InvalidShape("inputs")
        );
        assert_eq!(
            load_preset_into_state(&mut state, &json!({"expressions": []})).unwrap_err(),
            PresetError::InvalidShape("expressions")
        );
    }

    #[test]
    fn load_preset_rejects_undecodable_member() {
        let mut state = EngineState::new();
        let err = load_preset_into_state(
            &mut state,
            &json!({"groups": [{"id": 42, "name": "missing modules/enabled"}]}),
        )
        .unwrap_err();
        assert!(matches!(err, PresetError::Decode { key: "groups", .. }));
    }

    #[test]
    fn load_partial_preset_keeps_empty_sections() {
        let mut state = EngineState::new();
        let info = load_preset_into_state(
            &mut state,
            &json!({
                "groups": [],
                "modules": {},
                "inputs": { "focusedInput": { "id": "keep", "title": "Focus" } },
            }),
        )
        .unwrap();
        assert_eq!(state.focused_input, json!({"id": "keep", "title": "Focus"}));
        assert!(state.inputs.is_empty());
        assert!(state.expressions.is_empty());
        assert!(!info.is_scene_document);
    }

    #[test]
    fn load_preset_rejects_scene_null_group_holes_loudly() {
        let mut state = EngineState::new();
        let err = load_preset_into_state(
            &mut state,
            &json!({
                "id": "scene-1",
                "name": "A Scene",
                "groups": [{"id": "g1", "name": "A", "hidden": false, "modules": [], "enabled": 1,
                            "alpha": 1, "inherit": true, "inheritFrom": -1,
                            "pipeline": false, "clearing": false,
                            "compositeOperation": "normal"}, null],
            }),
        )
        .unwrap_err();
        assert!(matches!(err, PresetError::Decode { key: "groups", .. }));
    }

    #[test]
    fn scene_document_fixture_is_detected() {
        let value = fixture_preset();
        assert!(is_scene_document(&value));
        assert!(!is_scenes_playlist(&value));
    }

    #[test]
    fn create_preset_from_empty_state() {
        let state = EngineState::new();
        assert_eq!(
            create_preset_from_state(&state),
            json!({
                "groups": [],
                "modules": {},
                "inputs": {
                    "focusedInput": Value::Null,
                    "inputs": {},
                    "inputLinks": {},
                },
                "expressions": {"assignments": {}},
                "tweens": [],
                "midi": {},
            })
        );
    }
}
