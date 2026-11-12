//! Scene row ↔ Group matching and performance-grid row selection.
//!
//! Ported from `src/renderer/src/application/worker/scene-rows.js` (431
//! lines). The behaviours that must be preserved (WS-8, "Row names"):
//!
//! - A named row matches its Group by name, **case-insensitively**, either
//!   exactly or by word prefix (`webcam` matches `Webcam 2`), first match
//!   wins. Rows without a name keep the legacy positional lookup.
//! - A digit-suffixed row (`effects2`) only falls back to the base-named Group
//!   (`Effects`) when no sibling row already owns that family
//!   ([`row_family_is_shared`]) — otherwise two rows would claim one Group.
//! - Rows advance independently on a per-cell dwell target
//!   ([`should_advance_row`]) and the candidate set for a row excludes
//!   disabled Scenes and empty cells ([`pick_next_row_scene`]).
//!
//! ## JS → Rust notes
//!
//! - The JS works on plain scene objects and compares identity. The Rust port
//!   works on [`serde_json::Value`] and returns *indexes* (`get_row_group_index`)
//!   with `get_row_group` as the reference-returning convenience.
//! - `rowNames` entries may be `null`/`undefined`/blank in JS. The Rust port
//!   normalises them to `""` ([`normalize_row_names`]) and every function trims
//!   again on read, exactly like `getRowNameIndex`.
//! - The JS `random` parameter is only called when a random pick actually
//!   happens. The Rust port takes an `FnMut() -> f64` so the same call-count
//!   behaviour is testable.
//! - `switchChance` is a 5-line pure helper from `phrase-detector.js:87-96`;
//!   it is duplicated here as [`switch_chance`] rather than pulling all of
//!   `modv-media` into the engine crate. `modv_media::audio::phrase` is the
//!   canonical port.
//!
//! Line numbers refer to the tree at commit `8a512b6e` (branch `rust-hybrid`).

use std::collections::HashSet;

use serde_json::Value;

use crate::engine_state::GALLERY_GROUP_NAME;
use crate::scene_timing::clamp_cell_dwell_numeric;

/// `isGalleryGroup` (`scene-rows.js:5-6`): the Group is the internal Gallery.
pub fn is_gallery_group(group: &Value) -> bool {
    group.get("name").and_then(Value::as_str) == Some(GALLERY_GROUP_NAME)
}

/// `hasModules` (`scene-rows.js:8-20`): supports both array-shaped and
/// object-shaped module maps.
pub fn has_modules(group: &Value) -> bool {
    match group.get("modules") {
        Some(Value::Array(modules)) => !modules.is_empty(),
        Some(Value::Object(modules)) => !modules.is_empty(),
        _ => false,
    }
}

/// `getSceneGroups` (`scene-rows.js:22-23`): the Scene's Groups, or an empty
/// slice when the Scene has no `groups` array.
pub fn get_scene_groups(scene: &Value) -> &[Value] {
    scene
        .get("groups")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

/// `isSceneEnabled` (`scene-rows.js:25`): only an explicit `false` disables a
/// Scene (`enabled: 0` still counts as enabled in JS).
pub fn is_scene_enabled(scene: &Value) -> bool {
    scene.is_object() && !matches!(scene.get("enabled"), Some(Value::Bool(false)))
}

/// `getRowNameIndex` (`scene-rows.js:27-32`): the trimmed name at a row, or
/// `""` when the row has no name. Non-string entries read as unnamed.
pub fn get_row_name_index(row_names: &[String], row_index: usize) -> String {
    row_names
        .get(row_index)
        .map(|name| name.trim().to_string())
        .unwrap_or_default()
}

/// `baseName` (`scene-rows.js:38-42`): lower-case and strip a trailing number
/// plus the single separator space before it, so `Effects`, `Effects 2` and
/// `effects2` share the family `effects`.
pub fn base_name(value: &str) -> String {
    let trimmed = value.trim().to_lowercase();
    let bytes = trimmed.as_bytes();
    let mut end = bytes.len();

    while end > 0 && bytes[end - 1].is_ascii_digit() {
        end -= 1;
    }

    if end == bytes.len() {
        return trimmed;
    }

    // The JS regex ` ?\d+$` also eats one separator space.
    if end > 0 && bytes[end - 1] == b' ' {
        end -= 1;
    }

    trimmed[..end].to_string()
}

/// `strictMatchesRowName` (`scene-rows.js:48-61`): exact name, or the row
/// name followed by a space (`webcam` matches `Webcam 2`).
pub fn strict_matches_row_name(group: &Value, row_name: &str) -> bool {
    let Some(name) = group.get("name").and_then(Value::as_str) else {
        return false;
    };

    let key = name.trim().to_lowercase();
    let row_key = row_name.trim().to_lowercase();

    if row_key.is_empty() {
        return false;
    }

    key == row_key || key.starts_with(&format!("{row_key} "))
}

/// `matchesRowName` (`scene-rows.js:68-81`): the strict match plus the loose
/// digit-suffix family match.
pub fn matches_row_name(group: &Value, row_name: &str) -> bool {
    if strict_matches_row_name(group, row_name) {
        return true;
    }

    let Some(name) = group.get("name").and_then(Value::as_str) else {
        return false;
    };

    let key_base = base_name(name);
    let row_base = base_name(row_name);

    !key_base.is_empty() && !row_base.is_empty() && key_base == row_base
}

/// `rowFamilyIsShared` (`scene-rows.js:88-113`): another row already owns the
/// base-name family, so a digit-suffixed row must not fall back to it.
pub fn row_family_is_shared(row_index: usize, row_names: &[String]) -> bool {
    let row_key = get_row_name_index(row_names, row_index);

    if row_key.is_empty() {
        return false;
    }

    let family = base_name(&row_key);

    if family.is_empty() {
        return false;
    }

    for (index, name) in row_names.iter().enumerate() {
        if index == row_index || name.trim().is_empty() {
            continue;
        }

        if base_name(name) == family {
            return true;
        }
    }

    false
}

/// `getRowGroup` (`scene-rows.js:119-151`) as an index into
/// [`get_scene_groups`].
///
/// Strict matches win before the loose family fallback, so a `2`-suffixed row
/// cannot grab the base-named Group from the base row. `None` also covers a
/// `null` row hole (the JS `groups[rowIndex] || null`).
pub fn get_row_group_index(scene: &Value, row_index: usize, row_names: &[String]) -> Option<usize> {
    let groups = get_scene_groups(scene);
    let row_name = get_row_name_index(row_names, row_index);

    if row_name.is_empty() {
        // Legacy positional lookup; a `null` hole is "no Group".
        return match groups.get(row_index) {
            Some(group) if !group.is_null() => Some(row_index),
            _ => None,
        };
    }

    let row_key = row_name.trim().to_lowercase();

    for (index, group) in groups.iter().enumerate() {
        if strict_matches_row_name(group, &row_key) {
            return Some(index);
        }
    }

    if row_family_is_shared(row_index, row_names) {
        return None;
    }

    for (index, group) in groups.iter().enumerate() {
        if matches_row_name(group, &row_key) {
            return Some(index);
        }
    }

    None
}

/// [`get_row_group_index`] returning the Group itself
/// (`scene-rows.js:119-151`).
pub fn get_row_group<'a>(
    scene: &'a Value,
    row_index: usize,
    row_names: &[String],
) -> Option<&'a Value> {
    let index = get_row_group_index(scene, row_index, row_names)?;

    get_scene_groups(scene).get(index)
}

