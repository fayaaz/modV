//! Performance-grid cell editing: clearing and merging a cell.
//!
//! Ported from `src/renderer/src/application/worker/scene-cell.js` (171
//! lines). Both operations return the Scene payload the caller writes back; the
//! store action wrapper (the Vuex commit) stays in the UI.
//!
//! - [`clear_scene_cell_modules`] removes every Module from the Group saved in
//!   a cell and prunes their snapshots, returning the removed ids so the caller
//!   can also clean the live Group.
//! - [`merge_scene_cell`] writes an edited live Group back into a cell, merging
//!   the cell's module snapshots, inputs, input links, expressions and tweens
//!   with the rest of the Scene. Named rows look their Group up by name, so an
//!   appended Group can live anywhere; unnamed rows are positional and pad
//!   missing rows with `null` holes.
//!
//! Line numbers refer to the tree at commit `8a512b6e` (branch `rust-hybrid`).

use serde_json::{Map, Value, json};

use crate::scene_rows::{get_row_group_index, get_row_name_index};

/// `getModuleIds` (`scene-cell.js:7-19`).
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

/// Result of [`clear_scene_cell_modules`].
#[derive(Clone, Debug, PartialEq)]
pub struct ClearedSceneCell {
    /// The Scene's `groups` array with the cell's `modules` emptied.
    pub groups: Vec<Value>,
    /// The Scene's `modules` map without the removed snapshots.
    pub modules: Map<String, Value>,
    /// The Module ids that were removed from the cell.
    pub module_ids: Vec<String>,
}

/// `clone` (`scene-cell.js:3-5`): a `JSON.parse(JSON.stringify(...))` copy.
fn clone_value(value: &Value) -> Value {
    value.clone()
}

/// `clearSceneCellModules` (`scene-cell.js:76-101`).
///
/// Returns `None` when the cell has no Group or the Group has no Modules, so
/// the caller can skip the update. The Scene document is not mutated.
pub fn clear_scene_cell_modules(
    scene: &Value,
    row_index: usize,
    row_names: &[String],
) -> Option<ClearedSceneCell> {
    let source_groups = scene
        .get("groups")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let index = get_row_group_index(scene, row_index, row_names)?;
    let group = source_groups.get(index)?;
    let module_ids = get_module_ids(group);

    if module_ids.is_empty() {
        return None;
    }

    let mut groups = source_groups
        .iter()
        .map(clone_value)
        .collect::<Vec<Value>>();
    groups[index]["modules"] = json!([]);

    let mut modules = scene
        .get("modules")
        .and_then(Value::as_object)
        .map(clone_value_map)
        .unwrap_or_default();

    for module_id in &module_ids {
        modules.remove(module_id);
    }

    Some(ClearedSceneCell {
        groups,
        modules,
        module_ids,
    })
}

