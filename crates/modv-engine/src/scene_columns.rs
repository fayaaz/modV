//! Scene column helpers: naming, reordering and empty Scene creation.
//!
//! Ported from `src/renderer/src/application/worker/scene-columns.js` (90
//! lines). These are the pure parts of the Scene grid's column actions; the
//! Vuex dispatch/persist wrapper stays in the UI.
//!
//! - [`next_scene_name`] returns the first unused `Scene N`, so adding a column
//!   never produces a duplicate name the rename action would then reject.
//! - [`move_scene_to_index`] reorders a column and returns `None` for a no-op
//!   so callers can skip persisting it (the JS returns the same array
//!   reference; Rust callers get `None` instead).
//! - [`create_empty_scene`] builds a Scene document with no Groups or Modules,
//!   ready to be filled by Ctrl+dragging cells from other Scenes.
//!
//! Line numbers refer to the tree at commit `8a512b6e` (branch `rust-hybrid`).

use serde_json::{Map, Value, json};

/// `SCENE_NAME_PREFIX` (`scene-columns.js:1`).
pub const SCENE_NAME_PREFIX: &str = "Scene";

/// `normalizeName` (`scene-columns.js:3-4`): only strings have a name.
fn normalize_name(name: Option<&Value>) -> String {
    name.and_then(Value::as_str)
        .map(|name| name.trim().to_lowercase())
        .unwrap_or_default()
}

/// `nextSceneName` (`scene-columns.js:10-30`).
///
/// Names are compared case-insensitively ignoring surrounding whitespace;
/// non-strings, blanks and duplicates are ignored.
pub fn next_scene_name(existing_names: &[Value]) -> String {
    let taken: Vec<String> = existing_names
        .iter()
        .map(|name| normalize_name(Some(name)))
        .filter(|name| !name.is_empty())
        .collect();

    let mut index = 1;

    loop {
        let name = format!("{SCENE_NAME_PREFIX} {index}");

        if !taken.contains(&name.to_lowercase()) {
            return name;
        }

        index += 1;
    }
}

/// `moveSceneToIndex` (`scene-columns.js:36-58`).
///
/// Returns the reordered list, or `None` when the move is a no-op (unknown
/// Scene, out-of-range or non-integer target, or same position) so the caller
/// skips persisting. The input list is never mutated.
pub fn move_scene_to_index(
    scenes: &[Value],
    scene_id: &str,
    to_index: Option<i64>,
) -> Option<Vec<Value>> {
    let Some(to_index) = to_index else {
        return None;
    };

    if to_index < 0 || to_index as usize >= scenes.len() {
        return None;
    }

    let from_index = scenes
        .iter()
        .position(|scene| scene.get("id").and_then(Value::as_str) == Some(scene_id))?;

    let to_index = to_index as usize;

    if to_index == from_index {
        return None;
    }

    let mut reordered = scenes.to_vec();
    let moved = reordered.remove(from_index);
    reordered.insert(to_index, moved);

    Some(reordered)
}

/// Options for [`create_empty_scene`] (`scene-columns.js:64-90`).
#[derive(Clone, Debug)]
pub struct EmptySceneOptions<'a> {
    /// The Scene id (the JS caller supplies a fresh uuid).
    pub id: &'a str,
    /// An explicit name; blank falls back to [`next_scene_name`].
    pub name: Option<&'a str>,
    /// Existing Scene names, used to generate a unique default name.
    pub existing_names: &'a [Value],
    /// Timing defaults (`{ minSeconds, maxSeconds }`).
    pub timing: Option<&'a Value>,
}

impl Default for EmptySceneOptions<'_> {
    fn default() -> Self {
        Self {
            id: "",
            name: None,
            existing_names: &[],
            timing: None,
        }
    }
}

/// `createEmptyScene` (`scene-columns.js:64-90`).
///
/// The document is a valid Scene: `id`, `name`, `enabled`, timing, and empty
/// `groups` / `modules` / `inputs` / `expressions` / `tweens` / `midi` slots.
/// `minSeconds`/`maxSeconds` are only written when the timing object carries
/// them, matching the JS `undefined` keys being dropped by the JSON clone.
pub fn create_empty_scene(options: &EmptySceneOptions<'_>) -> Value {
    let requested_name = options.name.map(str::trim).unwrap_or_default();
    let name = if requested_name.is_empty() {
        next_scene_name(options.existing_names)
    } else {
        requested_name.to_string()
    };

    let mut scene = Map::new();
    scene.insert("id".to_string(), json!(options.id));
    scene.insert("name".to_string(), json!(name));
    scene.insert("enabled".to_string(), json!(true));

    if let Some(timing) = options.timing.filter(|timing| timing.is_object()) {
        if let Some(min_seconds) = timing.get("minSeconds") {
            scene.insert("minSeconds".to_string(), min_seconds.clone());
        }
        if let Some(max_seconds) = timing.get("maxSeconds") {
            scene.insert("maxSeconds".to_string(), max_seconds.clone());
        }
    }

    scene.insert("timingOverride".to_string(), json!(false));
    scene.insert("groups".to_string(), json!([]));
    scene.insert("modules".to_string(), json!({}));
    scene.insert(
        "inputs".to_string(),
        json!({
            "focusedInput": {"id": null, "title": null},
            "inputs": {},
            "inputLinks": {},
        }),
    );
    scene.insert("expressions".to_string(), json!({"assignments": {}}));
    scene.insert("tweens".to_string(), json!([]));
    scene.insert("midi".to_string(), json!({}));

    Value::Object(scene)
}