/// `isRowCellEmpty` (`scene-rows.js:153-157`): no Group, or a Group without
/// modules.
pub fn is_row_cell_empty(scene: &Value, row_index: usize, row_names: &[String]) -> bool {
    match get_row_group(scene, row_index, row_names) {
        Some(group) => !has_modules(group),
        None => true,
    }
}

/// `normalizeRowNames` (`scene-rows.js:159-167`): index-exact, so blanks and
/// duplicates are kept and only whitespace is trimmed. Non-array input is
/// empty.
pub fn normalize_row_names(value: Option<&Value>) -> Vec<String> {
    match value.and_then(Value::as_array) {
        Some(names) => names
            .iter()
            .map(|name| {
                name.as_str()
                    .map(|name| name.trim().to_string())
                    .unwrap_or_default()
            })
            .collect(),
        None => Vec::new(),
    }
}

/// `assignSceneRowIndexes` (`scene-rows.js:179-240`): one row index per Group
/// snapshot (holes stay `None`).
///
/// Named rows win over the Group's document position, so applying a Scene
/// places its Groups on the cells that show them; unmatched Groups are slotted
/// below the named rows in document order. Without any named row the
/// historical positional mapping is returned unchanged.
pub fn assign_scene_row_indexes(groups: &[Value], row_names: &[String]) -> Vec<Option<usize>> {
    let names: Vec<String> = row_names
        .iter()
        .map(|name| name.trim().to_string())
        .collect();

    if !names.iter().any(|name| !name.is_empty()) {
        return (0..groups.len()).map(Some).collect();
    }

    let mut indexes: Vec<Option<usize>> = vec![None; groups.len()];
    let mut taken_groups: HashSet<usize> = HashSet::new();
    let mut taken_rows: HashSet<usize> = HashSet::new();

    for (row_index, name) in names.iter().enumerate() {
        if name.is_empty() {
            continue;
        }

        let mut group_index = None;

        for (index, group) in groups.iter().enumerate() {
            if !taken_groups.contains(&index) && strict_matches_row_name(group, name) {
                group_index = Some(index);
                break;
            }
        }

        if group_index.is_none() && !row_family_is_shared(row_index, &names) {
            for (index, group) in groups.iter().enumerate() {
                if !taken_groups.contains(&index) && matches_row_name(group, name) {
                    group_index = Some(index);
                    break;
                }
            }
        }

        if let Some(group_index) = group_index {
            indexes[group_index] = Some(row_index);
            taken_groups.insert(group_index);
            taken_rows.insert(row_index);
        }
    }

    let mut next_row = names.len();

    for (index, group) in groups.iter().enumerate() {
        if group.is_null() || taken_groups.contains(&index) {
            continue;
        }

        while taken_rows.contains(&next_row) {
            next_row += 1;
        }

        indexes[index] = Some(next_row);
        taken_rows.insert(next_row);
        next_row += 1;
    }

    indexes
}

/// `getRowCount` (`scene-rows.js:242-278`): the grid height.
///
/// Takes the longest of the named rows, the widest Scene Group list (Gallery
/// excluded), and the live Groups' stamped `sceneRow`/count. Transition ghosts
/// and the Gallery Group never contribute; a corrupted/`null` entry is skipped
/// rather than panicking.
pub fn get_row_count(scenes: &[Value], live_groups: &[Value], row_names: &[String]) -> usize {
    let mut row_count = row_names.len();

    for scene in scenes {
        let groups = get_scene_groups(scene)
            .iter()
            .filter(|group| !is_gallery_group(group))
            .count();

        row_count = row_count.max(groups);
    }

    let mut live_count = 0usize;

    for group in live_groups {
        if group.is_null() || is_gallery_group(group) {
            continue;
        }

        if group.get("transitionGhost").and_then(Value::as_bool) == Some(true) {
            continue;
        }

        live_count += 1;

        // `Number.isFinite(group.sceneRow) ? group.sceneRow + 1 : 0`
        let scene_row = group
            .get("sceneRow")
            .and_then(Value::as_f64)
            .filter(|row| row.is_finite())
            .map(|row| (row + 1.0).max(0.0))
            .unwrap_or(0.0);

        row_count = row_count.max(scene_row as usize);
    }

    row_count.max(live_count)
}

/// `getRowLabel` (`scene-rows.js:280-321`): the explicit row name, else the
/// Group name carried by a strict majority of Scenes at that row, else
/// `Row N`.
///
/// Scenes without a Group on the row count into the denominator, so a name
/// shared by a handful of legacy Scenes cannot dress an unnamed row up.
pub fn get_row_label(scenes: &[Value], row_index: usize, row_names: &[String]) -> String {
    let row_name = get_row_name_index(row_names, row_index);

    if !row_name.is_empty() {
        return row_name;
    }

    let denominator = scenes.len().max(1);
    // Insertion order matters (JS `Map`), so counts live in a Vec.
    let mut counts: Vec<(String, usize)> = Vec::new();

    for scene in scenes {
        let Some(group) = get_scene_groups(scene).get(row_index) else {
            continue;
        };
        let Some(name) = group.get("name").and_then(Value::as_str) else {
            continue;
        };

        if name.is_empty() {
            continue;
        }

        match counts.iter_mut().find(|(key, _)| key == name) {
            Some((_, count)) => *count += 1,
            None => counts.push((name.to_string(), 1)),
        }
    }

    let mut label: Option<String> = None;
    let mut best_count = 0usize;

    for (name, count) in counts {
        if count * 2 > denominator && count > best_count {
            label = Some(name);
            best_count = count;
        }
    }

    label.unwrap_or_else(|| format!("Row {}", row_index + 1))
}

/// `getCandidateScenes` (`scene-rows.js:323-327`): enabled Scenes with a
/// non-empty cell at the row.
fn get_candidate_scenes<'a>(
    scenes: &'a [Value],
    row_index: usize,
    row_names: &[String],
) -> Vec<&'a Value> {
    scenes
        .iter()
        .filter(|scene| is_scene_enabled(scene) && !is_row_cell_empty(scene, row_index, row_names))
        .collect()
}

/// Arguments for [`pick_next_row_scene`] (`scene-rows.js:329-367`).
pub struct PickNextRowScene<'a> {
    /// The Scenes to choose from.
    pub scenes: &'a [Value],
    /// The row whose cells are being considered.
    pub row_index: usize,
    /// The Scene the row is playing now.
    pub current_scene_id: Option<&'a str>,
    /// `"in-order"` (default) or `"random"`.
    pub order: &'a str,
    /// Named rows; empty for the legacy positional grid.
    pub row_names: &'a [String],
}

