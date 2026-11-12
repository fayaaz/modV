//! The preset-load SWAP engine, ported from
//! `store/modules/common/swap.js`.
//!
//! Loading a preset writes into a swap buffer, then each store module commits
//! `SWAP` to fold the buffer into the live state — keeping only the
//! properties marked "allowed" (`sharedPropertyRestrictions`). This module
//! ports the two passes exactly as the JS performs them.
//!
//! ## JS semantics (empirically verified against `node:24`)
//!
//! **Pass 1 (`state` → `swap`)**, for each key in `state`, with
//! `sharedPropertyRestrictions` *present* (the `groups`/`modules` case):
//!
//! | restriction | effect |
//! |---|---|
//! | `false` | untouched (`modules.registered`) |
//! | `true` | `swap[K] = state[K]; delete state[K]` (**overwrites** whatever the preset wrote into `swap[K]` — `modules.propQueue`/`metaQueue`) |
//! | `fn` on an array | predicate-matching items **stay** in `state`, the rest are appended to `swap`; `swapIndexes[K] = swap[K].len()` recorded *before* the append (`groups`: gallery group stays) |
//! | `fn` on an object | `fn(state[K])` returns the list of child keys to **keep** in `state`; the rest move to `swap` (`modules.active`: gallery modules stay) |
//! | key absent from a present restrictions object | **untouched** (the JS `else if` chain falls through silently) |
//!
//! With **no restrictions argument at all** (the `inputs`/`expressions`
//! case), pass 1 deletes the whole `state[K]`; pass 2 re-creates it from the
//! swap buffer only.
//!
//! **Pass 2 (`swap` → `state`)**:
//!
//! | restriction | effect |
//! |---|---|
//! | `false` or absent | untouched |
//! | `true` | `state[K] = clone(swap[K])` |
//! | `fn` on an array | `state[K] = [...state[K], ...swap[K][..swapIndexes[K]]]`; `swap[K]` truncated to the remainder — i.e. only the items that were **already in the swap buffer before pass 1** (the preset's) move into state; the items pass 1 moved out of state stay in `swap` (`groups`: old live groups end in `swap`, where `cleanUpGroupResources({readFromSwap: true})` destroys them) |
//! | `fn` on an object | children of `swap[K]` **not** in `fn(swap[K])` are copied into `state[K]` — note this copies the pass-1 movers *back* into `state`, including old non-gallery modules (`modules.active`); the gallery keys that are in `fn(swap[K])` stay in `swap` |
//! | no restrictions | `state[K] = merge(state[K], swap[K])` — swap wins per key; arrays concat |
//!
//! Because pass 2 object restrictions **copy back** rather than move,
//! after a preset load `state.active` legitimately contains: the kept
//! gallery modules, the preset's modules, *and* the previously live
//! non-gallery modules (unreachable from the restored groups, which now
//! reference the preset's module ids). The JS removes them via
//! `removeActiveModule({ writeToSwap: true })`, which deletes only from the
//! swap buffer — so stale ids linger in `state.active` (pre-existing modV
//! behaviour, reproduced here faithfully; see WS-13 "SWAP semantics").
//!
//! `setDefault` (JS 5th argument of the factory, **default `true`**): after
//! both passes, `Object.assign(swap, getDefault())` — each default key is
//! reset to a fresh empty value; extra swap keys survive. The JS module
//! calls: `groups` `SWAP(swap, getDefaultState, restrictions, false)` —
//! no reset; `modules` — also `false`; **`inputs` — defaults to `true`**
//! (`inputs.js:268`: `SWAP(swap, getDefaultState)`); `expressions` —
//! also defaults to `true`. (The note in the M2a task brief that inputs
//! uses `setDefault=false` is wrong; the verified source wins.)
//!
//! ## Representation
//!
//! The store state of each module is represented as an
//! `IndexMap<String, Value>` keyed by state key (`"groups"` | `"registered"` |
//! `"active"` | `"propQueue"` | `"metaQueue"` | `"focusedInput"` | `"inputs"` |
//! `"inputLinks"` | `"assignments"`). `groups` is an array; the rest are
//! objects. M2b composes [`swap`] once per store module in the JS call order
//! — groups, modules, inputs, expressions (`SWAP_MODULES`,
//! `preset-serializer.js:1`).
//!
//! ## Robustness deviations from the JS (JS would crash, Rust tolerates)
//!
//! - A restrictor applied to a non-array/non-object value leaves the value
//!   untouched in pass 1 (JS throws on `.filter`/`Object.keys`).
//! - A missing swap counterpart for an array restriction initialises an
//!   empty array (JS throws on `.length`).
//! - Pass 2 treats a missing `state[K]` as an empty array/object (JS throws
//!   on spreading `undefined` for arrays).
//! - A missing `swapIndexes[K]` in pass 2 copies the whole array into state
//!   (JS `slice(0, undefined)` semantics).

