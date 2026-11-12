//! Scenes playlist import / export and the performance config it carries.
//!
//! Ported from the playlist actions and helpers in
//! `src/renderer/src/application/worker/store/modules/scenes.js`:
//!
//! - `getPlaylistPerformance` (`scenes.js:266-288`)
//! - `getImportedPerformance` (`scenes.js:290-306`)
//! - `normalizeTransitionList` (`scenes.js:1490-1514`)
//! - `normalizePerformanceData` (`scenes.js:1516-1543`)
//! - `normalizeScene` (`scenes.js:1603-1631`) — the legacy
//!   `group` + `layers` shape
//! - `exportPlaylist` (`scenes.js:2076-2088`)
//! - `importPlaylist` (`scenes.js:2090-2163`)
//!
//! A playlist is `{ type: "modv-scenes-playlist", version: 1, scenes: [...],
//! performance: {...} }`. Performance settings travel with the playlist;
//! runtime state (`enabled`, `currentSceneId`) does not.
//!
//! ## JS → Rust notes
//!
//! - The JS `ADD_SCENE` mutation replaces an existing Scene **in place** by id
//!   and appends otherwise; [`import_playlist_with`] keeps that ordering.
//! - The JS generates a fresh `uuidv4()` for a playlist Scene without an id.
//!   The Rust port keeps a process-unique [`generate_scene_id`] instead of
//!   pulling `uuid`'s `v4` feature (which would add `getrandom`); ids only
//!   need to be unique within the engine, and the UI may rewrite them on
//!   save.
//! - `JSON.parse(JSON.stringify(...))` clones become `Value::clone` (deep).
//!
//! Line numbers refer to the tree at commit `8a512b6e` (branch `rust-hybrid`).

use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Map, Value, json};

use crate::preset::SCENES_PLAYLIST_TYPE;
use crate::scene_rows::normalize_row_names;
use crate::scene_timing::{normalize_performance_timing, normalize_scene_timing};

/// The playlist document version (`scenes.js:264`).
pub const PLAYLIST_VERSION: i64 = 1;

/// Performance keys that travel in a playlist (`scenes.js:266-273`).
pub const PLAYLIST_PERFORMANCE_KEYS: &[&str] = &[
    "minSeconds",
    "maxSeconds",
    "transitionSeconds",
    "allowedTransitions",
    "order",
    "rowNames",
];

/// How [`import_playlist`] treats the existing Scene list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportMode {
    /// Merge by id: imported Scenes replace same-id Scenes in place and new
    /// Scenes are appended (`scenes.js:2140-2146`, the default).
    Merge,
    /// Replace the whole list (`REPLACE_SCENES`, `scenes.js:2140-2142`).
    Replace,
}

/// Why a playlist could not be imported.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PlaylistError {
    /// The document is not a JSON object.
    #[error("playlist document is not an object")]
    NotAnObject,
    /// The document declares a different `type` token.
    #[error("playlist has the wrong type `{found}`")]
    WrongType {
        /// The offending type token.
        found: String,
    },
    /// The `scenes` key is missing or not an array.
    #[error("playlist `scenes` is missing or not an array")]
    MissingScenes,
}

/// What an import changed (`scenes.js:2159-2162`).
#[derive(Clone, Debug, PartialEq)]
pub struct ImportReport {
    /// The Scene list after the import.
    pub scenes: Vec<Value>,
    /// The merged performance config, when the playlist carried one.
    pub performance: Option<Value>,
    /// How many Scenes were appended.
    pub added: usize,
    /// How many existing Scenes were replaced in place.
    pub updated: usize,
    /// How many playlist entries were unusable and skipped.
    pub skipped: usize,
    /// Whether the Scene list was replaced wholesale.
    pub replace: bool,
}

/// `serialize_minimal_f64` (`engine_state.rs`) for a plain JSON number: JS
/// writes an integral number without a fractional part.
fn json_number(value: f64) -> Value {
    const SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;
    if value.fract() == 0.0 && value.abs() <= SAFE_INTEGER {
        json!(value as i64)
    } else {
        json!(value)
    }
}