/// `pickNextRowScene` (`scene-rows.js:329-367`): the next Scene for one row.
///
/// Returns `None` with no candidates, the only candidate when there is one,
/// otherwise the next candidate in order (wrapping) or — for
/// `order == "random"` — a random pick that excludes the current Scene unless
/// it is the only candidate. `random` is only called on the random branch.
pub fn pick_next_row_scene<'a, F>(args: &PickNextRowScene<'a>, mut random: F) -> Option<&'a Value>
where
    F: FnMut() -> f64,
{
    let candidates = get_candidate_scenes(args.scenes, args.row_index, args.row_names);

    if candidates.is_empty() {
        return None;
    }

    if candidates.len() == 1 {
        return Some(candidates[0]);
    }

    if args.order == "random" {
        let pool: Vec<&Value> = candidates
            .iter()
            .copied()
            .filter(|scene| scene.get("id").and_then(Value::as_str) != args.current_scene_id)
            .collect();
        let choices = if pool.is_empty() { &candidates } else { &pool };
        // `Math.min(choices.length - 1, Math.floor(random() * choices.length))`
        let index = (random() * choices.len() as f64).floor();
        // Out-of-range rolls are clamped rather than yielding `undefined`; a
        // real `Math.random` roll is always in `[0, 1)`.
        let index = index.clamp(0.0, (choices.len() - 1) as f64) as usize;

        return Some(choices[index]);
    }

    let current_index = args.current_scene_id.and_then(|id| {
        candidates
            .iter()
            .position(|scene| scene.get("id").and_then(Value::as_str) == Some(id))
    });

    match current_index {
        None => Some(candidates[0]),
        Some(index) => Some(candidates[(index + 1) % candidates.len()]),
    }
}

/// Arguments for [`resolve_row_advance_scene`] (`scene-rows.js:374-405`).
pub struct ResolveRowAdvanceScene<'a> {
    /// The Scenes to choose from.
    pub scenes: &'a [Value],
    /// The row whose cells are being considered.
    pub row_index: usize,
    /// The Scene the row is playing now.
    pub current_scene_id: Option<&'a str>,
    /// The Scene prewarmed for this move, if any.
    pub pending_scene_id: Option<&'a str>,
    /// `"in-order"` (default) or `"random"`.
    pub order: &'a str,
    /// Named rows; empty for the legacy positional grid.
    pub row_names: &'a [String],
}

/// `resolveRowAdvanceScene` (`scene-rows.js:374-405`): the Scene a row should
/// move to.
///
/// A prewarmed Scene is used only when it is still a valid candidate (enabled
/// and non-empty at this row), so the warmed GPU instance is the one that
/// loads; otherwise a fresh pick is made.
pub fn resolve_row_advance_scene<'a, F>(
    args: &ResolveRowAdvanceScene<'a>,
    mut random: F,
) -> Option<&'a Value>
where
    F: FnMut() -> f64,
{
    if let Some(pending_id) = args.pending_scene_id {
        let pending = args
            .scenes
            .iter()
            .find(|scene| scene.get("id").and_then(Value::as_str) == Some(pending_id));

        if let Some(pending) = pending {
            if is_scene_enabled(pending)
                && !is_row_cell_empty(pending, args.row_index, args.row_names)
            {
                return Some(pending);
            }
        }
    }

    pick_next_row_scene(
        &PickNextRowScene {
            scenes: args.scenes,
            row_index: args.row_index,
            current_scene_id: args.current_scene_id,
            order: args.order,
            row_names: args.row_names,
        },
        &mut random,
    )
}

/// Arguments for [`should_advance_row`] (`scene-rows.js:407-431`).
pub struct ShouldAdvanceRow {
    /// Seconds the cell has been active.
    pub dwell: f64,
    /// Global minimum dwell, in seconds.
    pub min_seconds: Option<f64>,
    /// Global maximum dwell, in seconds.
    pub max_seconds: Option<f64>,
    /// The per-cell target chosen when the cell became active.
    pub target_seconds: Option<f64>,
    /// Whether the phrase detector flipped this frame.
    pub phrase_flipped: bool,
}

/// The outcome of [`should_advance_row`] (`scene-rows.js:426-430`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RowAdvance {
    /// The row should move to its next Scene.
    pub advance: bool,
    /// The advance came from the phrase/musical roll.
    pub musical: bool,
    /// The dwell deadline was reached.
    pub forced: bool,
}

/// Probability a phrase change switches Scenes (`phrase-detector.js:87-96`):
/// ramps linearly from 0 at `minSeconds` to 1 at `maxSeconds`.
pub fn switch_chance(elapsed: f64, min_seconds: f64, max_seconds: f64) -> f64 {
    if max_seconds <= min_seconds {
        return 1.0;
    }

    ((elapsed - min_seconds) / (max_seconds - min_seconds)).clamp(0.0, 1.0)
}

/// `shouldAdvanceRow` (`scene-rows.js:407-431`).
///
/// The deadline is the per-cell target clamped into `[min, max]`; a missing
/// target means the maximum (legacy behaviour). The musical roll only happens
/// after a phrase flip, and `random` is only called then.
pub fn should_advance_row<F>(args: &ShouldAdvanceRow, mut random: F) -> RowAdvance
where
    F: FnMut() -> f64,
{
    let target = args.target_seconds.or(args.max_seconds).or(Some(f64::NAN));
    let deadline = clamp_cell_dwell_numeric(target, args.min_seconds, args.max_seconds);
    let forced = args.dwell >= deadline;
    let musical = if args.phrase_flipped {
        // A missing bound behaves like the JS `NaN` comparison: the chance is
        // unusable, so the roll never succeeds.
        let chance = match (args.min_seconds, args.max_seconds) {
            (Some(min), Some(max)) => switch_chance(args.dwell, min, max),
            _ => 0.0,
        };

        random() < chance
    } else {
        false
    };

    RowAdvance {
        advance: forced || musical,
        musical,
        forced,
    }
}

#[cfg(test)]
mod tests {
    //! Port of `tests/unit/scene-rows.test.js` (Vitest).
    use super::*;
    use serde_json::json;

    fn group(name: &str, modules: Value) -> Value {
        json!({"name": name, "modules": modules})
    }

    fn group_with(name: &str, modules: Value, extra: Value) -> Value {
        let mut value = group(name, modules);
        if let (Some(target), Some(extra)) = (value.as_object_mut(), extra.as_object()) {
            for (key, item) in extra {
                target.insert(key.clone(), item.clone());
            }
        }
        value
    }

    fn scene(id: &str, groups: Value) -> Value {
        json!({"id": id, "name": id, "groups": groups})
    }

    fn a(name: &str) -> Value {
        group(name, json!(["module-a"]))
    }

    fn b(name: &str) -> Value {
        group(name, json!(["module-b"]))
    }

    fn c(name: &str) -> Value {
        group(name, json!(["module-c"]))
    }

    fn empty(name: &str) -> Value {
        group(name, json!([]))
    }

    fn names(values: &[&str]) -> Vec<String> {
        values.iter().map(|name| name.to_string()).collect()
    }

    // ————————————————— getSceneGroups —————————————————