use indexmap::IndexMap;
use serde_json::{Map, Value};

use crate::engine_state::GALLERY_GROUP_NAME;

/// Predicate keeping array items in `state` during an `ArrayKeep` swap
/// (`groups`: the gallery group).
pub type ArrayKeepPredicate = fn(&Value) -> bool;

/// Predicate returning the object child keys to keep in `state` during an
/// `ObjectKeep` swap (`modules.active`: gallery modules by `$id`).
pub type ObjectKeepPredicate = fn(&Map<String, Value>) -> Vec<String>;

/// A per-key restriction for [`SwapPolicy`].
#[derive(Clone, Copy)]
pub enum Restriction {
    /// JS `false`: the key never moves.
    Keep,
    /// JS `true`: the whole key moves state→swap in pass 1 and back in
    /// pass 2 (cloned).
    Move,
    /// JS function on an array: items for which the predicate returns
    /// `true` stay in `state`; the rest are appended to `swap`.
    ArrayKeep(ArrayKeepPredicate),
    /// JS function on an object: children whose keys are returned by the
    /// predicate stay in `state`; the rest move to `swap`.
    ObjectKeep(ObjectKeepPredicate),
}

/// The SWAP policy of one store module: which keys are restricted, whether
/// the swap buffer is reset to defaults afterwards, and the default shapes
/// used for the reset (`getDefaultState`).
#[derive(Clone)]
pub struct SwapPolicy {
    /// `None` mirrors calling `SWAP(swap, getDefault)` with **no**
    /// restrictions argument at all (`inputs`, `expressions`): pass 1 deletes
    /// each `state[K]` outright, and the default reset applies.
    ///
    /// `Some(map)` mirrors a `sharedPropertyRestrictions` object: keys absent
    /// from the map are left untouched in both passes.
    pub restrictions: Option<IndexMap<String, Restriction>>,
    /// JS `setDefault` (default `true`): after both passes, reset each key in
    /// `defaults` to a fresh empty value on the swap buffer.
    pub reset_swap: bool,
    /// `getDefaultState()` shapes, applied only when `reset_swap` is set.
    pub defaults: IndexMap<String, Value>,
}

fn is_gallery_group(value: &Value) -> bool {
    value.get("name").and_then(Value::as_str) == Some(GALLERY_GROUP_NAME)
}

fn gallery_module_keys(map: &Map<String, Value>) -> Vec<String> {
    map.values()
        .filter(|module| {
            module
                .get("meta")
                .and_then(|meta| meta.get("isGallery"))
                .and_then(Value::as_bool)
                == Some(true)
        })
        .filter_map(|module| {
            module
                .get("$id")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect()
}

fn object(pairs: &[(&str, Value)]) -> IndexMap<String, Value> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), v.clone()))
        .collect()
}

impl SwapPolicy {
    /// `groups` policy: the Gallery group stays in `state`; everything else
    /// is swapped. No swap reset (`SWAP(swap, getDefaultState, restrictions,
    /// false)` — `groups.js:570`).
    pub fn for_groups() -> Self {
        let mut restrictions = IndexMap::new();
        restrictions.insert(
            "groups".to_string(),
            Restriction::ArrayKeep(is_gallery_group),
        );

        Self {
            restrictions: Some(restrictions),
            reset_swap: false,
            defaults: object(&[("groups", Value::Array(Vec::new()))]),
        }
    }