/// Deep-copies a JSON object map.
fn clone_map(map: &Map<String, Value>) -> Map<String, Value> {
    map.iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

/// A process-unique Scene id, replacing the JS `uuidv4()` call
/// (`scenes.js:2127`). Only uniqueness within this engine matters.
pub fn generate_scene_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or_default();
    let sequence = COUNTER.fetch_add(1, Ordering::Relaxed);

    format!("scene-{now:x}-{sequence:x}")
}

/// `normalizeTransitionList` (`scenes.js:1490-1514`): strings only, non-empty,
/// de-duplicated, order preserved.
pub fn normalize_transition_list(value: Option<&Value>) -> Vec<String> {
    let Some(Value::Array(names)) = value else {
        return Vec::new();
    };

    let mut result: Vec<String> = Vec::new();

    for name in names {
        let Some(name) = name.as_str() else {
            continue;
        };

        if !name.is_empty() && !result.iter().any(|existing| existing == name) {
            result.push(name.to_string());
        }
    }

    result
}

/// `normalizePerformanceData` (`scenes.js:1516-1543`).
///
/// Migrates a legacy single `transition` string to a one-item
/// `allowedTransitions` list, normalises `rowNames`, and keeps every other key
/// it received. `transition` itself is always dropped.
pub fn normalize_performance_data(data: &Value) -> Value {
    let source = data.as_object().cloned().unwrap_or_default();
    let mut normalized = clone_map(&source);

    let has_allowed_transitions = source.contains_key("allowedTransitions");
    let legacy_transition = source.get("transition").cloned();

    normalized.remove("transition");

    let allowed = if has_allowed_transitions {
        normalize_transition_list(source.get("allowedTransitions"))
    } else {
        Vec::new()
    };

    if !allowed.is_empty() {
        normalized.insert(
            "allowedTransitions".to_string(),
            Value::Array(allowed.into_iter().map(Value::String).collect()),
        );
    } else if let Some(Value::String(legacy)) = legacy_transition {
        if !legacy.is_empty() {
            normalized.insert("allowedTransitions".to_string(), json!([legacy]));
        } else if has_allowed_transitions {
            normalized.insert("allowedTransitions".to_string(), json!([]));
        }
    } else if has_allowed_transitions {
        normalized.insert("allowedTransitions".to_string(), json!([]));
    }

    if normalized.contains_key("rowNames") {
        let names = normalize_row_names(source.get("rowNames"));
        normalized.insert(
            "rowNames".to_string(),
            Value::Array(names.into_iter().map(Value::String).collect()),
        );
    }

    Value::Object(normalized)
}

/// The performance config a playlist carries (`getPlaylistPerformance`,
/// `scenes.js:275-288`).
///
/// The three timing values are normalised and always present; the remaining
/// keys are copied verbatim from the input when it declares them. Runtime keys
/// (`enabled`, `currentSceneId`) are never exported.
pub fn get_playlist_performance(performance: &Value) -> Value {
    let timing = normalize_performance_timing(Some(performance));

    let mut data = Map::new();
    data.insert("minSeconds".to_string(), json_number(timing.min_seconds));
    data.insert("maxSeconds".to_string(), json_number(timing.max_seconds));
    data.insert(
        "transitionSeconds".to_string(),
        json_number(timing.transition_seconds),
    );

    for key in ["allowedTransitions", "order", "rowNames"] {
        if let Some(value) = performance.get(key) {
            data.insert(key.to_string(), value.clone());
        }
    }

    Value::Object(data)
}

/// The performance patch an imported playlist carries
/// (`getImportedPerformance`, `scenes.js:290-306`): only the known keys, and
/// `None` when there are none.
///
/// The legacy single `transition` key is also accepted (and later removed by
/// [`normalize_performance_data`]) so a playlist saved before the
/// allowed-transitions list still migrates; the JS helper drops it here because
/// the stored-Scenes hydration path normally migrates first.
pub fn get_imported_performance(performance: Option<&Value>) -> Option<Value> {
    let performance = performance.filter(|performance| performance.is_object())?;
    let mut data = Map::new();

    for key in PLAYLIST_PERFORMANCE_KEYS {
        if let Some(value) = performance.get(*key) {
            data.insert((*key).to_string(), value.clone());
        }
    }

    if let Some(transition) = performance.get("transition") {
        data.insert("transition".to_string(), transition.clone());
    }

    if data.is_empty() {
        return None;
    }

    Some(Value::Object(data))
}