    #[test]
    fn get_scene_groups_returns_the_scene_groups() {
        // scene-rows.test.js:33-36
        let scene = json!({"groups": [a("A"), b("B")]});
        assert_eq!(get_scene_groups(&scene).len(), 2);
        assert_eq!(get_scene_groups(&scene)[0]["name"], json!("A"));
    }

    #[test]
    fn get_scene_groups_is_empty_without_groups() {
        // scene-rows.test.js:38-42
        assert!(get_scene_groups(&json!({})).is_empty());
        assert!(get_scene_groups(&Value::Null).is_empty());
        assert!(get_scene_groups(&json!({"groups": "nope"})).is_empty());
    }

    // ————————————————— isSceneEnabled —————————————————

    #[test]
    fn only_an_explicit_false_disables_a_scene() {
        // scene-rows.test.js:45-51
        assert!(!is_scene_enabled(&json!({"enabled": false})));
        assert!(is_scene_enabled(&json!({"enabled": true})));
        assert!(is_scene_enabled(&json!({})));
        assert!(is_scene_enabled(&json!({"enabled": 0})));
    }

    // ————————————————— isRowCellEmpty —————————————————

    #[test]
    fn row_cell_is_empty_when_the_group_is_missing() {
        // scene-rows.test.js:55-58
        assert!(is_row_cell_empty(&scene("s", json!([a("A")])), 1, &[]));
        assert!(is_row_cell_empty(&json!({}), 0, &[]));
    }

    #[test]
    fn row_cell_is_empty_when_the_group_has_no_modules() {
        // scene-rows.test.js:60-63
        assert!(is_row_cell_empty(
            &scene("s", json!([empty("Empty")])),
            0,
            &[]
        ));
        assert!(is_row_cell_empty(
            &scene("s", json!([{"name": "No modules"}])),
            0,
            &[]
        ));
    }

    #[test]
    fn row_cell_is_not_empty_when_the_group_has_modules() {
        // scene-rows.test.js:65-67
        assert!(!is_row_cell_empty(&scene("s", json!([a("A")])), 0, &[]));
    }

    #[test]
    fn row_cell_supports_object_shaped_module_maps() {
        // scene-rows.test.js:69-76
        assert!(is_row_cell_empty(
            &scene("s", json!([{"name": "Object", "modules": {}}])),
            0,
            &[]
        ));
        assert!(!is_row_cell_empty(
            &scene("s", json!([{"name": "Object", "modules": {"m": {}}}])),
            0,
            &[]
        ));
    }

    // ————————————————— getRowGroup —————————————————

    #[test]
    fn row_group_returns_the_group_at_the_index() {
        // scene-rows.test.js:80-83
        let scene = scene("s", json!([a("A"), b("B")]));
        assert_eq!(get_row_group(&scene, 1, &[]).unwrap()["name"], json!("B"));
    }

    #[test]
    fn row_group_is_none_when_missing() {
        // scene-rows.test.js:85-90
        assert!(get_row_group(&scene("s", json!([a("A")])), 3, &[]).is_none());
        assert!(get_row_group(&json!({}), 0, &[]).is_none());
        assert!(get_row_group(&Value::Null, 0, &[]).is_none());
    }

    #[test]
    fn named_row_matches_group_case_insensitively() {
        // scene-rows.test.js:92-98
        let webcam = group("WEBCAM", json!(["cam"]));
        let simple = scene("s", json!([webcam.clone(), b("B")]));
        assert_eq!(
            get_row_group(&simple, 0, &names(&["webcam"])).unwrap()["name"],
            json!("WEBCAM")
        );

        let nested = scene("s", json!([a("A"), webcam.clone(), b("B")]));
        assert_eq!(
            get_row_group(&nested, 0, &names(&["Webcam"])).unwrap()["name"],
            json!("WEBCAM")
        );
    }

    #[test]
    fn named_row_matches_word_prefixed_group_names_only() {
        // scene-rows.test.js:100-106
        let cam2 = group("Webcam 2", json!(["cam-2"]));
        assert!(
            get_row_group(&scene("s", json!([cam2.clone()])), 0, &names(&["webcam"])).is_some()
        );
        assert!(
            get_row_group(&scene("s", json!([cam2.clone()])), 0, &names(&["webcam 2"])).is_some()
        );
        assert!(get_row_group(&scene("s", json!([cam2.clone()])), 0, &names(&["cam"])).is_none());
        assert!(
            get_row_group(&scene("s", json!([cam2])), 1, &names(&["cam", "webcam 2"])).is_some()
        );
    }

    #[test]
    fn named_row_matches_digit_suffixed_group_names_with_or_without_separator() {
        // scene-rows.test.js:108-120
        let fx = group("Effects", json!(["fx"]));
        let fx2 = group("Effects 2", json!(["fx-2"]));
        let fx_tight = group("Effects2", json!(["fx-tight"]));

        assert!(
            get_row_group(&scene("s", json!([fx.clone()])), 0, &names(&["effects2"])).is_some()
        );
        assert!(
            get_row_group(
                &scene("s", json!([fx_tight.clone()])),
                0,
                &names(&["effects"])
            )
            .is_some()
        );
        assert!(get_row_group(&scene("s", json!([fx2])), 0, &names(&["effects"])).is_some());
        assert!(get_row_group(&scene("s", json!([fx])), 0, &names(&["Effects 10"])).is_some());
    }

    #[test]
    fn solo_digit_suffixed_rows_match_the_base_group_unless_a_sibling_owns_it() {
        // scene-rows.test.js:122-138
        let fx = group("Effects", json!(["fx"]));
        let solo = scene("s", json!([fx.clone(), b("B")]));

        assert!(get_row_group(&solo, 0, &names(&["effects2"])).is_some());
        assert!(get_row_group(&solo, 1, &names(&["Effects", "effects2"])).is_none());
        assert!(get_row_group(&solo, 0, &names(&["Effects", "effects2"])).is_some());

        let fx2 = group("Effects 2", json!(["fx-2"]));
        let shared_family = scene("s", json!([fx, fx2.clone()]));
        assert_eq!(
            get_row_group(&shared_family, 1, &names(&["Effects", "Effects 2"])).unwrap()["name"],
            json!("Effects 2")
        );
    }

    #[test]
    fn digit_suffixed_names_do_not_match_across_different_base_names() {
        // scene-rows.test.js:140-148
        assert!(
            get_row_group(
                &scene("s", json!([group("Webcam 2", json!(["cam-2"]))])),
                0,
                &names(&["cam"])
            )
            .is_none()
        );
        assert!(
            get_row_group(
                &scene("s", json!([group("CAM2", json!(["cam-2"]))])),
                0,
                &names(&["cam"])
            )
            .is_some()
        );
    }

    #[test]
    fn named_row_without_a_match_is_empty_at_other_indexes() {
        // scene-rows.test.js:150-152
        let scene = scene("s", json!([a("A"), b("B")]));
        assert!(get_row_group(&scene, 0, &names(&["webcam"])).is_none());
    }

    #[test]
    fn named_row_returns_only_the_first_matching_group() {
        // scene-rows.test.js:154-158
        let first = group("Webcam", json!(["cam-1"]));
        let second = group("webcam", json!(["cam-2"]));
        let scene = scene("s", json!([first, second]));
        assert_eq!(
            get_row_group(&scene, 0, &names(&["webcam"])).unwrap()["modules"],
            json!(["cam-1"])
        );
    }