#[cfg(test)]
mod tests {
    //! Port of `tests/unit/scene-columns.test.js` (Vitest).
    use super::*;
    use crate::preset::is_scene_document;

    // ————————————————— nextSceneName —————————————————

    #[test]
    fn next_scene_name_starts_at_scene_one() {
        // scene-columns.test.js:9-11
        assert_eq!(next_scene_name(&[]), "Scene 1");
    }

    #[test]
    fn next_scene_name_skips_names_already_in_use() {
        // scene-columns.test.js:13-17
        assert_eq!(
            next_scene_name(&[json!("Scene 1"), json!("Scene 2")]),
            "Scene 3"
        );
        assert_eq!(next_scene_name(&[json!("Scene 2")]), "Scene 1");
    }

    #[test]
    fn next_scene_name_matches_case_insensitively_and_ignores_whitespace() {
        // scene-columns.test.js:19-21
        assert_eq!(
            next_scene_name(&[json!("scene 1"), json!("  SCENE 2  ")]),
            "Scene 3"
        );
    }

    #[test]
    fn next_scene_name_ignores_blank_and_non_string_names() {
        // scene-columns.test.js:23-25
        assert_eq!(
            next_scene_name(&[Value::Null, Value::Null, json!(""), json!("   "), json!(42)]),
            "Scene 1"
        );
    }

    // ————————————————— createEmptyScene —————————————————

    #[test]
    fn create_empty_scene_generates_a_unique_name() {
        // scene-columns.test.js:31-49
        let scene = create_empty_scene(&EmptySceneOptions {
            id: "scene-id",
            name: None,
            existing_names: &[json!("Scene 1")],
            timing: Some(&json!({"minSeconds": 4, "maxSeconds": 9})),
        });

        assert_eq!(scene["id"], json!("scene-id"));
        assert_eq!(scene["name"], json!("Scene 2"));
        assert_eq!(scene["enabled"], json!(true));
        assert_eq!(scene["minSeconds"], json!(4));
        assert_eq!(scene["maxSeconds"], json!(9));
        assert_eq!(scene["timingOverride"], json!(false));
        assert_eq!(scene["groups"], json!([]));
        assert_eq!(scene["modules"], json!({}));
        assert!(is_scene_document(&scene));
    }

    #[test]
    fn create_empty_scene_uses_an_explicit_name() {
        // scene-columns.test.js:51-61
        let scene = create_empty_scene(&EmptySceneOptions {
            id: "scene-id",
            name: Some("  New Column  "),
            existing_names: &[json!("Scene 1")],
            timing: Some(&json!({"minSeconds": 4, "maxSeconds": 9})),
        });

        assert_eq!(scene["name"], json!("New Column"));
    }

    #[test]
    fn create_empty_scene_has_no_groups_or_modules_to_copy() {
        // scene-columns.test.js:63-75
        let scene = create_empty_scene(&EmptySceneOptions {
            id: "scene-id",
            timing: None,
            ..EmptySceneOptions::default()
        });

        assert_eq!(scene["groups"], json!([]));
        assert_eq!(scene["modules"], json!({}));
        assert_eq!(
            scene["inputs"],
            json!({
                "focusedInput": {"id": null, "title": null},
                "inputs": {},
                "inputLinks": {},
            })
        );
        assert_eq!(scene["expressions"], json!({"assignments": {}}));
        assert_eq!(scene["tweens"], json!([]));
        assert_eq!(scene["midi"], json!({}));
    }

    // ————————————————— moveSceneToIndex —————————————————

    fn scenes() -> Vec<Value> {
        vec![json!({"id": "a"}), json!({"id": "b"}), json!({"id": "c"})]
    }

    fn ids(scenes: &[Value]) -> Vec<&str> {
        scenes
            .iter()
            .filter_map(|scene| scene["id"].as_str())
            .collect()
    }

    #[test]
    fn moves_a_scene_later_in_the_list() {
        // scene-columns.test.js:81-87
        let moved = move_scene_to_index(&scenes(), "a", Some(2)).unwrap();
        assert_eq!(ids(&moved), vec!["b", "c", "a"]);
    }

    #[test]
    fn moves_a_scene_earlier_in_the_list() {
        // scene-columns.test.js:89-95
        let moved = move_scene_to_index(&scenes(), "c", Some(0)).unwrap();
        assert_eq!(ids(&moved), vec!["c", "a", "b"]);
    }

    #[test]
    fn moving_a_scene_does_not_mutate_the_input() {
        // scene-columns.test.js:97-103
        let original = scenes();
        let _ = move_scene_to_index(&original, "a", Some(2));
        assert_eq!(ids(&original), vec!["a", "b", "c"]);
    }

    #[test]
    fn move_returns_none_for_a_no_op() {
        // scene-columns.test.js:105-107 (JS returns the same list reference)
        assert!(move_scene_to_index(&scenes(), "b", Some(1)).is_none());
    }

    #[test]
    fn move_ignores_unknown_scenes_and_invalid_indexes() {
        // scene-columns.test.js:109-115
        assert!(move_scene_to_index(&scenes(), "missing", Some(1)).is_none());
        assert!(move_scene_to_index(&scenes(), "a", Some(-1)).is_none());
        assert!(move_scene_to_index(&scenes(), "a", Some(3)).is_none());
        assert!(move_scene_to_index(&scenes(), "a", None).is_none());
        assert!(move_scene_to_index(&[], "a", Some(0)).is_none());
    }
}