/// Merge an imported performance patch over the current config
/// (`scenes.js:2106-2113`): `normalizePerformanceTiming({ ...current,
/// ...normalizePerformanceData(imported) })`.
pub fn merge_performance(current: &Value, imported: &Value) -> Value {
    let mut merged = current.as_object().cloned().unwrap_or_default();

    if let Some(data) = normalize_performance_data(imported).as_object() {
        for (key, value) in data {
            merged.insert(key.clone(), value.clone());
        }
    }

    let timing = normalize_performance_timing(Some(&Value::Object(merged.clone())));
    merged.insert("minSeconds".to_string(), json_number(timing.min_seconds));
    merged.insert("maxSeconds".to_string(), json_number(timing.max_seconds));
    merged.insert(
        "transitionSeconds".to_string(),
        json_number(timing.transition_seconds),
    );

    Value::Object(merged)
}

/// `normalizeScene` (`scenes.js:1603-1631`).
///
/// A Scene with a `groups` array is already current and is returned as a copy.
/// The legacy shape (a single `group` plus a flat `layers` array of Modules) is
/// converted to one Group carrying every layer id. Anything else is `None`.
pub fn normalize_scene_document(scene: &Value) -> Option<Value> {
    if !scene.is_object() {
        return None;
    }

    let id = scene.get("id").filter(|id| is_truthy(id))?.clone();

    if scene.get("groups").is_some_and(Value::is_array) {
        return Some(scene.clone());
    }

    let layers = scene.get("layers").and_then(Value::as_array)?;

    let mut modules = Map::new();

    for layer in layers {
        if let Some(layer_id) = layer.get("$id").filter(|id| is_truthy(id)) {
            if let Some(layer_id) = layer_id.as_str() {
                modules.insert(layer_id.to_string(), layer.clone());
            }
        }
    }

    let mut group = scene
        .get("group")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    group.insert(
        "name".to_string(),
        scene.get("name").cloned().unwrap_or(Value::Null),
    );
    group.insert(
        "modules".to_string(),
        Value::Array(modules.keys().cloned().map(Value::String).collect()),
    );

    let mut normalized = Map::new();
    normalized.insert("id".to_string(), id);
    normalized.insert(
        "name".to_string(),
        scene.get("name").cloned().unwrap_or(Value::Null),
    );
    normalized.insert(
        "groups".to_string(),
        Value::Array(vec![Value::Object(group)]),
    );
    normalized.insert("modules".to_string(), Value::Object(modules));

    Some(Value::Object(normalized))
}

/// A JS `!!` test for the values a Scene id can take.
fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(number) => number.as_f64().is_some_and(|n| n != 0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(map) => !map.is_empty(),
    }
}

/// `exportPlaylist` (`scenes.js:2076-2088`).
///
/// Every Scene is cloned verbatim; performance settings travel under
/// [`get_playlist_performance`]. Runtime state is excluded.
pub fn export_playlist(scenes: &[Value], performance: &Value) -> Value {
    json!({
        "type": SCENES_PLAYLIST_TYPE,
        "version": PLAYLIST_VERSION,
        "scenes": scenes,
        "performance": get_playlist_performance(performance),
    })
}

/// Validate the playlist document and return its Scene array.
fn playlist_scenes(playlist: &Value) -> Result<&Vec<Value>, PlaylistError> {
    let Some(object) = playlist.as_object() else {
        return Err(PlaylistError::NotAnObject);
    };

    if let Some(Value::String(kind)) = object.get("type") {
        if kind != SCENES_PLAYLIST_TYPE {
            return Err(PlaylistError::WrongType {
                found: kind.clone(),
            });
        }
    }

    object
        .get("scenes")
        .and_then(Value::as_array)
        .ok_or(PlaylistError::MissingScenes)
}