    #[test]
    fn falls_back_to_positional_lookup_without_row_names() {
        // scene-rows.test.js:160-165
        let scene = scene("s", json!([a("A"), b("B")]));
        assert_eq!(get_row_group(&scene, 1, &[]).unwrap()["name"], json!("B"));
        assert_eq!(
            get_row_group(&scene, 1, &names(&[""])).unwrap()["name"],
            json!("B")
        );
    }

    #[test]
    fn blank_row_name_is_positional() {
        // scene-rows.test.js:167-170
        let scene = scene("s", json!([a("A"), b("B")]));
        assert_eq!(
            get_row_group(&scene, 1, &names(&["", "  "])).unwrap()["name"],
            json!("B")
        );
    }

    // ————————————————— assignSceneRowIndexes —————————————————

    #[test]
    fn assign_row_indexes_keeps_positional_indexes_without_row_names() {
        // scene-rows.test.js:174-178
        assert_eq!(
            assign_scene_row_indexes(&[a("A"), b("B"), c("C")], &[]),
            vec![Some(0), Some(1), Some(2)]
        );
        assert_eq!(
            assign_scene_row_indexes(&[a("A"), b("B")], &[]),
            vec![Some(0), Some(1)]
        );
        assert_eq!(
            assign_scene_row_indexes(&[Value::Null, b("B")], &names(&["", "  "])),
            vec![Some(0), Some(1)]
        );
    }

    #[test]
    fn assign_row_indexes_pins_named_groups_to_their_rows() {
        // scene-rows.test.js:180-194
        let cam = group("WEBCAM", json!([]));
        let fg = group("Foreground", json!([]));
        let fx = group("Effects", json!([]));
        let row_names = names(&["webcam", "Background", "Foreground", "Effects", "effects2"]);

        assert_eq!(
            assign_scene_row_indexes(&[cam.clone(), fg.clone(), fx.clone()], &row_names),
            vec![Some(0), Some(2), Some(3)]
        );
        assert_eq!(
            assign_scene_row_indexes(&[fx, fg, cam], &row_names),
            vec![Some(3), Some(2), Some(0)]
        );
    }

    #[test]
    fn assign_row_indexes_slots_unmatched_groups_below_named_rows() {
        // scene-rows.test.js:196-204
        let groups = [
            group("WEBCAM", json!([])),
            group("Background", json!([])),
            group("Effects", json!([])),
        ];
        assert_eq!(
            assign_scene_row_indexes(&groups, &names(&["webcam", "Effects"])),
            vec![Some(0), Some(2), Some(1)]
        );
    }

    #[test]
    fn assign_row_indexes_matches_case_insensitively_and_by_word_prefix() {
        // scene-rows.test.js:206-211
        let cam2 = group("Webcam 2", json!([]));
        assert_eq!(
            assign_scene_row_indexes(&[cam2.clone()], &names(&["webcam"])),
            vec![Some(0)]
        );
        assert_eq!(
            assign_scene_row_indexes(&[cam2.clone()], &names(&["Webcam 2"])),
            vec![Some(0)]
        );
        assert_eq!(
            assign_scene_row_indexes(&[cam2], &names(&["cam"])),
            vec![Some(1)]
        );
    }

    #[test]
    fn assign_row_indexes_does_not_let_a_suffixed_row_claim_a_owned_group() {
        // scene-rows.test.js:213-218
        let fx = group("Effects", json!([]));
        assert_eq!(
            assign_scene_row_indexes(&[fx.clone()], &names(&["Effects", "effects2"])),
            vec![Some(0)]
        );
        assert_eq!(
            assign_scene_row_indexes(&[fx], &names(&["effects2"])),
            vec![Some(0)]
        );
    }

    #[test]
    fn assign_row_indexes_leaves_holes_null_when_named_rows_are_in_use() {
        // scene-rows.test.js:220-225
        assert_eq!(
            assign_scene_row_indexes(&[Value::Null, group("Effects", json!([]))], &[]),
            vec![Some(0), Some(1)]
        );
        assert_eq!(
            assign_scene_row_indexes(
                &[Value::Null, group("Effects", json!([]))],
                &names(&["Effects"])
            ),
            vec![None, Some(0)]
        );
    }

    // ————————————————— getRowCount —————————————————

    #[test]
    fn row_count_is_zero_without_scenes() {
        // scene-rows.test.js:230-232
        assert_eq!(get_row_count(&[], &[], &[]), 0);
    }

    #[test]
    fn row_count_uses_the_scene_with_the_most_groups() {
        // scene-rows.test.js:234-241
        let scenes = [
            scene("one", json!([a("A"), b("B")])),
            scene("two", json!([a("A"), b("B"), c("C")])),
            scene("three", json!([a("A")])),
        ];
        assert_eq!(get_row_count(&scenes, &[], &[]), 3);
    }

    #[test]
    fn row_count_ignores_the_gallery_group_in_scenes() {
        // scene-rows.test.js:243-249
        let scenes = [
            scene(
                "one",
                json!([
                    group(GALLERY_GROUP_NAME, json!(["gallery-module"])),
                    a("A"),
                    b("B")
                ]),
            ),
            scene(
                "two",
                json!([a("A"), group(GALLERY_GROUP_NAME, json!([])), b("B"), c("C")]),
            ),
        ];
        assert_eq!(get_row_count(&scenes, &[], &[]), 3);
    }

    #[test]
    fn row_count_includes_live_groups_by_scene_row() {
        // scene-rows.test.js:251-257
        let live = [
            group_with("A", json!(["a"]), json!({"sceneRow": 0})),
            group_with("D", json!(["d"]), json!({"sceneRow": 3})),
        ];
        assert_eq!(get_row_count(&[], &live, &[]), 4);
    }

    #[test]
    fn row_count_ignores_live_gallery_groups() {
        // scene-rows.test.js:259-266
        let live = [
            group_with("A", json!(["a"]), json!({"sceneRow": 0})),
            group_with(
                GALLERY_GROUP_NAME,
                json!(["gallery"]),
                json!({"sceneRow": 9}),
            ),
            group_with("B", json!(["b"]), json!({"sceneRow": 1})),
        ];
        assert_eq!(get_row_count(&[], &live, &[]), 2);
    }

    #[test]
    fn row_count_ignores_outgoing_ghost_groups() {
        // scene-rows.test.js:268-275
        let live = [
            group_with("A", json!(["a"]), json!({"sceneRow": 0})),
            group_with(
                "A ghost",
                json!(["old"]),
                json!({"sceneRow": 0, "transitionGhost": true}),
            ),
            group_with("B", json!(["b"]), json!({"sceneRow": 1})),
        ];
        assert_eq!(get_row_count(&[], &live, &[]), 2);
    }

    #[test]
    fn row_count_skips_holes_in_a_partial_live_list() {
        // scene-rows.test.js:277-284
        let live = [
            group_with("A", json!(["a"]), json!({"sceneRow": 0})),
            Value::Null,
            group_with("B", json!(["b"]), json!({"sceneRow": 2})),
        ];
        assert_eq!(get_row_count(&[], &live, &[]), 3);
    }