    /// `modules` policy: `registered` never moves; gallery modules stay in
    /// `state`; `propQueue`/`metaQueue` move wholesale. No swap reset
    /// (`modules.js:878`).
    pub fn for_modules() -> Self {
        let mut restrictions = IndexMap::new();
        restrictions.insert("registered".to_string(), Restriction::Keep);
        restrictions.insert(
            "active".to_string(),
            Restriction::ObjectKeep(gallery_module_keys),
        );
        restrictions.insert("propQueue".to_string(), Restriction::Move);
        restrictions.insert("metaQueue".to_string(), Restriction::Move);

        Self {
            restrictions: Some(restrictions),
            reset_swap: false,
            defaults: object(&[
                ("registered", Value::Object(Map::new())),
                ("active", Value::Object(Map::new())),
                ("propQueue", Value::Object(Map::new())),
                ("metaQueue", Value::Object(Map::new())),
            ]),
        }
    }

    /// `inputs` policy: no restrictions argument (`inputs.js:268` —
    /// `SWAP(swap, getDefaultState)`), swap buffer **is** reset to defaults.
    pub fn for_inputs() -> Self {
        Self {
            restrictions: None,
            reset_swap: true,
            defaults: object(&[
                (
                    "focusedInput",
                    Value::Object(
                        [
                            ("id".to_string(), Value::Null),
                            ("title".to_string(), Value::Null),
                        ]
                        .into_iter()
                        .collect(),
                    ),
                ),
                ("inputs", Value::Object(Map::new())),
                ("inputLinks", Value::Object(Map::new())),
            ]),
        }
    }

    /// `expressions` policy: no restrictions argument (`expressions.js:167`
    /// — `SWAP(swap, getDefaultState)`), swap buffer **is** reset to defaults.
    pub fn for_expressions() -> Self {
        Self {
            restrictions: None,
            reset_swap: true,
            defaults: object(&[("assignments", Value::Object(Map::new()))]),
        }
    }
}

fn ensure_array(value: &mut Value) -> Option<&mut Vec<Value>> {
    if !value.is_array() {
        *value = Value::Array(Vec::new());
    }
    value.as_array_mut()
}

fn ensure_object(value: &mut Value) -> Option<&mut Map<String, Value>> {
    if !value.is_object() {
        *value = Value::Object(Map::new());
    }
    value.as_object_mut()
}