/// `importPlaylist` (`scenes.js:2090-2163`) with the current performance config
/// supplied so Scene timing normalises against the merged config.
///
/// Scenes merge by id in [`ImportMode::Merge`] (imported wins, new Scenes are
/// appended) or replace the list in [`ImportMode::Replace`]. Invalid entries
/// are skipped and counted in [`ImportReport::skipped`].
pub fn import_playlist_with(
    current_scenes: &mut Vec<Value>,
    current_performance: &Value,
    playlist: &Value,
    mode: ImportMode,
) -> Result<ImportReport, PlaylistError> {
    let raw_scenes = playlist_scenes(playlist)?;

    let imported_performance = get_imported_performance(playlist.get("performance"));
    let performance = imported_performance
        .as_ref()
        .map(|imported| merge_performance(current_performance, imported));
    let performance_for_timing = performance
        .clone()
        .unwrap_or_else(|| current_performance.clone());

    let mut imported_scenes: Vec<Value> = Vec::new();
    let mut skipped = 0usize;

    for raw in raw_scenes {
        if !raw.is_object() {
            skipped += 1;
            continue;
        }

        let mut candidate = raw.clone();

        // `id: rawScene.id || uuidv4()`
        let needs_id = candidate.get("id").filter(|id| is_truthy(id)).is_none();

        if needs_id {
            candidate["id"] = json!(generate_scene_id());
        }

        let Some(mut normalized) = normalize_scene_document(&candidate) else {
            skipped += 1;
            continue;
        };

        let timing = normalize_scene_timing(Some(&normalized), Some(&performance_for_timing));
        let object = normalized
            .as_object_mut()
            .expect("normalised Scene is an object");
        object.insert("minSeconds".to_string(), json_number(timing.min_seconds));
        object.insert("maxSeconds".to_string(), json_number(timing.max_seconds));
        object.insert("timingOverride".to_string(), json!(timing.timing_override));

        imported_scenes.push(normalized);
    }

    let replace = mode == ImportMode::Replace;
    let mut added = 0usize;
    let mut updated = 0usize;

    if replace {
        current_scenes.clear();
        current_scenes.extend(imported_scenes);
        added = current_scenes.len();
    } else {
        for scene in imported_scenes {
            let scene_id = scene.get("id").and_then(Value::as_str).map(str::to_string);

            match scene_id.and_then(|id| {
                current_scenes
                    .iter()
                    .position(|existing| existing.get("id").and_then(Value::as_str) == Some(&id))
            }) {
                Some(index) => {
                    current_scenes[index] = scene;
                    updated += 1;
                }
                None => {
                    current_scenes.push(scene);
                    added += 1;
                }
            }
        }
    }

    Ok(ImportReport {
        scenes: current_scenes.clone(),
        performance,
        added,
        updated,
        skipped,
        replace,
    })
}

/// [`import_playlist_with`] without a current performance config (defaults
/// apply when normalising Scene timing), matching the deliverable signature.
pub fn import_playlist(
    current_scenes: &mut Vec<Value>,
    playlist: &Value,
    mode: ImportMode,
) -> Result<ImportReport, PlaylistError> {
    import_playlist_with(current_scenes, &Value::Object(Map::new()), playlist, mode)
}

#[cfg(test)]
mod tests {
    //! Playlist round-trip/merge tests mirroring the Vitest expectations for
    //! `exportPlaylist` / `importPlaylist` and the performance helpers.
    use super::*;
    use serde_json::json;

    fn scene(id: &str, name: &str) -> Value {
        json!({
            "id": id,
            "name": name,
            "enabled": true,
            "minSeconds": 10,
            "maxSeconds": 26,
            "timingOverride": true,
            "groups": [{"id": format!("{id}-g"), "name": "Background", "modules": ["m1"]}],
            "modules": {"m1": {"$id": "m1", "$moduleName": "Bloom"}},
            "inputs": {"focusedInput": {"id": null, "title": null}, "inputs": {}, "inputLinks": {}},
            "expressions": {"assignments": {}},
            "tweens": [],
            "midi": {},
        })
    }

    fn performance() -> Value {
        json!({
            "enabled": true,
            "currentSceneId": "a",
            "minSeconds": 8,
            "maxSeconds": 24,
            "transitionSeconds": 2,
            "allowedTransitions": ["Fade", "Three Wipe"],
            "order": "in-order",
            "rowNames": ["Background", "Effects"],
        })
    }

