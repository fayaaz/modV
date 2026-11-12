//! Pure snapshot building for Scene hover previews.
//!
//! Ported from `src/renderer/src/application/worker/scene-preview-data.js` (93
//! lines). A hover preview renders from **saved** Group/Module snapshots, never
//! from the live Groups, so hovering a cell cannot disturb the running
//! composition (WS-8, "Scene preview").
//!
//! [`create_preview_module`] gives every preview Module a fresh id so it cannot
//! collide with a live Module's shader command, ISF renderer or Point10 scene
//! instance, and flags it `isGallery`/`scenePreview` so it renders through the
//! Gallery contexts instead of resizing the live composition's shared
//! ISF/shader canvases.
//!
//! Line numbers refer to the tree at commit `8a512b6e` (branch `rust-hybrid`).

use serde_json::{Map, Value, json};

use crate::scene_rows::{get_row_group, is_row_cell_empty};

/// The saved Group/module snapshots for one grid cell
/// (`scene-preview-data.js:25-41`).
#[derive(Clone, Debug, PartialEq)]
pub struct SceneRowPreview<'a> {
    /// The Group snapshot saved in the Scene document.
    pub group: &'a Value,
    /// The Module snapshots its `modules` list points at, in draw order.
    pub modules: Vec<&'a Value>,
}

/// `getModuleIds` (`scene-preview-data.js:6-18`): array-shaped lists as-is,
/// object-shaped maps by key, anything else empty.
fn get_module_ids(group: &Value) -> Vec<String> {
    match group.get("modules") {
        Some(Value::Array(modules)) => modules
            .iter()
            .filter_map(|module| module.as_str().map(str::to_string))
            .collect(),
        Some(Value::Object(modules)) => modules.keys().cloned().collect(),
        _ => Vec::new(),
    }
}

/// `getSceneRowPreview` (`scene-preview-data.js:25-41`).
///
/// Returns `None` for a missing/empty cell or a cell whose Module snapshots are
/// all missing. Module ids without a snapshot are skipped, not treated as an
/// error.
pub fn get_scene_row_preview<'a>(
    scene: &'a Value,
    row_index: usize,
    row_names: &[String],
) -> Option<SceneRowPreview<'a>> {
    if is_row_cell_empty(scene, row_index, row_names) {
        return None;
    }

    let group = get_row_group(scene, row_index, row_names)?;
    let module_snapshots = scene.get("modules").and_then(Value::as_object);

    let modules: Vec<&Value> = get_module_ids(group)
        .into_iter()
        .filter_map(|module_id| module_snapshots.and_then(|snapshots| snapshots.get(&module_id)))
        .collect();

    if modules.is_empty() {
        return None;
    }

    Some(SceneRowPreview { group, modules })
}

/// Merge object `source` into `target` the way a JS object spread does:
/// missing/non-object sources contribute nothing.
fn spread_object(target: &mut Map<String, Value>, source: Option<&Value>) {
    if let Some(Value::Object(source)) = source {
        for (key, value) in source {
            target.insert(key.clone(), value.clone());
        }
    }
}

/// `true` when a JSON value is falsy in JavaScript terms (the `!value` test
/// `createPreviewModule` uses for `compositeOperation`).
fn is_falsy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => true,
        Some(Value::Bool(value)) => !*value,
        Some(Value::Number(number)) => number.as_f64() == Some(0.0),
        Some(Value::String(text)) => text.is_empty(),
        _ => false,
    }
}

