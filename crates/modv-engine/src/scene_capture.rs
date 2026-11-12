//! Capture live Group state back into a Scene document.
//!
//! Ported from `src/renderer/src/application/worker/scene-capture.js` (141
//! lines) plus the refusal rule from `scenes.js` `captureLiveStateToScene`
//! (`scenes.js:1956-1990`). WS-8, "Capture live state to Scene":
//!
//! - Only the **applied** Scene's Groups are collected: non-ghost, non-Gallery
//!   and stamped with `sceneId` / `sceneRow`.
//! - The Scene's `groups` array is rebuilt from those Groups keyed by row,
//!   with `null` holes for rows the live stack no longer has. A hole keeps the
//!   remaining rows at their positions.
//! - Capture is refused while performance mode is running ([`CaptureError::PerformanceRunning`])
//!   because rows are changing under the capture.
//! - An empty Scene column instead captures the whole live user layer stack
//!   (`collectSceneGroups`'s `fromLive: true` case), so a newly added column
//!   can be filled from the current setup. A *populated* Scene that is not
//!   applied is never overwritten.
//!
//! ## JS → Rust notes
//!
//! - The JS works on live Group objects held by Vuex. The Rust engine owns
//!   typed [`Group`]s, so capture reads [`EngineState::groups`] and serialises
//!   each Group as a JSON Scene snapshot.
//! - `stripGroup` (`scene-capture.js:10-16`) removes `context`,
//!   `writeToSwap` and `readFromSwap`; the typed [`Group`] keeps unknown keys
//!   in `extra`, so they serialise and are then removed here.
//! - The JS `clone` is a `JSON.parse(JSON.stringify(...))` deep copy;
//!   `serde_json::to_value` produces the same deep copy.
//!
//! Line numbers refer to the tree at commit `8a512b6e` (branch `rust-hybrid`).

use std::collections::BTreeMap;

use serde_json::Value;

use crate::engine_state::{EngineState, GALLERY_GROUP_NAME, Group};

/// Upper bound on a captured row index.
///
/// Alias of [`crate::scene_rows::MAX_SCENE_ROWS`]: the performance grid and the
/// capture path share one bound, so a `sceneRow` the grid rejects is never
/// materialised by a capture either.
pub use crate::scene_rows::MAX_SCENE_ROWS as MAX_CAPTURED_ROWS;

/// A Scene's `groups` array: a snapshot per row, `None` for a row hole.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SceneGroups {
    /// One entry per grid row; `None` is the JS `null` hole.
    pub groups: Vec<Option<Value>>,
}

/// Result of [`collect_scene_groups`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CollectedSceneGroups {
    /// The snapshot to save.
    pub groups: Vec<Option<Value>>,
    /// `true` when the snapshot came from the live layer stack rather than the
    /// applied Scene's stamped Groups (`scene-capture.js:113-137`).
    pub from_live: bool,
}

/// Why a Scene cannot be captured.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CaptureError {
    /// Performance mode is running (`scenes.js:1963-1966`), so rows are in
    /// flux and a capture would mix two Scenes.
    #[error("cannot capture a Scene while performance mode is running")]
    PerformanceRunning,
    /// None of the live Groups are stamped with this Scene id — the Scene is
    /// not applied, so its saved snapshot must not be overwritten
    /// (`scene-capture.js:35-51`).
    #[error("cannot capture a Scene that is not applied")]
    SceneNotApplied,
    /// The live stack has no user Groups to capture
    /// (`scene-capture.js:68-93`).
    #[error("cannot capture a Scene with no live Groups")]
    NoLiveGroups,
}

/// `isGalleryGroup` (`scene-capture.js:3-4`).
fn is_gallery_group(group: &Group) -> bool {
    group.name == GALLERY_GROUP_NAME
}

/// `stripGroup` + `clone` (`scene-capture.js:6-16`): a deep JSON copy without
/// the render-only keys.
fn group_snapshot(group: &Group) -> Value {
    match serde_json::to_value(group) {
        Ok(Value::Object(mut map)) => {
            map.remove("context");
            map.remove("writeToSwap");
            map.remove("readFromSwap");
            Value::Object(map)
        }
        Ok(other) => other,
        Err(_) => Value::Object(serde_json::Map::new()),
    }
}