    // ————————————————— export —————————————————

    #[test]
    fn export_has_the_playlist_type_version_and_scenes() {
        let scenes = vec![scene("a", "Scene A")];
        let exported = export_playlist(&scenes, &performance());

        assert_eq!(exported["type"], json!(SCENES_PLAYLIST_TYPE));
        assert_eq!(exported["version"], json!(PLAYLIST_VERSION));
        assert_eq!(exported["scenes"], json!(scenes));
    }

    #[test]
    fn export_excludes_runtime_performance_state() {
        let exported = export_playlist(&[], &performance());
        let exported_performance = exported["performance"].as_object().unwrap();

        assert!(!exported_performance.contains_key("enabled"));
        assert!(!exported_performance.contains_key("currentSceneId"));
        assert_eq!(
            exported_performance.keys().cloned().collect::<Vec<_>>(),
            vec![
                "minSeconds",
                "maxSeconds",
                "transitionSeconds",
                "allowedTransitions",
                "order",
                "rowNames",
            ]
        );
    }

    #[test]
    fn export_normalizes_the_performance_timing() {
        let exported = export_playlist(
            &[],
            &json!({"minSeconds": 0.2, "maxSeconds": 3.14159, "transitionSeconds": 0.04}),
        );

        assert_eq!(exported["performance"]["minSeconds"], json!(1));
        assert_eq!(exported["performance"]["maxSeconds"], json!(3.1));
        assert_eq!(exported["performance"]["transitionSeconds"], json!(0.1));
    }

    #[test]
    fn export_only_copies_performance_keys_the_input_declares() {
        let exported = export_playlist(&[], &json!({"minSeconds": 8}));
        let exported_performance = exported["performance"].as_object().unwrap();

        assert_eq!(
            exported_performance.keys().cloned().collect::<Vec<_>>(),
            vec!["minSeconds", "maxSeconds", "transitionSeconds"]
        );
    }

    // ————————————————— performance data —————————————————

    #[test]
    fn normalize_transition_list_drops_non_strings_and_duplicates() {
        // scenes.js:1490-1514
        assert_eq!(
            normalize_transition_list(Some(&json!(["Fade", "Fade", "Three Wipe", "", 42, null]))),
            vec!["Fade".to_string(), "Three Wipe".to_string()]
        );
        assert!(normalize_transition_list(Some(&json!("Fade"))).is_empty());
        assert!(normalize_transition_list(None).is_empty());
    }

    #[test]
    fn legacy_transition_migrates_to_a_one_item_allowed_list() {
        // scenes.js:1522-1536
        let normalized = normalize_performance_data(&json!({
            "transition": "Three Wipe",
            "order": "random",
        }));

        assert_eq!(normalized["allowedTransitions"], json!(["Three Wipe"]));
        assert!(normalized.get("transition").is_none());
        assert_eq!(normalized["order"], json!("random"));
    }

    #[test]
    fn allowed_transitions_win_over_the_legacy_transition() {
        // scenes.js:1526-1536
        let normalized = normalize_performance_data(&json!({
            "allowedTransitions": ["None", "None", "Fade"],
            "transition": "Three Wipe",
        }));

        assert_eq!(normalized["allowedTransitions"], json!(["None", "Fade"]));
        assert!(normalized.get("transition").is_none());
    }

    #[test]
    fn an_empty_allowed_list_falls_back_to_the_legacy_transition() {
        // scenes.js:1526-1536: `hasAllowedTransitions` only clears the list
        // when there is no legacy string to fall back to.
        let normalized = normalize_performance_data(&json!({
            "allowedTransitions": [],
            "transition": "Three Wipe",
        }));

        assert_eq!(normalized["allowedTransitions"], json!(["Three Wipe"]));
    }

    #[test]
    fn normalize_performance_data_normalizes_row_names() {
        // scenes.js:1538-1540
        let normalized = normalize_performance_data(&json!({
            "rowNames": [" webcam ", null, 42],
        }));

        assert_eq!(normalized["rowNames"], json!(["webcam", "", ""]));
    }