/// Perform a SWAP between a live state map and a preset-loaded swap buffer.
/// Port of the returned function of the `SWAP(...)` factory in
/// `common/swap.js:14-103`.
pub fn swap(
    state: &mut IndexMap<String, Value>,
    swap_buf: &mut IndexMap<String, Value>,
    policy: &SwapPolicy,
) {
    let mut swap_indexes: IndexMap<String, usize> = IndexMap::new();

    // ——— Pass 1: state → swap ———
    for key in state.keys().cloned().collect::<Vec<_>>() {
        match &policy.restrictions {
            // JS: `if (sharedPropertyRestrictions) { ... } else { delete state[key]; }`
            None => {
                state.shift_remove(&key);
            }
            Some(restrictions) => match restrictions.get(&key) {
                // JS: `else if (sharedPropertyRestrictions[key])` with a falsy value — a present
                // restrictions object with this key missing/false means "untouched".
                Some(Restriction::Keep) | None => {}
                Some(Restriction::Move) => {
                    if let Some(value) = state.shift_remove(&key) {
                        swap_buf.insert(key, value);
                    }
                }
                Some(Restriction::ArrayKeep(keep)) => {
                    if !matches!(state.get(&key), Some(Value::Array(_))) {
                        continue; // tolerate a non-array state value
                    }
                    if let Some(existing) = swap_buf.get(&key) {
                        if !existing.is_array() {
                            continue; // tolerate a non-array swap value
                        }
                    }

                    let Some(Value::Array(items)) = state.shift_remove(&key) else {
                        continue;
                    };

                    let entry = swap_buf
                        .entry(key.clone())
                        .or_insert_with(|| Value::Array(Vec::new()));
                    let Some(swap_array) = ensure_array(entry) else {
                        continue;
                    };

                    swap_indexes.insert(key.clone(), swap_array.len());

                    let (keep_items, moved): (Vec<Value>, Vec<Value>) =
                        items.into_iter().partition(keep);
                    swap_array.extend(moved);
                    state.insert(key, Value::Array(keep_items));
                }
                Some(Restriction::ObjectKeep(keep)) => {
                    if !matches!(state.get(&key), Some(Value::Object(_))) {
                        continue; // tolerate a non-object state value
                    }
                    if let Some(existing) = swap_buf.get(&key) {
                        if !existing.is_object() {
                            continue; // tolerate a non-object swap value
                        }
                    }

                    let Some(Value::Object(mut obj)) = state.shift_remove(&key) else {
                        continue;
                    };

                    let keep_keys = keep(&obj);
                    let entry = swap_buf
                        .entry(key.clone())
                        .or_insert_with(|| Value::Object(Map::new()));
                    if let Some(swap_object) = ensure_object(entry) {
                        let child_keys: Vec<String> = obj.keys().cloned().collect();
                        for child in child_keys {
                            if keep_keys.iter().any(|k| k == &child) {
                                continue;
                            }
                            if let Some(value) = obj.remove(&child) {
                                swap_object.insert(child, value);
                            }
                        }
                    }

                    state.insert(key, Value::Object(obj));
                }
            },
        }
    }

    // ——— Pass 2: swap → state ———
    for key in swap_buf.keys().cloned().collect::<Vec<_>>() {
        match &policy.restrictions {
            // JS: state[key] = isArray ? [...state[key], ...swap[key]] : { ...state[key], ...swap[key] }
            None => {
                let swap_value = match swap_buf.get(&key) {
                    Some(value) => value.clone(),
                    None => continue,
                };
                let merged = match swap_value {
                    Value::Array(swap_items) => {
                        let out = match state.shift_remove(&key) {
                            Some(Value::Array(mut existing)) => {
                                existing.extend(swap_items);
                                existing
                            }
                            _ => swap_items,
                        };
                        Value::Array(out)
                    }
                    Value::Object(swap_object) => {
                        let mut out = match state.shift_remove(&key) {
                            Some(Value::Object(existing)) => existing,
                            _ => Map::new(),
                        };
                        out.extend(swap_object);
                        Value::Object(out)
                    }
                    other => other,
                };
                state.insert(key, merged);
            }
            Some(restrictions) => match restrictions.get(&key) {
                Some(Restriction::Keep) | None => {}
                Some(Restriction::Move) => {
                    if let Some(value) = swap_buf.get(&key).cloned() {
                        state.insert(key, value);
                    }
                }
                Some(Restriction::ArrayKeep(_)) => {
                    let swap_array = match swap_buf.get(&key).and_then(Value::as_array) {
                        Some(array) => array.clone(),
                        None => continue, // tolerate a non-array swap value
                    };
                    if let Some(existing) = state.get(&key) {
                        if !existing.is_array() {
                            continue; // tolerate a non-array state value
                        }
                    }
                    // JS: swap[key].slice(0, swapIndexes[key]) — a missing index
                    // means slice(0, undefined) = the whole array.
                    let swap_index = swap_indexes
                        .get(&key)
                        .copied()
                        .unwrap_or(swap_array.len())
                        .min(swap_array.len());
                    let to_state = swap_array[..swap_index].to_vec();
                    let remainder = swap_array[swap_index..].to_vec();

                    let mut out = match state.shift_remove(&key) {
                        Some(Value::Array(existing)) => existing,
                        _ => Vec::new(),
                    };
                    out.extend(to_state);
                    state.insert(key.clone(), Value::Array(out));
                    swap_buf.insert(key, Value::Array(remainder));
                }
                Some(Restriction::ObjectKeep(keep)) => {
                    if let Some(existing) = state.get(&key) {
                        if !existing.is_object() {
                            continue; // tolerate a non-object state value
                        }
                    }
                    let swap_object = match swap_buf.get(&key) {
                        Some(Value::Object(obj)) => obj.clone(),
                        _ => continue,
                    };
                    let keep_keys = keep(&swap_object);

                    let mut out = match state.shift_remove(&key) {
                        Some(Value::Object(existing)) => existing,
                        _ => Map::new(),
                    };
                    for (child, value) in swap_object {
                        if keep_keys.iter().any(|k| k == &child) {
                            continue;
                        }
                        out.insert(child, value);
                    }
                    state.insert(key, Value::Object(out));
                }
            },
        }
    }

    // ——— setDefault ———
    if policy.reset_swap {
        for (key, value) in policy.defaults.iter() {
            swap_buf.insert(key.clone(), value.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn gallery_group() -> Value {
        json!({"id": "gal", "name": GALLERY_GROUP_NAME, "modules": ["mg"]})
    }

    fn group(id: &str, name: &str) -> Value {
        json!({"id": id, "name": name, "modules": []})
    }

    fn map_of(pairs: &[(&str, Value)]) -> IndexMap<String, Value> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    fn group_ids(map: &IndexMap<String, Value>, key: &str) -> Vec<String> {
        map[key]
            .as_array()
            .expect("groups must be an array")
            .iter()
            .map(|g| g["id"].as_str().expect("group id").to_string())
            .collect()
    }

    #[test]
    fn groups_preset_load_preserves_gallery_group_first() {
        // Preset load wrote its groups into the swap buffer first (createGroup
        // with writeToSwap), then SWAP folds them in.
        let mut state = map_of(&[(
            "groups",
            Value::Array(vec![
                gallery_group(),
                group("old1", "Old 1"),
                group("old2", "Old 2"),
            ]),
        )]);
        let mut swap_buf = map_of(&[("groups", Value::Array(vec![group("preset-a", "Preset A")]))]);

        swap(&mut state, &mut swap_buf, &SwapPolicy::for_groups());

        // Gallery stays in place at the front; the preset's groups are appended.
        assert_eq!(group_ids(&state, "groups"), vec!["gal", "preset-a"]);
        // The old live groups end up in the swap buffer (destroyed later by
        // cleanUpGroupResources({ readFromSwap: true })).
        assert_eq!(group_ids(&swap_buf, "groups"), vec!["old1", "old2"]);
    }

    #[test]
    fn groups_swap_without_gallery_moves_everything() {
        let mut state = map_of(&[("groups", Value::Array(vec![group("old1", "Old 1")]))]);
        let mut swap_buf = map_of(&[("groups", Value::Array(vec![group("preset-a", "Preset A")]))]);

        swap(&mut state, &mut swap_buf, &SwapPolicy::for_groups());

        assert_eq!(group_ids(&state, "groups"), vec!["preset-a"]);
        assert_eq!(group_ids(&swap_buf, "groups"), vec!["old1"]);
    }

    #[test]
    fn groups_swap_empty_live_state() {
        let mut state = map_of(&[("groups", Value::Array(vec![]))]);
        let mut swap_buf = map_of(&[("groups", Value::Array(vec![group("preset-a", "Preset A")]))]);

        swap(&mut state, &mut swap_buf, &SwapPolicy::for_groups());

        assert_eq!(group_ids(&state, "groups"), vec!["preset-a"]);
        assert_eq!(group_ids(&swap_buf, "groups"), Vec::<String>::new());
    }

    #[test]
    fn groups_swap_without_preset_groups() {
        let mut state = map_of(&[(
            "groups",
            Value::Array(vec![gallery_group(), group("old1", "Old 1")]),
        )]);
        let mut swap_buf = map_of(&[("groups", Value::Array(vec![]))]);

        swap(&mut state, &mut swap_buf, &SwapPolicy::for_groups());

        assert_eq!(group_ids(&state, "groups"), vec!["gal"]);
        assert_eq!(group_ids(&swap_buf, "groups"), vec!["old1"]);
    }

    fn module(id: &str, is_gallery: bool) -> Value {
        json!({"$id": id, "meta": {"isGallery": is_gallery, "name": id}})
    }

    #[test]
    fn modules_swap_keeps_registered_untouched() {
        let mut state = map_of(&[
            ("registered", json!({"Bands": {"meta": {"name": "Bands"}}})),
            ("active", Value::Object(Map::new())),
            ("propQueue", Value::Object(Map::new())),
            ("metaQueue", Value::Object(Map::new())),
        ]);
        let mut swap_buf = map_of(&[
            ("registered", Value::Object(Map::new())),
            ("active", json!({"n1": module("n1", false)})),
            ("propQueue", Value::Object(Map::new())),
            ("metaQueue", Value::Object(Map::new())),
        ]);

        swap(&mut state, &mut swap_buf, &SwapPolicy::for_modules());

        // registered:false — never moves either way.
        assert_eq!(
            state["registered"],
            json!({"Bands": {"meta": {"name": "Bands"}}})
        );
        assert_eq!(swap_buf["registered"], json!({}));
    }

    #[test]
    fn modules_swap_copies_preset_and_old_active_modules_back_into_state() {
        // JS-exact behaviour, verified against the original implementation:
        // pass 1 moves old non-gallery modules into the swap buffer, pass 2
        // copies non-restricted swap children *back* into state — including
        // the old ones (they linger in state.active, unreachable from the
        // restored groups). Only gallery modules found in the swap buffer
        // stay there.
        let mut state = map_of(&[
            ("registered", Value::Object(Map::new())),
            (
                "active",
                json!({
                    "g1": module("g1", true),
                    "m1": module("m1", false),
                    "m2": module("m2", false),
                }),
            ),
            ("propQueue", json!({"m1": {"x": 1}})),
            ("metaQueue", Value::Object(Map::new())),
        ]);
        let mut swap_buf = map_of(&[
            ("registered", Value::Object(Map::new())),
            (
                "active",
                json!({
                    "n1": module("n1", false),
                    "n2": module("n2", true), // preset-loaded gallery module stays in swap
                }),
            ),
            ("propQueue", json!({"n1": {"y": 2}})), // overwritten by pass 1 (JS behaviour)
            ("metaQueue", json!({"n1": {"z": 3}})), // overwritten by pass 1 (JS behaviour)
        ]);

        swap(&mut state, &mut swap_buf, &SwapPolicy::for_modules());

        let state_keys: Vec<&String> = state["active"].as_object().unwrap().keys().collect();
        assert_eq!(state_keys, ["g1", "n1", "m1", "m2"]);

        let swap_keys: Vec<&String> = swap_buf["active"].as_object().unwrap().keys().collect();
        assert_eq!(swap_keys, ["n1", "n2", "m1", "m2"]);

        // Move semantics: swap's propQueue/metaQueue are overwritten by
        // state's in pass 1, then copied back into state in pass 2. The
        // preset's propQueue writes are discarded — JS-exact.
        assert_eq!(state["propQueue"], json!({"m1": {"x": 1}}));
        assert_eq!(state["metaQueue"], json!({}));
        assert_eq!(swap_buf["propQueue"], json!({"m1": {"x": 1}}));
    }

    #[test]
    fn modules_gallery_modules_never_move_to_swap_in_pass1() {
        let mut state = map_of(&[
            ("registered", Value::Object(Map::new())),
            ("active", json!({"g1": module("g1", true)})),
            ("propQueue", Value::Object(Map::new())),
            ("metaQueue", Value::Object(Map::new())),
        ]);
        let mut swap_buf = map_of(&[
            ("registered", Value::Object(Map::new())),
            ("active", Value::Object(Map::new())),
            ("propQueue", Value::Object(Map::new())),
            ("metaQueue", Value::Object(Map::new())),
        ]);

        swap(&mut state, &mut swap_buf, &SwapPolicy::for_modules());

        assert!(state["active"].as_object().unwrap().contains_key("g1"));
        assert!(!swap_buf["active"].as_object().unwrap().contains_key("g1"));
    }

    #[test]
    fn inputs_plain_swap_replaces_wholesale_and_resets() {
        let mut state = map_of(&[
            ("focusedInput", json!({"id": "in-old", "title": "Old"})),
            (
                "inputs",
                json!({"in-old": {"id": "in-old", "type": "action"}}),
            ),
            (
                "inputLinks",
                json!({"in-old": {"id": "in-old", "type": "state"}}),
            ),
        ]);
        let mut swap_buf = map_of(&[
            ("focusedInput", json!({"id": null, "title": null})),
            (
                "inputs",
                json!({"in-preset": {"id": "in-preset", "type": "commit"}}),
            ),
            ("inputLinks", json!({})),
        ]);

        swap(&mut state, &mut swap_buf, &SwapPolicy::for_inputs());

        // Live inputs are replaced by the preset's inputs.
        assert_eq!(
            state["inputs"],
            json!({"in-preset": {"id": "in-preset", "type": "commit"}})
        );
        // focusedInput is reset to the default null pair (preset loads do not
        // carry theirs through the swap — setFocusedInput runs without
        // writeToSwap — JS-exact).
        assert_eq!(state["focusedInput"], json!({"id": null, "title": null}));
        assert_eq!(state["inputLinks"], json!({}));
        // setDefault=true: the swap buffer is reset to defaults.
        assert_eq!(swap_buf["inputs"], json!({}));
        assert_eq!(swap_buf["inputLinks"], json!({}));
        assert_eq!(swap_buf["focusedInput"], json!({"id": null, "title": null}));
    }

    #[test]
    fn expressions_plain_swap_replaces_wholesale_and_resets() {
        let mut state = map_of(&[(
            "assignments",
            json!({"ex-old": {"id": "ex-old", "expression": "value"}}),
        )]);
        let mut swap_buf = map_of(&[(
            "assignments",
            json!({"ex-preset": {"id": "ex-preset", "expression": "value * 2"}}),
        )]);

        swap(&mut state, &mut swap_buf, &SwapPolicy::for_expressions());

        assert_eq!(
            state["assignments"],
            json!({"ex-preset": {"id": "ex-preset", "expression": "value * 2"}})
        );
        assert_eq!(swap_buf["assignments"], json!({}));
    }

    #[test]
    fn unrestricted_mode_tolerates_missing_state_counterpart() {
        // JS would throw spreading undefined for arrays; Rust treats a
        // missing state key as empty.
        let mut state = IndexMap::new();
        let mut swap_buf = map_of(&[("tweens", json!([1, 2, 3]))]);

        swap(&mut state, &mut swap_buf, &SwapPolicy::for_inputs());

        assert_eq!(state["tweens"], json!([1, 2, 3]));
    }

    #[test]
    fn unrestricted_mode_tolerates_missing_swap_counterpart() {
        let mut state = map_of(&[("assignments", json!({"a": 1}))]);
        let mut swap_buf = IndexMap::new();

        swap(&mut state, &mut swap_buf, &SwapPolicy::for_expressions());

        // Nothing in the swap buffer: state keeps… the pass-1 `delete state[key]`
        // behaviour leaves the state key *deleted* (JS-exact).
        assert!(!state.contains_key("assignments"));
    }

    #[test]
    fn array_restriction_on_non_array_value_leaves_it_untouched() {
        let mut state = map_of(&[("groups", json!({"not": "an array"}))]);
        let mut swap_buf = map_of(&[("groups", json!([group("preset-a", "Preset A")]))]);

        swap(&mut state, &mut swap_buf, &SwapPolicy::for_groups());

        assert_eq!(state["groups"], json!({"not": "an array"}));
    }

    #[test]
    fn object_keep_predicate_is_applied_to_swap_buffer_in_pass2() {
        // A gallery module sitting in the swap buffer stays there even when
        // the live state has no gallery module (synthetic but allowed).
        let mut state = map_of(&[
            ("registered", Value::Object(Map::new())),
            ("active", json!({"m1": module("m1", false)})),
            ("propQueue", Value::Object(Map::new())),
            ("metaQueue", Value::Object(Map::new())),
        ]);
        let mut swap_buf = map_of(&[
            ("registered", Value::Object(Map::new())),
            (
                "active",
                json!({"g1": module("g1", true), "n1": module("n1", false)}),
            ),
            ("propQueue", Value::Object(Map::new())),
            ("metaQueue", Value::Object(Map::new())),
        ]);

        swap(&mut state, &mut swap_buf, &SwapPolicy::for_modules());

        let state_keys: Vec<&String> = state["active"].as_object().unwrap().keys().collect();
        assert_eq!(state_keys, ["n1", "m1"]);
        let swap_keys: Vec<&String> = swap_buf["active"].as_object().unwrap().keys().collect();
        assert_eq!(swap_keys, ["g1", "n1", "m1"]);
    }

    #[test]
    fn set_default_false_keeps_swap_buffer_contents() {
        let mut state = map_of(&[("groups", Value::Array(vec![group("old1", "Old 1")]))]);
        let mut swap_buf = map_of(&[("groups", Value::Array(vec![]))]);

        swap(&mut state, &mut swap_buf, &SwapPolicy::for_groups());

        // reset_swap=false: the swap buffer is NOT reset (groups CLEAR_SWAP
        // commits that later in the JS flow).
        assert_eq!(group_ids(&swap_buf, "groups"), vec!["old1"]);
    }

    #[test]
    fn pass2_missing_state_key_treats_state_as_empty() {
        // groups (ArrayKeep) where state lost the key entirely: pass 2 must
        // not panic, and the preset groups land in state.
        let mut state = IndexMap::new();
        let mut swap_buf = map_of(&[("groups", Value::Array(vec![group("preset-a", "Preset A")]))]);

        swap(&mut state, &mut swap_buf, &SwapPolicy::for_groups());

        assert_eq!(group_ids(&state, "groups"), vec!["preset-a"]);
    }
}