/// `fillRowHoles` (`scene-capture.js:18-26`): every unset row becomes `None`,
/// materialised up to the last row that was actually captured.
///
/// The JS kept a sparse array; the engine builds the rows sparsely too (a
/// `BTreeMap`) so a corrupt `sceneRow` cannot size an allocation, then
/// materialises only `0..=last_row`.
fn rows_from_sparse(rows: BTreeMap<usize, Value>) -> SceneGroups {
    let Some(last_row) = rows.keys().next_back().copied() else {
        return SceneGroups::default();
    };

    let mut groups = vec![None; last_row + 1];

    for (row, group) in rows {
        if let Some(slot) = groups.get_mut(row) {
            *slot = Some(group);
        }
    }

    SceneGroups { groups }
}

/// Record a snapshot at `row`, ignoring an out-of-range index.
fn insert_row(rows: &mut BTreeMap<usize, Value>, row: usize, group: Value) {
    if row < MAX_CAPTURED_ROWS {
        rows.insert(row, group);
    }
}

/// `buildCapturedGroups` (`scene-capture.js:35-61`).
///
/// Collects the live Groups stamped with `scene_id`, sorted by `sceneRow`, and
/// rebuilds the row-indexed array. Returns `None` when the Scene is not
/// applied, so a stale document is never overwritten with another Scene's
/// Groups.
pub fn build_captured_groups(scene_id: &str, groups: &[Group]) -> Option<SceneGroups> {
    let mut captured: Vec<&Group> = groups
        .iter()
        .filter(|group| {
            group.scene_id.as_deref() == Some(scene_id)
                && group.transition_ghost != Some(true)
                && !is_gallery_group(group)
                && group.scene_row.is_some_and(|row| row < MAX_CAPTURED_ROWS)
        })
        .collect();

    // `sort((a, b) => a.sceneRow - b.sceneRow)` — a stable sort keeps the
    // live order for duplicate rows, and the last one written wins, exactly
    // like the JS assignment loop.
    captured.sort_by_key(|group| group.scene_row);

    if captured.is_empty() {
        return None;
    }

    let mut rows: BTreeMap<usize, Value> = BTreeMap::new();

    for group in captured {
        // The filter above guarantees the row.
        if let Some(row) = group.scene_row {
            insert_row(&mut rows, row, group_snapshot(group));
        }
    }

    Some(rows_from_sparse(rows))
}

/// `buildCapturedGroupsFromLive` (`scene-capture.js:68-93`).
///
/// Collects every live user Group (no Gallery, no transition ghosts). Used for
/// an empty Scene column, which has no stamped Groups to match.
pub fn build_captured_groups_from_live(groups: &[Group]) -> Option<SceneGroups> {
    let captured: Vec<&Group> = groups
        .iter()
        .filter(|group| group.transition_ghost != Some(true) && !is_gallery_group(group))
        .collect();

    if captured.is_empty() {
        return None;
    }

    let mut rows: BTreeMap<usize, Value> = BTreeMap::new();

    for (index, group) in captured.iter().enumerate() {
        // `Number.isFinite(group.sceneRow) && group.sceneRow >= 0
        //   ? Math.trunc(group.sceneRow) : i`; a `sceneRow` past the bound is
        // treated like a missing one and falls back to the live index.
        let row = group
            .scene_row
            .filter(|row| *row < MAX_CAPTURED_ROWS)
            .unwrap_or(index);

        insert_row(&mut rows, row, group_snapshot(group));
    }

    Some(rows_from_sparse(rows))
}

/// `sceneHasGroups` (`scene-capture.js:100-110`): a Scene with no Groups at
/// all is safe to fill from the live layer stack.
pub fn scene_has_groups(scene: &Value) -> bool {
    match scene.get("groups") {
        Some(Value::Array(groups)) => groups.iter().any(|group| !group.is_null()),
        _ => match scene.get("layers") {
            Some(Value::Array(layers)) => !layers.is_empty(),
            _ => false,
        },
    }
}