    #[test]
    fn imported_performance_only_keeps_known_keys() {
        // scenes.js:290-306
        let imported = get_imported_performance(Some(&json!({
            "minSeconds": 4,
            "enabled": true,
            "currentSceneId": "a",
            "order": "random",
        })))
        .unwrap();

        assert_eq!(imported, json!({"minSeconds": 4, "order": "random"}));
        assert!(get_imported_performance(Some(&json!({"enabled": true}))).is_none());
        assert!(get_imported_performance(None).is_none());
        assert!(get_imported_performance(Some(&json!("nope"))).is_none());
    }

    #[test]
    fn merge_performance_overrides_only_the_imported_fields() {
        // scenes.js:2106-2113
        let current = json!({
            "enabled": true,
            "currentSceneId": "a",
            "minSeconds": 8,
            "maxSeconds": 24,
            "transitionSeconds": 2,
            "order": "in-order",
        });
        let imported =
            json!({"minSeconds": 4, "allowedTransitions": ["None"], "transition": "Fade"});
        let merged = merge_performance(&current, &imported);

        assert_eq!(merged["enabled"], json!(true));
        assert_eq!(merged["currentSceneId"], json!("a"));
        assert_eq!(merged["minSeconds"], json!(4));
        assert_eq!(merged["maxSeconds"], json!(24));
        assert_eq!(merged["allowedTransitions"], json!(["None"]));
        assert_eq!(merged["order"], json!("in-order"));
    }

    // ————————————————— normalizeScene —————————————————

    #[test]
    fn normalize_scene_document_keeps_a_current_document() {
        // scenes.js:1608-1610
        let value = scene("a", "Scene A");
        assert_eq!(normalize_scene_document(&value), Some(value));
    }

    #[test]
    fn normalize_scene_document_converts_the_legacy_layers_shape() {
        // scenes.js:1612-1631
        let legacy = json!({
            "id": "legacy",
            "name": "Legacy Scene",
            "group": {"alpha": 0.5, "compositeOperation": "screen"},
            "layers": [
                {"$id": "m1", "$moduleName": "Bloom"},
                {"$id": "m2", "$moduleName": "Twirl"},
                {"name": "ignored"},
            ],
        });

        let normalized = normalize_scene_document(&legacy).unwrap();
        assert_eq!(normalized["id"], json!("legacy"));
        assert_eq!(normalized["groups"][0]["name"], json!("Legacy Scene"));
        assert_eq!(normalized["groups"][0]["modules"], json!(["m1", "m2"]));
        assert_eq!(normalized["groups"][0]["alpha"], json!(0.5));
        assert!(normalized["modules"].get("m1").is_some());
        assert!(normalized["modules"].get("m2").is_some());
    }

    #[test]
    fn normalize_scene_document_rejects_unusable_documents() {
        // scenes.js:1604-1614
        assert!(normalize_scene_document(&json!({})).is_none());
        assert!(normalize_scene_document(&json!({"id": "", "groups": []})).is_none());
        assert!(normalize_scene_document(&json!({"id": "a"})).is_none());
        assert!(normalize_scene_document(&json!({"id": "a", "layers": "nope"})).is_none());
        assert!(normalize_scene_document(&Value::Null).is_none());
    }

    // ————————————————— import —————————————————

    #[test]
    fn import_merges_by_id_in_place_and_appends_new_scenes() {
        // scenes.js:2116-2146 (ADD_SCENE semantics)
        let mut current = vec![scene("a", "A"), scene("b", "B")];
        let playlist = json!({
            "type": SCENES_PLAYLIST_TYPE,
            "version": 1,
            "scenes": [scene("b", "B renamed"), scene("c", "C")],
        });

        let report = import_playlist(&mut current, &playlist, ImportMode::Merge).unwrap();

        assert_eq!(report.updated, 1);
        assert_eq!(report.added, 1);
        assert_eq!(report.skipped, 0);
        assert!(!report.replace);
        let ids: Vec<&str> = current
            .iter()
            .filter_map(|scene| scene["id"].as_str())
            .collect();
        assert_eq!(ids, vec!["a", "b", "c"]);
        assert_eq!(current[1]["name"], json!("B renamed"));
        assert_eq!(report.scenes, current);
    }