/// Deep-copies a JSON object map.
fn clone_value_map(map: &Map<String, Value>) -> Map<String, Value> {
    map.iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

/// `pruneUnreferencedModules` (`scene-cell.js:21-35`).
fn prune_unreferenced_modules(groups: &[Value], modules: &mut Map<String, Value>) {
    let mut referenced: Vec<String> = Vec::new();

    for group in groups {
        for module_id in get_module_ids(group) {
            if !referenced.contains(&module_id) {
                referenced.push(module_id);
            }
        }
    }

    modules.retain(|module_id, _| referenced.contains(module_id));
}

/// `mergeTweens` (`scene-cell.js:37-69`): merge by tween id, keeping the
/// existing container shape (array vs object).
fn merge_tweens(existing: Option<&Value>, incoming: Option<&Value>) -> Value {
    let as_object = matches!(existing, Some(Value::Object(_)));
    let mut map: Map<String, Value> = Map::new();

    let mut add = |tweens: Option<&Value>| match tweens {
        Some(Value::Array(tweens)) => {
            for tween in tweens {
                if let Some(id) = tween.get("id").filter(|id| !id.is_null()) {
                    map.insert(id_key(id), tween.clone());
                }
            }
        }
        Some(Value::Object(tweens)) => {
            for (id, tween) in tweens {
                if tween.is_null() {
                    continue;
                }

                let key = tween
                    .get("id")
                    .filter(|id| !id.is_null())
                    .map(id_key)
                    .unwrap_or_else(|| id.clone());
                map.insert(key, tween.clone());
            }
        }
        _ => {}
    };

    add(existing);
    add(incoming);

    if as_object {
        Value::Object(map)
    } else {
        Value::Array(map.into_values().collect())
    }
}

/// A JS object key for a tween id: strings verbatim, anything else its JSON.
fn id_key(id: &Value) -> String {
    match id {
        Value::String(id) => id.clone(),
        other => other.to_string(),
    }
}

/// `mergeSceneCell` (`scene-cell.js:107-171`).
///
/// `cell` is the live snapshot: `{ group, modules, inputs, expressions, tweens }`.
/// Returns the merged Scene fields as an object, or `None` when the cell has no
/// Group.
pub fn merge_scene_cell(
    scene: &Value,
    row_index: usize,
    row_names: &[String],
    cell: &Value,
) -> Option<Value> {
    let group = cell.get("group").filter(|group| !group.is_null())?;

    let source_groups = scene
        .get("groups")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut groups: Vec<Value> = source_groups.iter().map(clone_value).collect();
    let group = clone_value(group);

    match get_row_group_index(scene, row_index, row_names) {
        Some(index) => groups[index] = group,
        None => {
            if get_row_name_index(row_names, row_index).is_empty() {
                // Unnamed rows are positional: keep the cell on its row by
                // padding any missing rows with holes, exactly like a
                // Ctrl+drag copy does.
                if groups.len() < row_index {
                    groups.resize(row_index, Value::Null);
                }

                if groups.len() == row_index {
                    groups.push(group);
                } else {
                    groups[row_index] = group;
                }
            } else {
                // Named rows look their Group up by name, so it can live
                // anywhere.
                groups.push(group);
            }
        }
    }

    let mut modules = scene
        .get("modules")
        .and_then(Value::as_object)
        .map(clone_value_map)
        .unwrap_or_default();

    if let Some(cell_modules) = cell.get("modules").and_then(Value::as_object) {
        for (module_id, module) in cell_modules {
            modules.insert(module_id.clone(), module.clone());
        }
    }

    prune_unreferenced_modules(&groups, &mut modules);

    let base_inputs = scene
        .get("inputs")
        .filter(|inputs| inputs.is_object())
        .cloned()
        .unwrap_or_else(|| json!({}));
    let cell_inputs = cell
        .get("inputs")
        .filter(|inputs| inputs.is_object())
        .cloned()
        .unwrap_or_else(|| json!({}));

    let mut inputs = base_inputs.as_object().cloned().unwrap_or_default();
    if !matches!(
        inputs.get("focusedInput"),
        Some(focused) if !focused.is_null()
    ) {
        inputs.insert(
            "focusedInput".to_string(),
            json!({"id": null, "title": null}),
        );
    }
    merge_object_key(&mut inputs, "inputs", &base_inputs, &cell_inputs);
    merge_object_key(&mut inputs, "inputLinks", &base_inputs, &cell_inputs);

    let base_expressions = scene
        .get("expressions")
        .filter(|expressions| expressions.is_object())
        .cloned()
        .unwrap_or_else(|| json!({}));
    let cell_expressions = cell
        .get("expressions")
        .filter(|expressions| expressions.is_object())
        .cloned()
        .unwrap_or_else(|| json!({}));

    let mut expressions = base_expressions.as_object().cloned().unwrap_or_default();
    let base_assignments = base_expressions
        .get("assignments")
        .and_then(Value::as_object);
    let cell_assignments = cell_expressions
        .get("assignments")
        .and_then(Value::as_object);
    let mut assignments = base_assignments.map(clone_value_map).unwrap_or_default();

    if let Some(cell_assignments) = cell_assignments {
        for (id, assignment) in cell_assignments {
            assignments.insert(id.clone(), assignment.clone());
        }
    }

    expressions.insert("assignments".to_string(), Value::Object(assignments));

    Some(json!({
        "groups": groups,
        "modules": Value::Object(modules),
        "inputs": Value::Object(inputs),
        "expressions": Value::Object(expressions),
        "tweens": merge_tweens(scene.get("tweens"), cell.get("tweens")),
    }))
}

/// Merge one nested object key (`inputs.inputs`, `inputs.inputLinks`) from the
/// Scene and the cell into `target`.
fn merge_object_key(target: &mut Map<String, Value>, key: &str, base: &Value, cell: &Value) {
    let mut merged = base
        .get(key)
        .and_then(Value::as_object)
        .map(clone_value_map)
        .unwrap_or_default();

    if let Some(cell_values) = cell.get(key).and_then(Value::as_object) {
        for (id, value) in cell_values {
            merged.insert(id.clone(), value.clone());
        }
    }

    target.insert(key.to_string(), Value::Object(merged));
}

#[cfg(test)]
mod tests {
    //! Port of `tests/unit/scene-cell.test.js` (Vitest).
    use super::*;

    fn group(id: &str, name: &str, modules: Value) -> Value {
        json!({"id": id, "name": name, "modules": modules})
    }

    fn scene(extra: Value) -> Value {
        let mut base = json!({
            "id": "scene-1",
            "name": "Scene 1",
            "groups": [
                group("g0", "Background", json!(["m0"])),
                group("g1", "Foreground", json!(["m1", "m2"])),
            ],
            "modules": {
                "m0": {"id": "m0"},
                "m1": {"id": "m1"},
                "m2": {"id": "m2"},
            },
            "inputs": {
                "focusedInput": {"id": null, "title": null},
                "inputs": {"i0": {"id": "i0"}},
                "inputLinks": {"i0": {"source": "bass"}},
            },
            "expressions": {"assignments": {"a0": {"inputId": "i0"}}},
            "tweens": [{"id": "t0"}],
        });

        if let (Some(target), Some(extra)) = (base.as_object_mut(), extra.as_object()) {
            for (key, value) in extra {
                target.insert(key.clone(), value.clone());
            }
        }

        base
    }

    fn cell() -> Value {
        json!({
            "group": group("live", "Foreground", json!(["m3"])),
            "modules": {"m3": {"id": "m3"}},
            "inputs": {
                "inputs": {"i3": {"id": "i3"}},
                "inputLinks": {"i3": {"source": "treble"}},
            },
            "expressions": {"assignments": {"a3": {"inputId": "i3"}}},
            "tweens": [{"id": "t3"}],
        })
    }

    // ————————————————— clearSceneCellModules —————————————————

    #[test]
    fn clear_removes_the_cells_modules_and_their_snapshots() {
        // scene-cell.test.js:36-43
        let result = clear_scene_cell_modules(&scene(json!({})), 1, &[]).unwrap();

        assert_eq!(result.module_ids, vec!["m1", "m2"]);
        assert_eq!(result.groups[1]["modules"], json!([]));
        assert_eq!(result.groups[0]["modules"], json!(["m0"]));
        assert_eq!(
            result.modules.keys().cloned().collect::<Vec<_>>(),
            vec!["m0"]
        );
    }

    #[test]
    fn clear_returns_none_for_an_empty_cell() {
        // scene-cell.test.js:45-54
        assert!(clear_scene_cell_modules(&scene(json!({})), 5, &[]).is_none());
        assert!(
            clear_scene_cell_modules(
                &scene(json!({"groups": [null, group("g1", "F", json!([]))]})),
                0,
                &[]
            )
            .is_none()
        );
    }

    #[test]
    fn clear_uses_named_rows_to_find_the_cells_group() {
        // scene-cell.test.js:56-70
        let value = scene(json!({
            "groups": [
                group("ga", "Effects", json!(["m0"])),
                group("gb", "Background", json!(["m1"])),
            ],
            "modules": {"m0": {"id": "m0"}, "m1": {"id": "m1"}},
        }));
        let row_names = vec!["".to_string(), "".to_string(), "Background".to_string()];

        let result = clear_scene_cell_modules(&value, 2, &row_names).unwrap();

        assert_eq!(result.groups[1]["modules"], json!([]));
        assert_eq!(result.groups[0]["modules"], json!(["m0"]));
        assert_eq!(Value::Object(result.modules), json!({"m0": {"id": "m0"}}));
    }

    #[test]
    fn clear_does_not_mutate_the_scene() {
        // scene-cell.test.js:72-78
        let value = scene(json!({}));
        let result = clear_scene_cell_modules(&value, 1, &[]).unwrap();

        assert_eq!(value["groups"][1]["modules"], json!(["m1", "m2"]));
        assert_ne!(Value::Array(result.groups), value["groups"]);
    }

    // ————————————————— mergeSceneCell —————————————————

    #[test]
    fn merge_replaces_the_group_at_the_cells_row() {
        // scene-cell.test.js:93-99
        let result = merge_scene_cell(&scene(json!({})), 1, &[], &cell()).unwrap();

        assert_eq!(result["groups"][1]["id"], json!("live"));
        assert_eq!(result["groups"][0]["id"], json!("g0"));
        assert_eq!(result["groups"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn merge_appends_a_group_when_the_row_has_no_snapshot() {
        // scene-cell.test.js:101-106
        let result = merge_scene_cell(&scene(json!({"groups": [null]})), 1, &[], &cell()).unwrap();

        assert!(result["groups"][0].is_null());
        assert_eq!(result["groups"][1]["id"], json!("live"));
    }

    #[test]
    fn merge_pads_positional_rows_below_the_saved_groups() {
        // scene-cell.test.js:108-116
        let result = merge_scene_cell(&scene(json!({})), 3, &[], &cell()).unwrap();

        assert_eq!(result["groups"].as_array().unwrap().len(), 4);
        assert_eq!(result["groups"][0]["id"], json!("g0"));
        assert_eq!(result["groups"][1]["id"], json!("g1"));
        assert!(result["groups"][2].is_null());
        assert_eq!(result["groups"][3]["id"], json!("live"));
    }

    #[test]
    fn merge_appends_a_named_row_cell_without_padding() {
        // scene-cell.test.js:118-126
        let mut named_cell = cell();
        named_cell["group"] = group("live", "Webcam", json!(["m3"]));
        let row_names = vec![
            "".to_string(),
            "".to_string(),
            "".to_string(),
            "".to_string(),
            "".to_string(),
            "Webcam".to_string(),
        ];

        let result = merge_scene_cell(&scene(json!({})), 5, &row_names, &named_cell).unwrap();

        assert_eq!(result["groups"][2]["id"], json!("live"));
        assert_eq!(result["groups"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn merge_merges_module_snapshots_and_prunes_unreferenced_ones() {
        // scene-cell.test.js:128-135
        let result = merge_scene_cell(&scene(json!({})), 1, &[], &cell()).unwrap();
        let modules = result["modules"].as_object().unwrap();

        assert!(modules.contains_key("m0"));
        assert!(modules.contains_key("m3"));
        assert!(!modules.contains_key("m1"));
        assert!(!modules.contains_key("m2"));
    }

    #[test]
    fn merge_merges_inputs_links_expressions_and_tweens() {
        // scene-cell.test.js:137-146
        let result = merge_scene_cell(&scene(json!({})), 1, &[], &cell()).unwrap();

        assert!(result["inputs"]["inputs"].get("i0").is_some());
        assert!(result["inputs"]["inputs"].get("i3").is_some());
        assert!(result["inputs"]["inputLinks"].get("i3").is_some());
        assert!(result["expressions"]["assignments"].get("a0").is_some());
        assert!(result["expressions"]["assignments"].get("a3").is_some());
        assert_eq!(result["tweens"], json!([{"id": "t0"}, {"id": "t3"}]));
    }

    #[test]
    fn merge_uses_named_rows_to_replace_the_right_group() {
        // scene-cell.test.js:148-161
        let value = scene(json!({
            "groups": [
                group("ga", "Effects", json!(["m0"])),
                group("gb", "Background", json!(["m1"])),
            ],
            "modules": {"m0": {"id": "m0"}, "m1": {"id": "m1"}},
        }));
        let row_names = vec!["".to_string(), "".to_string(), "Background".to_string()];

        let result = merge_scene_cell(&value, 2, &row_names, &cell()).unwrap();

        assert_eq!(result["groups"][1]["id"], json!("live"));
        assert_eq!(result["groups"][0]["id"], json!("ga"));
    }

    #[test]
    fn merge_does_not_mutate_the_scene() {
        // scene-cell.test.js:163-169
        let value = scene(json!({}));
        let result = merge_scene_cell(&value, 1, &[], &cell()).unwrap();

        assert_eq!(value["groups"][1]["id"], json!("g1"));
        assert_ne!(result["groups"], value["groups"]);
    }

    #[test]
    fn merge_returns_none_without_a_cell_group() {
        assert!(merge_scene_cell(&scene(json!({})), 0, &[], &json!({})).is_none());
        assert!(merge_scene_cell(&scene(json!({})), 0, &[], &json!({"group": null})).is_none());
    }

    #[test]
    fn merge_keeps_the_tween_container_shape() {
        // scene-cell.js:37-69
        let value = scene(json!({"tweens": {"t0": {"id": "t0"}}}));
        let result = merge_scene_cell(&value, 1, &[], &cell()).unwrap();

        assert_eq!(
            result["tweens"],
            json!({"t0": {"id": "t0"}, "t3": {"id": "t3"}})
        );
    }
}