    #[test]
    fn row_count_combines_scene_and_live_counts() {
        // scene-rows.test.js:286-293
        let scenes = [scene("one", json!([a("A"), b("B")]))];
        let live = [group_with("A", json!(["a"]), json!({"sceneRow": 4}))];
        assert_eq!(get_row_count(&scenes, &live, &[]), 5);
    }

    #[test]
    fn row_count_is_never_negative() {
        // scene-rows.test.js:295-298
        assert_eq!(get_row_count(&[], &[], &[]), 0);
        assert_eq!(get_row_count(&[scene("one", json!([]))], &[], &[]), 0);
    }

    #[test]
    fn row_count_grows_for_named_rows() {
        // scene-rows.test.js:300-305
        assert_eq!(get_row_count(&[], &[], &names(&["webcam", ""])), 2);
        assert_eq!(
            get_row_count(
                &[scene("one", json!([a("A")]))],
                &[],
                &names(&["webcam", "fx", ""])
            ),
            3
        );
    }

    #[test]
    fn row_count_keeps_the_longer_of_named_rows_and_positional_counts() {
        // scene-rows.test.js:307-310
        assert_eq!(
            get_row_count(
                &[scene("one", json!([a("A"), b("B"), c("C")]))],
                &[],
                &names(&["x"])
            ),
            3
        );
        assert_eq!(
            get_row_count(
                &[scene("one", json!([a("A")]))],
                &[],
                &names(&["x", "y", "z"])
            ),
            3
        );
    }

    // ————————————————— normalizeRowNames —————————————————

    #[test]
    fn normalize_row_names_trims_and_keeps_positions() {
        // scene-rows.test.js:319-324
        assert_eq!(
            normalize_row_names(Some(&json!([" webcam ", "  ", "fx"]))),
            names(&["webcam", "", "fx"])
        );
    }

    #[test]
    fn normalize_row_names_keeps_duplicates() {
        // scene-rows.test.js:326-332
        assert_eq!(
            normalize_row_names(Some(&json!(["Webcam", "webcam", "FX"]))),
            names(&["Webcam", "webcam", "FX"])
        );
    }

    #[test]
    fn normalize_row_names_returns_empty_for_non_inputs() {
        // scene-rows.test.js:334-337
        assert!(normalize_row_names(None).is_empty());
        assert!(normalize_row_names(Some(&json!("nope"))).is_empty());
        assert_eq!(
            normalize_row_names(Some(&json!([null, 42]))),
            names(&["", ""])
        );
    }

    // ————————————————— getRowLabel —————————————————

    #[test]
    fn row_label_returns_the_most_common_group_name() {
        // scene-rows.test.js:341-349
        let scenes = [
            scene(
                "one",
                json!([group("Effects", json!([])), group("Blur", json!([]))]),
            ),
            scene(
                "two",
                json!([group("Effects", json!([])), group("Tunnel", json!([]))]),
            ),
            scene(
                "three",
                json!([group("Contrast", json!([])), group("Blur", json!([]))]),
            ),
        ];
        assert_eq!(get_row_label(&scenes, 0, &[]), "Effects");
        assert_eq!(get_row_label(&scenes, 1, &[]), "Blur");
    }

    #[test]
    fn row_label_falls_back_when_a_name_covers_a_minority() {
        // scene-rows.test.js:351-378
        let plain = || {
            scene(
                "main",
                json!([
                    group("Background", json!([])),
                    group("Foreground", json!([]))
                ]),
            )
        };
        let legacy = || {
            scene(
                "legacy",
                json!([
                    group("WEBCAM", json!([])),
                    group("Background", json!([])),
                    group("Foreground", json!([])),
                    group("Effects", json!([])),
                ]),
            )
        };
        let scenes = vec![
            plain(),
            plain(),
            plain(),
            plain(),
            plain(),
            legacy(),
            legacy(),
            legacy(),
        ];
        assert_eq!(get_row_label(&scenes, 3, &[]), "Row 4");
        assert_eq!(get_row_label(&scenes, 1, &[]), "Foreground");
    }

    #[test]
    fn row_label_falls_back_on_a_tie() {
        // scene-rows.test.js:380-388
        let scenes = [
            scene("one", json!([group("First", json!([]))])),
            scene("two", json!([group("Second", json!([]))])),
            scene("three", json!([group("Second", json!([]))])),
            scene("four", json!([group("First", json!([]))])),
        ];
        assert_eq!(get_row_label(&scenes, 0, &[]), "Row 1");
    }

    #[test]
    fn row_label_lets_a_strict_majority_win() {
        // scene-rows.test.js:390-397
        let scenes = [
            scene("one", json!([group("First", json!([]))])),
            scene("two", json!([group("Second", json!([]))])),
            scene("three", json!([group("Second", json!([]))])),
        ];
        assert_eq!(get_row_label(&scenes, 0, &[]), "Second");
    }

    #[test]
    fn row_label_ignores_empty_names() {
        // scene-rows.test.js:399-406
        let scenes = [
            scene("one", json!([group("", json!([]))])),
            scene("two", json!([group("Named", json!([]))])),
            scene("three", json!([group("Named", json!([]))])),
        ];
        assert_eq!(get_row_label(&scenes, 0, &[]), "Named");
    }

    #[test]
    fn row_label_falls_back_to_a_numbered_row() {
        // scene-rows.test.js:408-414
        assert_eq!(get_row_label(&[], 0, &[]), "Row 1");
        assert_eq!(get_row_label(&[scene("one", json!([]))], 2, &[]), "Row 3");
        assert_eq!(
            get_row_label(
                &[
                    scene("one", json!([group("", json!([]))])),
                    scene("two", json!([]))
                ],
                0,
                &[]
            ),
            "Row 1"
        );
    }

    #[test]
    fn row_label_uses_the_explicit_row_name() {
        // scene-rows.test.js:416-420
        let scenes = [scene(
            "one",
            json!([group("Effects", json!([])), group("Blur", json!([]))]),
        )];
        assert_eq!(get_row_label(&scenes, 0, &names(&["webcam"])), "webcam");
        assert_eq!(
            get_row_label(&scenes, 1, &names(&["", "Fog layer"])),
            "Fog layer"
        );
    }

    // ————————————————— pickNextRowScene —————————————————

    fn pick(scenes: &[Value], row_index: usize, current: Option<&str>) -> Option<Value> {
        pick_next_row_scene(
            &PickNextRowScene {
                scenes,
                row_index,
                current_scene_id: current,
                order: "in-order",
                row_names: &[],
            },
            || 0.0,
        )
        .cloned()
    }

    #[test]
    fn pick_next_row_scene_returns_none_without_candidates() {
        // scene-rows.test.js:426-434
        let scenes = [scene("a", json!([empty("Empty")])), scene("b", json!([]))];
        assert!(pick(&scenes, 0, Some("a")).is_none());
    }