/// `collectSceneGroups` (`scene-capture.js:119-137`).
///
/// An applied Scene captures only its own stamped Groups (`from_live: false`).
/// An empty Scene captures the whole live layer stack instead
/// (`from_live: true`). A populated Scene that is not applied yields `None` so
/// the caller refuses to overwrite it.
pub fn collect_scene_groups(scene: &Value, live_groups: &[Group]) -> Option<CollectedSceneGroups> {
    let scene_id = scene.get("id").and_then(Value::as_str).unwrap_or_default();

    if !scene_id.is_empty()
        && let Some(stamped) = build_captured_groups(scene_id, live_groups)
    {
        return Some(CollectedSceneGroups {
            groups: stamped.groups,
            from_live: false,
        });
    }

    if scene_has_groups(scene) {
        return None;
    }

    build_captured_groups_from_live(live_groups).map(|live| CollectedSceneGroups {
        groups: live.groups,
        from_live: true,
    })
}

/// `canCaptureScene` (`scene-capture.js:139-141`).
pub fn can_capture_scene(scene: &Value, live_groups: &[Group]) -> bool {
    collect_scene_groups(scene, live_groups).is_some()
}

/// Capture the applied Scene from the engine state.
///
/// This is the engine-facing entry point: it refuses while performance mode is
/// running, requires at least one live Group stamped with `scene_id`, and
/// returns the row-indexed snapshot.
///
/// `row_count` is the performance grid height ([`crate::scene_rows::get_row_count`]).
/// The returned array is padded with `None` to that height when the grid is
/// taller than the stamped rows, so a capture keeps the column's row shape.
/// Pass `0` for "derive the height from the stamped rows" (the JS behaviour).
/// The height is clamped to [`MAX_CAPTURED_ROWS`] for the same reason the row
/// indexes are: `get_row_count` derives it from a JSON `sceneRow`.
pub fn capture_live_state(
    state: &EngineState,
    scene_id: &str,
    row_count: usize,
    performance_running: bool,
) -> Result<SceneGroups, CaptureError> {
    if performance_running {
        return Err(CaptureError::PerformanceRunning);
    }

    let mut captured =
        build_captured_groups(scene_id, &state.groups).ok_or(CaptureError::SceneNotApplied)?;

    let row_count = row_count.min(MAX_CAPTURED_ROWS);

    if captured.groups.len() < row_count {
        captured.groups.resize(row_count, None);
    }

    Ok(captured)
}

#[cfg(test)]
mod tests {
    //! Port of `tests/unit/scene-capture.test.js` (Vitest) plus the typed
    //! [`capture_live_state`] refusal rules from `scenes.js`.
    use super::*;
    use crate::engine_state::{GROUP_ENABLED, InheritFrom};
    use serde_json::{Map, json};