    #[test]
    fn import_replaces_the_whole_list() {
        // scenes.js:2140-2142 (REPLACE_SCENES)
        let mut current = vec![scene("a", "A"), scene("b", "B")];
        let playlist = json!({"scenes": [scene("c", "C")]});

        let report = import_playlist(&mut current, &playlist, ImportMode::Replace).unwrap();

        assert!(report.replace);
        assert_eq!(report.added, 1);
        assert_eq!(report.updated, 0);
        assert_eq!(current.len(), 1);
        assert_eq!(current[0]["id"], json!("c"));
    }

    #[test]
    fn import_migrates_a_legacy_performance_transition() {
        // scenes.js:2104-2113 with normalizePerformanceData
        let mut current = Vec::new();
        let playlist = json!({
            "scenes": [scene("a", "A")],
            "performance": {"transition": "Three Wipe", "order": "random"},
        });

        let report = import_playlist(&mut current, &playlist, ImportMode::Merge).unwrap();
        let performance = report.performance.unwrap();

        assert_eq!(performance["allowedTransitions"], json!(["Three Wipe"]));
        assert_eq!(performance["order"], json!("random"));
        assert!(performance.get("transition").is_none());
    }

    #[test]
    fn import_normalizes_scene_timing_against_the_performance() {
        // scenes.js:2134-2137 (normalizeSceneTiming)
        let mut current = Vec::new();
        let mut value = scene("a", "A");
        value.as_object_mut().unwrap().remove("minSeconds");
        value.as_object_mut().unwrap().remove("maxSeconds");
        value.as_object_mut().unwrap().remove("timingOverride");
        let playlist = json!({
            "scenes": [value],
            "performance": {"minSeconds": 5, "maxSeconds": 12},
        });

        import_playlist(&mut current, &playlist, ImportMode::Merge).unwrap();

        assert_eq!(current[0]["minSeconds"], json!(5));
        assert_eq!(current[0]["maxSeconds"], json!(12));
        assert_eq!(current[0]["timingOverride"], json!(false));
    }

    #[test]
    fn import_keeps_an_explicit_scene_timing_override() {
        // scenes.js:2134-2137
        let mut current = Vec::new();
        let playlist = json!({"scenes": [scene("a", "A")]});

        import_playlist(&mut current, &playlist, ImportMode::Merge).unwrap();

        assert_eq!(current[0]["minSeconds"], json!(10));
        assert_eq!(current[0]["maxSeconds"], json!(26));
        assert_eq!(current[0]["timingOverride"], json!(true));
    }