    #[test]
    fn pick_next_row_scene_returns_the_only_candidate() {
        // scene-rows.test.js:436-445
        let scenes = [scene("a", json!([a("A")])), scene("b", json!([]))];
        assert_eq!(pick(&scenes, 0, Some("b")).unwrap()["id"], json!("a"));
    }

    #[test]
    fn pick_next_row_scene_picks_the_candidate_after_the_current_scene() {
        // scene-rows.test.js:447-451
        let scenes = [
            scene("a", json!([a("A")])),
            scene("b", json!([b("B")])),
            scene("c", json!([c("C")])),
        ];
        assert_eq!(pick(&scenes, 0, Some("a")).unwrap()["id"], json!("b"));
    }

    #[test]
    fn pick_next_row_scene_wraps_around_at_the_end() {
        // scene-rows.test.js:453-457
        let scenes = [
            scene("a", json!([a("A")])),
            scene("b", json!([b("B")])),
            scene("c", json!([c("C")])),
        ];
        assert_eq!(pick(&scenes, 0, Some("c")).unwrap()["id"], json!("a"));
    }

    #[test]
    fn pick_next_row_scene_picks_the_first_candidate_when_current_is_not_one() {
        // scene-rows.test.js:459-480
        let scenes = [
            scene("a", json!([empty("Empty")])),
            scene("b", json!([b("B")])),
            scene("c", json!([empty("Empty")])),
            scene("d", json!([group("D", json!([]))])),
        ];
        assert_eq!(pick(&scenes, 0, Some("a")).unwrap()["id"], json!("b"));
        assert_eq!(pick(&scenes, 0, Some("missing")).unwrap()["id"], json!("b"));
    }

    #[test]
    fn pick_next_row_scene_excludes_disabled_scenes() {
        // scene-rows.test.js:482-495
        let scenes = [
            scene("a", json!([a("A")])),
            json!({"id": "b", "name": "b", "groups": [b("B")], "enabled": false}),
            scene("c", json!([c("C")])),
        ];
        assert_eq!(pick(&scenes, 0, Some("a")).unwrap()["id"], json!("c"));
    }

    #[test]
    fn pick_next_row_scene_skips_empty_cells() {
        // scene-rows.test.js:497-510
        let scenes = [
            scene("a", json!([a("A"), empty("Empty")])),
            scene("b", json!([empty("Empty"), b("B")])),
            scene("c", json!([c("C"), c("C")])),
        ];
        assert_eq!(pick(&scenes, 1, Some("a")).unwrap()["id"], json!("b"));
    }

    #[test]
    fn pick_next_row_scene_honours_named_rows() {
        // scene-rows.test.js:512-530
        let scenes = [
            scene("a", json!([group("A", json!(["m-a"]))])),
            scene("b", json!([group("B", json!(["m-b"]))])),
            scene("c", json!([group("C", json!(["m-c"]))])),
        ];
        let row_names = names(&["B"]);
        let picked = pick_next_row_scene(
            &PickNextRowScene {
                scenes: &scenes,
                row_index: 0,
                current_scene_id: Some("a"),
                order: "in-order",
                row_names: &row_names,
            },
            || 0.0,
        );
        assert_eq!(picked.unwrap()["id"], json!("b"));
    }

    #[test]
    fn pick_next_row_scene_picks_randomly_and_excludes_the_current_scene() {
        // scene-rows.test.js:532-554
        let scenes = [
            scene("a", json!([a("A")])),
            scene("b", json!([b("B")])),
            scene("c", json!([c("C")])),
        ];

        let mut calls = 0;
        let picked = pick_next_row_scene(
            &PickNextRowScene {
                scenes: &scenes,
                row_index: 0,
                current_scene_id: Some("b"),
                order: "random",
                row_names: &[],
            },
            || {
                calls += 1;
                0.0
            },
        );
        assert_eq!(picked.unwrap()["id"], json!("a"));
        assert_eq!(calls, 1);

        let picked = pick_next_row_scene(
            &PickNextRowScene {
                scenes: &scenes,
                row_index: 0,
                current_scene_id: Some("b"),
                order: "random",
                row_names: &[],
            },
            || 0.99,
        );
        assert_eq!(picked.unwrap()["id"], json!("c"));
    }

    #[test]
    fn pick_next_row_scene_can_still_pick_the_only_candidate_without_rolling() {
        // scene-rows.test.js:556-569
        let scenes = [scene("a", json!([a("A")])), scene("b", json!([]))];
        let mut calls = 0;
        let picked = pick_next_row_scene(
            &PickNextRowScene {
                scenes: &scenes,
                row_index: 0,
                current_scene_id: Some("a"),
                order: "random",
                row_names: &[],
            },
            || {
                calls += 1;
                0.0
            },
        );
        assert_eq!(picked.unwrap()["id"], json!("a"));
        assert_eq!(calls, 0);
    }

    #[test]
    fn pick_next_row_scene_does_not_exclude_a_non_candidate_current_scene() {
        // scene-rows.test.js:571-582
        let scenes = [scene("a", json!([a("A")])), scene("b", json!([b("B")]))];
        let picked = pick_next_row_scene(
            &PickNextRowScene {
                scenes: &scenes,
                row_index: 0,
                current_scene_id: Some("missing"),
                order: "random",
                row_names: &[],
            },
            || 0.0,
        );
        assert_eq!(picked.unwrap()["id"], json!("a"));
    }

    // ————————————————— shouldAdvanceRow —————————————————

    fn should(
        dwell: f64,
        min: f64,
        max: f64,
        target: Option<f64>,
        phrase: bool,
        roll: f64,
    ) -> RowAdvance {
        should_advance_row(
            &ShouldAdvanceRow {
                dwell,
                min_seconds: Some(min),
                max_seconds: Some(max),
                target_seconds: target,
                phrase_flipped: phrase,
            },
            || roll,
        )
    }

    #[test]
    fn advance_is_forced_at_the_maximum_dwell() {
        // scene-rows.test.js:587-595
        assert_eq!(
            should(10.0, 5.0, 10.0, None, false, 0.0),
            RowAdvance {
                advance: true,
                musical: false,
                forced: true,
            }
        );
    }

    #[test]
    fn advance_is_forced_past_the_maximum_dwell() {
        // scene-rows.test.js:597-606
        assert_eq!(
            should(12.0, 5.0, 10.0, None, false, 0.0),
            RowAdvance {
                advance: true,
                musical: false,
                forced: true,
            }
        );
    }

    #[test]
    fn does_not_advance_while_dwelling_below_the_minimum() {
        // scene-rows.test.js:608-620
        let mut calls = 0;
        let decision = should_advance_row(
            &ShouldAdvanceRow {
                dwell: 2.0,
                min_seconds: Some(5.0),
                max_seconds: Some(10.0),
                target_seconds: None,
                phrase_flipped: true,
            },
            || {
                calls += 1;
                0.0
            },
        );
        assert_eq!(
            decision,
            RowAdvance {
                advance: false,
                musical: false,
                forced: false,
            }
        );
        assert_eq!(calls, 1);
    }