    fn live_group(id: &str, name: &str) -> Group {
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

    fn stamped(id: &str, name: &str, scene_id: &str, scene_row: usize) -> Group {
        let mut group = live_group(id, name);
        group.scene_id = Some(scene_id.to_string());
        group.scene_row = Some(scene_row);
        group.modules.push("m1".to_string());
        group
    }

    fn scene(id: &str, groups: Value) -> Value {
        json!({"id": id, "name": id, "groups": groups})
    }

    fn ids(captured: &SceneGroups) -> Vec<Option<&str>> {
        captured
            .groups
            .iter()
            .map(|group| group.as_ref().and_then(|group| group["id"].as_str()))
            .collect()
    }

    // ————————————————— buildCapturedGroups —————————————————

    #[test]
    fn build_captured_groups_returns_none_when_the_scene_is_not_applied() {
        // scene-capture.test.js:22-29
        let live = [
            stamped("g1", "Other", "s2", 0),
            live_group("g2", "Unassigned"),
        ];
        assert!(build_captured_groups("s1", &live).is_none());
    }

    #[test]
    fn build_captured_groups_captures_only_the_scenes_groups() {
        // scene-capture.test.js:31-46
        let mut own = stamped("g2", "Scene group", "s1", 0);
        own.alpha = 0.5;
        let live = [
            stamped("g1", "Other", "s2", 0),
            own,
            live_group("g3", "Unassigned"),
        ];
        let captured = build_captured_groups("s1", &live).unwrap();

        assert_eq!(captured.groups.len(), 1);
        assert_eq!(captured.groups[0].as_ref().unwrap()["id"], json!("g2"));
        assert_eq!(captured.groups[0].as_ref().unwrap()["alpha"], json!(0.5));
    }

    #[test]
    fn build_captured_groups_excludes_transition_ghosts_and_the_gallery() {
        // scene-capture.test.js:48-65
        let mut ghost = stamped("g1", "Ghost", "s1", 0);
        ghost.transition_ghost = Some(true);
        let live = [
            ghost,
            stamped("g2", GALLERY_GROUP_NAME, "s1", 1),
            stamped("g3", "Real", "s1", 2),
        ];
        let captured = build_captured_groups("s1", &live).unwrap();

        assert_eq!(ids(&captured), vec![None, None, Some("g3")]);
    }

    #[test]
    fn build_captured_groups_stops_at_the_last_captured_row() {
        // scene-capture.test.js:67-80 — rows 0 and 2 capture, row 1 is a hole
        // and no row 3+ is invented.
        let live = [
            stamped("g0", "Row 0", "s1", 0),
            stamped("g2", "Row 2", "s1", 2),
        ];
        let captured = build_captured_groups("s1", &live).unwrap();

        assert_eq!(captured.groups.len(), 3);
        assert_eq!(ids(&captured), vec![Some("g0"), None, Some("g2")]);
    }

    #[test]
    fn build_captured_groups_ignores_an_out_of_range_scene_row() {
        // `sceneRow` comes from JSON; a corrupt/hostile value must not size an
        // allocation (`vec![None; row + 1]` used to abort the process).
        let live = [
            stamped("g0", "Row 0", "s1", 0),
            stamped("bad", "Bad", "s1", usize::MAX),
        ];
        let captured = build_captured_groups("s1", &live).unwrap();

        assert_eq!(ids(&captured), vec![Some("g0")]);

        // A Scene whose only stamped Group has a bogus row is not capturable.
        let only_bad = [stamped("bad", "Bad", "s1", usize::MAX)];
        assert!(build_captured_groups("s1", &only_bad).is_none());

        // The 32-bit hostile value from the review is rejected the same way.
        let wide = [stamped("bad", "Bad", "s1", 4_294_967_295)];
        assert!(build_captured_groups("s1", &wide).is_none());
    }

    #[test]
    fn max_captured_rows_bounds_the_array() {
        let live = [stamped("last", "Last", "s1", MAX_CAPTURED_ROWS)];
        assert!(
            build_captured_groups("s1", &live).is_none(),
            "a row index at the bound is out of range"
        );

        let live = [stamped("edge", "Edge", "s1", MAX_CAPTURED_ROWS - 1)];
        let captured = build_captured_groups("s1", &live).unwrap();
        assert_eq!(captured.groups.len(), MAX_CAPTURED_ROWS);
        assert_eq!(
            captured.groups[MAX_CAPTURED_ROWS - 1].as_ref().unwrap()["id"],
            json!("edge")
        );
    }

    #[test]
    fn build_captured_groups_clones_groups_and_strips_render_fields() {
        // scene-capture.test.js:82-102
        let mut live = stamped("g1", "Real", "s1", 0);
        live.extra
            .insert("context".to_string(), json!({"ctx": true}));
        live.extra.insert("writeToSwap".to_string(), json!(true));
        live.extra.insert("readFromSwap".to_string(), json!(false));

        let captured = build_captured_groups("s1", &[live.clone()]).unwrap();
        let snapshot = captured.groups[0].as_ref().unwrap();

        assert!(snapshot.get("context").is_none());
        assert!(snapshot.get("writeToSwap").is_none());
        assert!(snapshot.get("readFromSwap").is_none());
        assert_eq!(snapshot["name"], json!("Real"));

        // The live Group is untouched.
        assert_eq!(live.name, "Real");
    }

    #[test]
    fn build_captured_groups_ignores_groups_without_a_scene_row() {
        // scene-capture.test.js:104-110
        let mut dangling = live_group("g1", "Dangling");
        dangling.scene_id = Some("s1".to_string());
        assert!(build_captured_groups("s1", &[dangling]).is_none());
    }

    // ————————————————— buildCapturedGroupsFromLive —————————————————

    #[test]
    fn build_captured_groups_from_live_returns_none_without_user_groups() {
        // scene-capture.test.js:113-123
        assert!(build_captured_groups_from_live(&[]).is_none());

        let mut ghost = live_group("g2", "Ghost");
        ghost.transition_ghost = Some(true);
        assert!(
            build_captured_groups_from_live(&[live_group("g1", GALLERY_GROUP_NAME), ghost])
                .is_none()
        );
    }

    #[test]
    fn build_captured_groups_from_live_drops_gallery_and_ghosts() {
        // scene-capture.test.js:125-133
        let mut ghost = live_group("g2", "Ghost");
        ghost.transition_ghost = Some(true);
        let captured = build_captured_groups_from_live(&[
            live_group("g1", GALLERY_GROUP_NAME),
            ghost,
            live_group("g3", "Foreground"),
        ])
        .unwrap();

        assert_eq!(ids(&captured), vec![Some("g3")]);
    }

    #[test]
    fn build_captured_groups_from_live_uses_scene_rows_and_keeps_holes() {
        // scene-capture.test.js:135-146
        let mut row2 = live_group("g2", "Row 2");
        row2.scene_row = Some(2);
        let mut row0 = live_group("g0", "Row 0");
        row0.scene_row = Some(0);

        let captured = build_captured_groups_from_live(&[row2, row0]).unwrap();

        assert_eq!(ids(&captured), vec![Some("g0"), None, Some("g2")]);
    }

    #[test]
    fn build_captured_groups_from_live_falls_back_to_live_order() {
        // scene-capture.test.js:148-158
        let captured = build_captured_groups_from_live(&[
            live_group("g0", "First"),
            live_group("g1", "Second"),
        ])
        .unwrap();

        assert_eq!(ids(&captured), vec![Some("g0"), Some("g1")]);
    }

    #[test]
    fn build_captured_groups_from_live_falls_back_for_a_hostile_scene_row() {
        let mut bad = live_group("bad", "Bad");
        bad.scene_row = Some(usize::MAX);
        let captured = build_captured_groups_from_live(&[bad]).unwrap();

        // The layer is still captured, at its live index, instead of sizing a
        // `usize::MAX` array.
        assert_eq!(ids(&captured), vec![Some("bad")]);
    }

    #[test]
    fn build_captured_groups_from_live_clones_and_strips_render_fields() {
        // scene-capture.test.js:160-173
        let mut live = live_group("g1", "Real");
        live.extra
            .insert("context".to_string(), json!({"ctx": true}));
        live.extra.insert("writeToSwap".to_string(), json!(true));
        live.extra.insert("readFromSwap".to_string(), json!(false));

        let captured = build_captured_groups_from_live(&[live.clone()]).unwrap();
        let snapshot = captured.groups[0].as_ref().unwrap();

        assert!(snapshot.get("context").is_none());
        assert!(snapshot.get("writeToSwap").is_none());
        assert!(snapshot.get("readFromSwap").is_none());
        assert_eq!(live.name, "Real");
    }

    // ————————————————— sceneHasGroups —————————————————

    #[test]
    fn scene_has_groups_detects_populated_and_empty_documents() {
        // scene-capture.test.js:176-186
        assert!(!scene_has_groups(&scene("s1", json!([]))));
        assert!(!scene_has_groups(&scene("s1", json!([null, null]))));
        assert!(scene_has_groups(&scene("s1", json!([null, {"id": "g1"}]))));
        assert!(!scene_has_groups(&json!({"id": "legacy", "layers": []})));
        assert!(scene_has_groups(
            &json!({"id": "legacy", "layers": [{"$id": "m1"}]})
        ));
    }

    // ————————————————— collectSceneGroups —————————————————

    #[test]
    fn collect_scene_groups_returns_none_for_an_unapplied_populated_scene() {
        // scene-capture.test.js:188-195
        let live = [stamped("g1", "Other", "s2", 0)];
        assert!(collect_scene_groups(&scene("s1", json!([{"id": "g1"}])), &live).is_none());
    }

    #[test]
    fn collect_scene_groups_captures_an_applied_scenes_stamped_groups() {
        // scene-capture.test.js:197-204
        let live = [stamped("g1", "Own", "s1", 0)];
        let collected = collect_scene_groups(&scene("s1", json!([])), &live).unwrap();

        assert!(!collected.from_live);
        assert_eq!(
            ids(&SceneGroups {
                groups: collected.groups
            }),
            vec![Some("g1")]
        );
    }

    #[test]
    fn collect_scene_groups_captures_the_live_stack_into_an_empty_column() {
        // scene-capture.test.js:206-217
        let live = [
            live_group("g1", "Foreground"),
            live_group("g2", "Background"),
        ];
        let collected = collect_scene_groups(&scene("s1", json!([])), &live).unwrap();

        assert!(collected.from_live);
        assert_eq!(
            ids(&SceneGroups {
                groups: collected.groups
            }),
            vec![Some("g1"), Some("g2")]
        );
    }

    #[test]
    fn collect_scene_groups_returns_none_for_an_empty_scene_with_nothing_live() {
        // scene-capture.test.js:219-223
        let live = [live_group("g1", GALLERY_GROUP_NAME)];
        assert!(collect_scene_groups(&scene("s1", json!([])), &live).is_none());
    }

    // ————————————————— canCaptureScene —————————————————

    #[test]
    fn can_capture_scene_allows_applied_and_empty_scenes_only() {
        // scene-capture.test.js:226-234
        let empty = scene("s1", json!([]));
        let populated = scene("s2", json!([{"id": "g1"}]));
        let live = [stamped("g1", "Own", "s1", 0)];

        assert!(can_capture_scene(&empty, &live));
        assert!(!can_capture_scene(&populated, &live));
    }

    // ————————————————— captureLiveState —————————————————

    fn state_with(groups: Vec<Group>) -> EngineState {
        let mut state = EngineState::new();
        state.groups = groups;
        state
    }

    #[test]
    fn capture_live_state_refuses_while_performance_runs() {
        // scenes.js:1963-1966
        let state = state_with(vec![stamped("g1", "Own", "s1", 0)]);
        assert_eq!(
            capture_live_state(&state, "s1", 3, true).unwrap_err(),
            CaptureError::PerformanceRunning
        );
    }

    #[test]
    fn capture_live_state_refuses_an_unapplied_scene() {
        // scenes.js:1956-1990 — only an applied Scene is captured.
        let state = state_with(vec![stamped("g1", "Other", "s2", 0)]);
        assert_eq!(
            capture_live_state(&state, "s1", 3, false).unwrap_err(),
            CaptureError::SceneNotApplied
        );
    }

    #[test]
    fn capture_live_state_returns_row_indexed_groups() {
        let state = state_with(vec![
            stamped("g0", "Background", "s1", 0),
            stamped("g2", "Effects", "s1", 2),
            stamped("other", "Other", "s2", 1),
        ]);
        let captured = capture_live_state(&state, "s1", 0, false).unwrap();

        assert_eq!(ids(&captured), vec![Some("g0"), None, Some("g2")]);
    }

    #[test]
    fn capture_live_state_pads_to_the_grid_height() {
        let state = state_with(vec![stamped("g0", "Background", "s1", 0)]);
        let captured = capture_live_state(&state, "s1", 4, false).unwrap();

        assert_eq!(captured.groups.len(), 4);
        assert_eq!(captured.groups[0].as_ref().unwrap()["id"], json!("g0"));
        assert!(captured.groups[1..].iter().all(Option::is_none));
    }

    #[test]
    fn capture_live_state_clamps_a_hostile_grid_height() {
        let state = state_with(vec![stamped("g0", "Background", "s1", 0)]);
        let captured = capture_live_state(&state, "s1", usize::MAX, false).unwrap();

        assert_eq!(captured.groups.len(), MAX_CAPTURED_ROWS);
    }

    #[test]
    fn capture_live_state_ignores_ghosts_and_the_gallery() {
        let mut ghost = stamped("ghost", "Ghost", "s1", 1);
        ghost.transition_ghost = Some(true);
        let state = state_with(vec![
            stamped("g0", "Background", "s1", 0),
            ghost,
            stamped("gallery", GALLERY_GROUP_NAME, "s1", 2),
        ]);
        let captured = capture_live_state(&state, "s1", 0, false).unwrap();

        assert_eq!(ids(&captured), vec![Some("g0")]);
    }

    #[test]
    fn capture_error_messages_are_stable() {
        assert!(
            CaptureError::PerformanceRunning
                .to_string()
                .contains("performance mode")
        );
        assert!(
            CaptureError::SceneNotApplied
                .to_string()
                .contains("applied")
        );
        assert!(
            CaptureError::NoLiveGroups
                .to_string()
                .contains("live Groups")
        );
    }
}