    #[test]
    fn import_generates_ids_for_scenes_without_one() {
        // scenes.js:2125-2128 (`id: rawScene.id || uuidv4()`)
        let mut current = Vec::new();
        let mut value = scene("a", "A");
        value.as_object_mut().unwrap().remove("id");
        let playlist = json!({"scenes": [value, scene("b", "B")]});

        import_playlist(&mut current, &playlist, ImportMode::Merge).unwrap();

        let ids: Vec<&str> = current
            .iter()
            .map(|scene| scene["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids.len(), 2);
        assert!(!ids[0].is_empty());
        assert_ne!(ids[0], ids[1]);
        assert_eq!(ids[1], "b");
    }

    #[test]
    fn import_skips_unusable_entries() {
        // scenes.js:2118-2132
        let mut current = Vec::new();
        let playlist =
            json!({"scenes": [42, "nope", {"name": "no id or groups"}, scene("a", "A")]});

        let report = import_playlist(&mut current, &playlist, ImportMode::Merge).unwrap();

        assert_eq!(report.skipped, 3);
        assert_eq!(report.added, 1);
        assert_eq!(current.len(), 1);
    }

    #[test]
    fn import_accepts_legacy_scene_documents() {
        // scenes.js:2125-2132 via normalizeScene
        let mut current = Vec::new();
        let playlist = json!({
            "scenes": [{
                "id": "legacy",
                "name": "Legacy",
                "layers": [{"$id": "m1", "$moduleName": "Bloom"}],
            }],
        });

        import_playlist(&mut current, &playlist, ImportMode::Merge).unwrap();

        assert_eq!(current[0]["groups"][0]["modules"], json!(["m1"]));
        assert!(current[0]["modules"].get("m1").is_some());
    }

    #[test]
    fn import_validates_the_document_shape() {
        let mut current = Vec::new();

        assert_eq!(
            import_playlist(&mut current, &json!([1, 2]), ImportMode::Merge).unwrap_err(),
            PlaylistError::NotAnObject
        );
        assert_eq!(
            import_playlist(&mut current, &Value::Null, ImportMode::Merge).unwrap_err(),
            PlaylistError::NotAnObject
        );
        assert_eq!(
            import_playlist(
                &mut current,
                &json!({"type": "something-else", "scenes": []}),
                ImportMode::Merge
            )
            .unwrap_err(),
            PlaylistError::WrongType {
                found: "something-else".to_string()
            }
        );
        assert_eq!(
            import_playlist(&mut current, &json!({"scenes": "nope"}), ImportMode::Merge)
                .unwrap_err(),
            PlaylistError::MissingScenes
        );
        assert_eq!(
            import_playlist(&mut current, &json!({}), ImportMode::Merge).unwrap_err(),
            PlaylistError::MissingScenes
        );
    }

    #[test]
    fn import_does_not_change_scenes_on_a_validation_error() {
        let mut current = vec![scene("a", "A")];
        let _ = import_playlist(&mut current, &json!({"scenes": 42}), ImportMode::Replace);
        assert_eq!(current.len(), 1);
        assert_eq!(current[0]["id"], json!("a"));
    }

    #[test]
    fn import_with_performance_merges_over_the_current_config() {
        // scenes.js:2106-2113
        let mut current = Vec::new();
        let current_performance = json!({
            "enabled": false,
            "currentSceneId": null,
            "minSeconds": 8,
            "maxSeconds": 24,
            "transitionSeconds": 2,
            "order": "in-order",
        });
        let playlist = json!({
            "scenes": [],
            "performance": {"transitionSeconds": 3, "allowedTransitions": ["Fade"]},
        });

        let report = import_playlist_with(
            &mut current,
            &current_performance,
            &playlist,
            ImportMode::Merge,
        )
        .unwrap();
        let performance = report.performance.unwrap();

        assert_eq!(performance["transitionSeconds"], json!(3));
        assert_eq!(performance["allowedTransitions"], json!(["Fade"]));
        assert_eq!(performance["order"], json!("in-order"));
    }

    // ————————————————— round trip —————————————————

    #[test]
    fn export_import_round_trip_preserves_the_playlist() {
        let scenes = vec![scene("a", "A"), scene("b", "B")];
        let exported = export_playlist(&scenes, &performance());

        let mut reimported = Vec::new();
        import_playlist(&mut reimported, &exported, ImportMode::Replace).unwrap();
        let reexported = export_playlist(&reimported, &exported["performance"]);

        assert_eq!(reexported["type"], exported["type"]);
        assert_eq!(reexported["version"], exported["version"]);
        assert_eq!(reexported["performance"], exported["performance"]);
        assert_eq!(
            reexported["scenes"].as_array().unwrap().len(),
            exported["scenes"].as_array().unwrap().len()
        );
        for (before, after) in exported["scenes"]
            .as_array()
            .unwrap()
            .iter()
            .zip(reexported["scenes"].as_array().unwrap())
        {
            assert_eq!(before["id"], after["id"]);
            assert_eq!(before["name"], after["name"]);
            assert_eq!(before["groups"], after["groups"]);
            assert_eq!(before["modules"], after["modules"]);
            assert_eq!(before["inputs"], after["inputs"]);
            assert_eq!(before["expressions"], after["expressions"]);
            assert_eq!(before["tweens"], after["tweens"]);
        }
    }

    #[test]
    fn export_import_is_idempotent_after_the_first_pass() {
        let scenes = vec![scene("a", "A")];
        let first_export = export_playlist(&scenes, &performance());

        let mut once = Vec::new();
        import_playlist(&mut once, &first_export, ImportMode::Replace).unwrap();
        let second_export = export_playlist(&once, &first_export["performance"]);

        let mut twice = Vec::new();
        import_playlist(&mut twice, &second_export, ImportMode::Replace).unwrap();
        let third_export = export_playlist(&twice, &second_export["performance"]);

        assert_eq!(second_export, third_export);
    }
}