    #[test]
    fn advances_musically_when_the_phrase_flips_and_the_roll_succeeds() {
        // scene-rows.test.js:622-632
        assert_eq!(
            should(7.5, 5.0, 10.0, None, true, 0.49),
            RowAdvance {
                advance: true,
                musical: true,
                forced: false,
            }
        );
    }

    #[test]
    fn does_not_advance_musically_when_the_roll_fails() {
        // scene-rows.test.js:634-644
        assert_eq!(
            should(7.5, 5.0, 10.0, None, true, 0.5),
            RowAdvance {
                advance: false,
                musical: false,
                forced: false,
            }
        );
    }

    #[test]
    fn does_not_roll_without_a_phrase_flip() {
        // scene-rows.test.js:646-658
        let mut calls = 0;
        let decision = should_advance_row(
            &ShouldAdvanceRow {
                dwell: 7.5,
                min_seconds: Some(5.0),
                max_seconds: Some(10.0),
                target_seconds: None,
                phrase_flipped: false,
            },
            || {
                calls += 1;
                0.0
            },
        );
        assert_eq!(
            decision,
            RowAdvance {
                advance: false,
                musical: false,
                forced: false,
            }
        );
        assert_eq!(calls, 0);
    }

    #[test]
    fn maximum_dwell_is_forced_even_on_a_phrase_flip() {
        // scene-rows.test.js:660-670
        assert_eq!(
            should(10.0, 5.0, 10.0, None, true, 1.0),
            RowAdvance {
                advance: true,
                musical: false,
                forced: true,
            }
        );
    }

    #[test]
    fn forces_at_the_per_cell_target_before_the_maximum() {
        // scene-rows.test.js:672-682
        assert_eq!(should(6.0, 5.0, 10.0, Some(6.0), false, 0.0).forced, true);
    }

    #[test]
    fn does_not_force_before_the_per_cell_target() {
        // scene-rows.test.js:684-694
        assert!(!should(5.9, 5.0, 10.0, Some(6.0), false, 0.0).forced);
    }

    #[test]
    fn clamps_a_per_cell_target_above_the_maximum() {
        // scene-rows.test.js:696-715
        assert!(!should(9.5, 5.0, 10.0, Some(20.0), false, 0.0).forced);
        assert!(should(10.0, 5.0, 10.0, Some(20.0), false, 0.0).forced);
    }

    #[test]
    fn clamps_a_per_cell_target_below_the_minimum() {
        // scene-rows.test.js:717-736
        assert!(!should(4.9, 5.0, 10.0, Some(1.0), false, 0.0).forced);
        assert!(should(5.0, 5.0, 10.0, Some(1.0), false, 0.0).forced);
    }

    #[test]
    fn missing_per_cell_target_means_the_maximum() {
        // scene-rows.test.js:738-757
        assert!(!should(9.0, 5.0, 10.0, None, false, 0.0).forced);
        assert!(should(10.0, 5.0, 10.0, None, false, 0.0).forced);
    }

    // ————————————————— resolveRowAdvanceScene —————————————————

    fn resolve(
        scenes: &[Value],
        row_index: usize,
        current: Option<&str>,
        pending: Option<&str>,
    ) -> Option<Value> {
        resolve_row_advance_scene(
            &ResolveRowAdvanceScene {
                scenes,
                row_index,
                current_scene_id: current,
                pending_scene_id: pending,
                order: "in-order",
                row_names: &[],
            },
            || 0.0,
        )
        .cloned()
    }

    #[test]
    fn resolve_row_advance_uses_the_pending_scene_when_valid() {
        // scene-rows.test.js:763-774
        let scenes = [
            scene("a", json!([a("A")])),
            scene("b", json!([b("B")])),
            scene("c", json!([c("C")])),
        ];
        assert_eq!(
            resolve(&scenes, 0, Some("a"), Some("c")).unwrap()["id"],
            json!("c")
        );
    }

    #[test]
    fn resolve_row_advance_falls_back_when_pending_is_gone() {
        // scene-rows.test.js:776-784
        let scenes = [
            scene("a", json!([a("A")])),
            scene("b", json!([b("B")])),
            scene("c", json!([c("C")])),
        ];
        assert_eq!(
            resolve(&scenes, 0, Some("a"), Some("missing")).unwrap()["id"],
            json!("b")
        );
    }

    #[test]
    fn resolve_row_advance_falls_back_when_pending_is_disabled() {
        // scene-rows.test.js:787-802
        let scenes = [
            scene("a", json!([a("A")])),
            json!({"id": "b", "name": "b", "groups": [b("B")], "enabled": false}),
            scene("c", json!([c("C")])),
        ];
        assert_eq!(
            resolve(&scenes, 0, Some("a"), Some("b")).unwrap()["id"],
            json!("c")
        );
    }

    #[test]
    fn resolve_row_advance_falls_back_when_pending_row_is_empty() {
        // scene-rows.test.js:804-819
        let scenes = [
            scene("a", json!([a("A"), a("A")])),
            scene("b", json!([b("B"), empty("Empty")])),
            scene("c", json!([c("C"), c("C")])),
        ];
        assert_eq!(
            resolve(&scenes, 1, Some("a"), Some("b")).unwrap()["id"],
            json!("c")
        );
    }

    #[test]
    fn resolve_row_advance_picks_normally_without_a_prewarmed_scene() {
        // scene-rows.test.js:821-829
        let scenes = [
            scene("a", json!([a("A")])),
            scene("b", json!([b("B")])),
            scene("c", json!([c("C")])),
        ];
        assert_eq!(
            resolve(&scenes, 0, Some("b"), None).unwrap()["id"],
            json!("c")
        );
    }

    // ————————————————— helpers —————————————————

    #[test]
    fn base_name_strips_a_trailing_number_and_one_separator() {
        // scene-rows.js:38-42
        assert_eq!(base_name("Effects"), "effects");
        assert_eq!(base_name("Effects 2"), "effects");
        assert_eq!(base_name("effects2"), "effects");
        // The JS regex ` ?\d+$` only eats one separator space, so the extra
        // padding spaces survive.
        assert_eq!(base_name("  EFFECTS   10 "), "effects  ");
        assert_eq!(base_name("2d Scenes"), "2d scenes");
        assert_eq!(base_name("cam2"), "cam");
        assert_eq!(base_name(""), "");
    }

    #[test]
    fn switch_chance_ramps_from_zero_to_one() {
        // phrase-detector.js:87-96
        assert_eq!(switch_chance(4.0, 5.0, 10.0), 0.0);
        assert_eq!(switch_chance(7.5, 5.0, 10.0), 0.5);
        assert_eq!(switch_chance(12.0, 5.0, 10.0), 1.0);
        assert_eq!(switch_chance(4.0, 5.0, 5.0), 1.0);
    }

    #[test]
    fn row_family_is_shared_detects_sibling_rows() {
        // scene-rows.js:88-113
        assert!(!row_family_is_shared(0, &names(&["effects2"])));
        assert!(row_family_is_shared(1, &names(&["Effects", "effects2"])));
        assert!(row_family_is_shared(0, &names(&["Effects", "effects2"])));
        assert!(!row_family_is_shared(0, &names(&["", "effects2"])));
        assert!(!row_family_is_shared(1, &[]));
    }
}