/// `createPreviewModule` (`scene-preview-data.js:49-93`).
///
/// Builds a standalone Module for the preview renderer from a saved snapshot
/// plus the registered definition:
///
/// - `$id` is the fresh preview id, `$moduleName` falls back to the
///   definition's `meta.name`.
/// - `meta` is the definition meta overridden by the snapshot meta, then
///   forced to `isGallery: true` / `scenePreview: true`; missing `alpha`,
///   `enabled` and `compositeOperation` get `1`, `true` and `"source-over"`.
/// - `props` is the definition props overridden by the snapshot props;
///   `$props` prefers the snapshot's copy and falls back to the definition
///   props.
/// - `data` prefers the snapshot's data (including an explicit `null`) and
///   falls back to the definition data.
/// - `$status` starts empty.
///
/// Nested objects in the result are copies, never shared with the snapshot.
pub fn create_preview_module(snapshot: &Value, definition: &Value, id: &str) -> Option<Value> {
    if !snapshot.is_object() || !definition.is_object() || id.is_empty() {
        return None;
    }

    let mut module = snapshot.as_object()?.clone();

    module.insert("$id".to_string(), json!(id));

    let module_name = snapshot
        .get("$moduleName")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
        .or_else(|| {
            definition
                .get("meta")
                .and_then(|meta| meta.get("name"))
                .and_then(Value::as_str)
        });
    module.insert(
        "$moduleName".to_string(),
        match module_name {
            Some(name) => json!(name),
            // The JS leaves `undefined` here, which a JSON clone drops.
            None => Value::Null,
        },
    );

    let mut meta = Map::new();
    spread_object(&mut meta, definition.get("meta"));
    spread_object(&mut meta, snapshot.get("meta"));
    // Previews render through the gallery GL contexts (like Gallery item
    // previews) so they never resize or otherwise touch the live
    // composition's shared ISF/shader canvases.
    meta.insert("isGallery".to_string(), json!(true));
    meta.insert("scenePreview".to_string(), json!(true));

    if !meta.contains_key("alpha") {
        meta.insert("alpha".to_string(), json!(1));
    }

    if !meta.contains_key("enabled") {
        meta.insert("enabled".to_string(), json!(true));
    }

    if is_falsy(meta.get("compositeOperation")) {
        meta.insert("compositeOperation".to_string(), json!("source-over"));
    }

    module.insert("meta".to_string(), Value::Object(meta));

    let mut props = Map::new();
    spread_object(&mut props, definition.get("props"));
    spread_object(&mut props, snapshot.get("props"));
    module.insert("props".to_string(), Value::Object(props));

    let dollar_props = match snapshot.get("$props") {
        Some(value) if !value.is_null() => value.clone(),
        _ => definition
            .get("props")
            .cloned()
            .unwrap_or_else(|| json!({})),
    };
    module.insert("$props".to_string(), dollar_props);

    let data = match snapshot.get("data") {
        Some(value) => value.clone(),
        None => definition.get("data").cloned().unwrap_or_else(|| json!({})),
    };
    module.insert("data".to_string(), data);

    module.insert("$status".to_string(), json!([]));

    Some(Value::Object(module))
}

#[cfg(test)]
mod tests {
    //! Port of `tests/unit/scene-preview-data.test.js` (Vitest).
    use super::*;

    fn definition() -> Value {
        json!({
            "meta": {
                "name": "Ball",
                "type": "2d",
                "enabledInputId": "registered-enabled"
            },
            "props": {
                "size": {"type": "int", "default": 2},
                "color": {"type": "tween", "default": [0, 0, 0]}
            },
            "data": {"frames": 0}
        })
    }

    fn snapshot() -> Value {
        json!({
            "$id": "scene-module",
            "$moduleName": "Ball",
            "meta": {
                "name": "Ball",
                "type": "2d",
                "enabled": true,
                "alpha": 0.75,
                "compositeOperation": "overlay",
                "enabledInputId": "scene-enabled"
            },
            "props": {
                "size": 8,
                "color": [255, 0, 0]
            },
            "$props": {
                "size": {"type": "int", "default": 2, "id": "scene-size"},
                "color": {"type": "tween", "default": [0, 0, 0], "id": "scene-color"}
            },
            "data": {"frames": 12},
            "$status": [{"type": "error", "message": "old status"}]
        })
    }

    // ————————————————— getSceneRowPreview —————————————————

    #[test]
    fn preview_returns_the_group_and_its_module_snapshots() {
        // scene-preview-data.test.js:47-68
        let module_snapshot = snapshot();
        let scene = json!({
            "id": "scene",
            "groups": [{"name": "Row", "modules": ["scene-module"]}],
            "modules": {"scene-module": module_snapshot.clone()},
        });

        let preview = get_scene_row_preview(&scene, 0, &[]).unwrap();

        assert_eq!(preview.group, &scene["groups"][0]);
        assert_eq!(preview.modules, vec![&module_snapshot]);
    }

    #[test]
    fn preview_skips_module_ids_without_snapshots() {
        // scene-preview-data.test.js:63-68
        let module_snapshot = snapshot();
        let scene = json!({
            "groups": [{"name": "Row", "modules": ["missing", "scene-module"]}],
            "modules": {"scene-module": module_snapshot.clone()},
        });

        let preview = get_scene_row_preview(&scene, 0, &[]).unwrap();
        assert_eq!(preview.modules, vec![&module_snapshot]);
    }

    #[test]
    fn preview_returns_none_for_missing_or_empty_cells() {
        // scene-preview-data.test.js:71-79
        assert!(get_scene_row_preview(&json!({"groups": []}), 0, &[]).is_none());
        assert!(
            get_scene_row_preview(
                &json!({"groups": [{"name": "Empty", "modules": []}], "modules": {}}),
                0,
                &[]
            )
            .is_none()
        );
    }

    #[test]
    fn preview_supports_object_shaped_module_maps() {
        // scene-preview-data.test.js:81-88
        let scene = json!({
            "groups": [{"name": "Row", "modules": {"scene-module": true}}],
            "modules": {"scene-module": snapshot()},
        });

        assert_eq!(
            get_scene_row_preview(&scene, 0, &[]).unwrap().modules.len(),
            1
        );
    }

    #[test]
    fn preview_honours_named_rows() {
        let scene = json!({
            "groups": [
                {"name": "Effects", "modules": ["m-effects"]},
                {"name": "Background", "modules": ["m-background"]},
            ],
            "modules": {
                "m-effects": snapshot(),
                "m-background": snapshot(),
            },
        });
        let row_names = vec!["".to_string(), "".to_string(), "Background".to_string()];

        let preview = get_scene_row_preview(&scene, 2, &row_names).unwrap();
        assert_eq!(preview.group["name"], json!("Background"));
    }

    // ————————————————— createPreviewModule —————————————————

    #[test]
    fn creates_an_isolated_preview_module_with_a_fresh_id() {
        // scene-preview-data.test.js:92-110
        let source = snapshot();
        let module = create_preview_module(&source, &definition(), "preview-module").unwrap();

        assert_eq!(module["$id"], json!("preview-module"));
        assert_ne!(module["$id"], source["$id"]);
        assert_eq!(module["$moduleName"], json!("Ball"));
        assert_eq!(module["meta"]["scenePreview"], json!(true));
        // Previews render through the gallery GL contexts so the live
        // composition's shared ISF/shader canvases are never resized by a
        // hover.
        assert_eq!(module["meta"]["isGallery"], json!(true));
        assert_eq!(module["meta"]["enabled"], json!(true));
        assert_eq!(module["meta"]["alpha"], json!(0.75));
        assert_eq!(module["meta"]["compositeOperation"], json!("overlay"));
        assert_eq!(module["props"], json!({"size": 8, "color": [255, 0, 0]}));
        assert_eq!(module["data"], json!({"frames": 12}));
        assert_eq!(module["$status"], json!([]));
    }

    #[test]
    fn does_not_share_nested_objects_with_the_scene_snapshot() {
        // scene-preview-data.test.js:112-122
        let source = snapshot();
        let mut module = create_preview_module(&source, &definition(), "preview-module").unwrap();

        module["props"]["color"][0] = json!(1);
        module["data"]["frames"] = json!(99);

        assert_eq!(source["props"]["color"][0], json!(255));
        assert_eq!(source["data"]["frames"], json!(12));
    }

    #[test]
    fn falls_back_to_the_registered_definition_props_and_data() {
        // scene-preview-data.test.js:124-131
        let mut source = snapshot();
        source.as_object_mut().unwrap().remove("props");
        source.as_object_mut().unwrap().remove("data");

        let module = create_preview_module(&source, &definition(), "preview-module").unwrap();

        assert_eq!(module["props"], definition()["props"]);
        assert_eq!(module["data"], definition()["data"]);
    }

    #[test]
    fn returns_none_when_the_module_is_unavailable() {
        // scene-preview-data.test.js:133-136
        assert!(create_preview_module(&snapshot(), &Value::Null, "preview").is_none());
        assert!(create_preview_module(&Value::Null, &definition(), "preview").is_none());
        assert!(create_preview_module(&snapshot(), &definition(), "").is_none());
    }

    #[test]
    fn fills_missing_meta_defaults() {
        let source = json!({
            "$id": "m1",
            "$moduleName": "Bloom",
            "meta": {"name": "Bloom", "compositeOperation": ""},
            "props": {},
        });
        let module = create_preview_module(&source, &json!({}), "preview").unwrap();

        assert_eq!(module["meta"]["alpha"], json!(1));
        assert_eq!(module["meta"]["enabled"], json!(true));
        assert_eq!(module["meta"]["compositeOperation"], json!("source-over"));
        assert_eq!(module["data"], json!({}));
        assert_eq!(module["$props"], json!({}));
    }

    #[test]
    fn module_name_falls_back_to_the_definition_meta() {
        let source = json!({"$id": "m1", "meta": {}, "props": {}});
        let module =
            create_preview_module(&source, &json!({"meta": {"name": "Ball"}}), "preview").unwrap();

        assert_eq!(module["$moduleName"], json!("Ball"));
    }

    #[test]
    fn definition_props_merge_under_snapshot_props() {
        let source = json!({"$id": "m1", "props": {"size": 3}});
        let module = create_preview_module(
            &source,
            &json!({"props": {"size": 1, "speed": 2}}),
            "preview",
        )
        .unwrap();

        assert_eq!(module["props"], json!({"size": 3, "speed": 2}));
    }
}
