//! Deterministic store fixtures for the M2 parity gate (WS-1, `docs/rust-hybrid.md`).
//!
//! `modv-engine --emit-fixtures <DIR>` runs a fixed session against a fresh
//! [`Store`] — no GPU, no window, no control socket — and writes two committed
//! fixtures:
//!
//! * `deltas.json` — the `{ type, payload }` mutation stream the renderer's
//!   mirror store replays (`worker/index.worker.js:136-144`).
//! * `snapshot.json` — the [`Store::snapshot`] value, i.e. the six-key preset
//!   document plus `version` and `renderOrder`.
//!
//! `tests/unit/engine-mirror.test.js` replays the deltas into the real worker
//! (mirror) store and compares the result with the snapshot. That is the M2
//! parity gate: a Rust session and the JavaScript mirror agree on the state the
//! records produce.
//!
//! ## Deterministic ids
//!
//! Groups and modules get random UUIDv4 ids ([`crate::ids::uuid_v4`]) so a raw
//! capture would differ on every run. The engine is deliberately left alone:
//! determinism is the emitter's job. After the session the emitter walks the
//! delta stream in issue order, collects the ids the store generated (the
//! `group.id` of every `groups/ADD_GROUP`, the `module.$id` of every
//! `modules/ADD_ACTIVE_MODULE`) and maps them to stable placeholders —
//! `group-1`, `group-2`, … for groups, `module-1`, `module-2`, … for modules.
//!
//! The mapping is then applied to both fixtures, to every string value *and*
//! to every object key: a module id is also the key of its entry in the
//! `modules` map. Ids only ever appear as whole JSON strings (never as a
//! substring of another value — names, status messages and props are
//! consumer-authored), and the emitter asserts that no UUID-shaped string
//! survives the substitution, so a mapping gap fails loudly instead of writing
//! a fixture that only compares equal on the machine that produced it.
//!
//! Regenerate after a mutation-shape change with:
//!
//! ```text
//! make engine
//! ./crates/target/release/modv-engine --emit-fixtures tests/fixtures/engine
//! ```
//!
//! (or the equivalent `docker run` from the repository root, per `AGENTS.md`).

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use modv_engine::delta::Mutation;
use modv_engine::engine_state::GALLERY_GROUP_NAME;

use crate::store_ops::{Store, StoreError};

/// File name of the replayed mutation stream inside the fixture directory.
pub const DELTAS_FILE: &str = "deltas.json";

/// File name of the expected snapshot inside the fixture directory.
pub const SNAPSHOT_FILE: &str = "snapshot.json";

/// Errors from the fixture emitter.
#[derive(Debug)]
pub enum FixtureError {
    /// A store action in the scripted session failed.
    Store(StoreError),
    /// A session action returned a value without the id the script expects.
    Shape(String),
    /// A generated id survived substitution — a mapping gap, not a data issue.
    UnmappedId(String),
    /// Writing the fixtures failed.
    Io(io::Error),
}

impl std::fmt::Display for FixtureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Store(error) => write!(f, "{error}"),
            Self::Shape(detail) => write!(f, "unexpected session result: {detail}"),
            Self::UnmappedId(id) => {
                write!(f, "generated id {id} survived fixture substitution")
            }
            Self::Io(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for FixtureError {}

impl From<StoreError> for FixtureError {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

impl From<io::Error> for FixtureError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// A captured fixture pair, with generated ids already replaced.
pub struct Fixture {
    /// The mutation stream, in issue order.
    pub deltas: Vec<Mutation>,
    /// The expected snapshot ([`Store::snapshot`]).
    pub snapshot: Value,
    /// The `generated id → placeholder` mapping, in first-appearance order.
    pub ids: Vec<(String, String)>,
}

/// Run the scripted session, then capture and normalise the fixture pair.
///
/// The session is shared by `--emit-fixtures` and the Rust test in this module,
/// so the committed fixtures and the tested sequence can never drift apart.
pub fn capture(store: &mut Store) -> Result<Fixture, FixtureError> {
    let session = run_session(store)?;

    let deltas = store.drain_deltas();
    let snapshot = store.snapshot();
    let ids = collect_ids(&deltas)?;

    let map: HashMap<String, String> = ids.iter().cloned().collect();
    // The walk above only knows about ids carried by the records; every id the
    // session created must be covered, removed objects included.
    for generated in session.ids() {
        if !map.contains_key(generated) {
            return Err(FixtureError::UnmappedId(generated.clone()));
        }
    }
    let deltas = substitute_in_deltas(deltas, &map);
    let mut snapshot = snapshot;
    substitute_ids(&mut snapshot, &map);

    // Fail loudly rather than commit machine-specific ids.
    if let Some(id) = first_uuid_like(&serde_json::to_value(&deltas).unwrap_or(Value::Null)) {
        return Err(FixtureError::UnmappedId(id));
    }
    if let Some(id) = first_uuid_like(&snapshot) {
        return Err(FixtureError::UnmappedId(id));
    }

    Ok(Fixture {
        deltas,
        snapshot,
        ids,
    })
}

/// Write `deltas.json` and `snapshot.json` into `dir`, printing a summary.
///
/// The directory is created when missing; existing files are replaced.
pub fn emit(dir: &Path) -> Result<(), FixtureError> {
    let mut store = Store::new();
    let fixture = capture(&mut store)?;

    fs::create_dir_all(dir)?;

    let deltas = serde_json::to_vec_pretty(&fixture.deltas).map_err(io::Error::other)?;
    let mut snapshot = serde_json::to_vec_pretty(&fixture.snapshot).map_err(io::Error::other)?;
    snapshot.push(b'\n');

    let deltas_path = dir.join(DELTAS_FILE);
    let snapshot_path = dir.join(SNAPSHOT_FILE);
    fs::write(&deltas_path, deltas)?;
    fs::write(&snapshot_path, snapshot)?;

    let groups = fixture.snapshot["groups"].as_array().map_or(0, Vec::len);
    let modules = fixture.snapshot["modules"].as_object().map_or(0, Map::len);

    println!(
        "fixtures: {} deltas, {} preset groups, {} preset modules",
        fixture.deltas.len(),
        groups,
        modules
    );
    for (generated, placeholder) in &fixture.ids {
        println!("  {placeholder} = {generated}");
    }
    println!("  deltas   → {}", absolute(&deltas_path));
    println!("  snapshot → {}", absolute(&snapshot_path));
    Ok(())
}

/// The scripted session the fixture pair records.
///
/// Order matters: it walks the mirror-sensitive mutations in the order a real
/// session uses them, so the mirror store sees Groups appear before their
/// Modules are added and Modules exist before they are mutated or removed.
/// Every payload uses the exact mutation/action shape the renderer sends:
///
/// * `groups/createGroup` → `groups/ADD_GROUP`
/// * `modules/makeActiveModule` → `modules/ADD_ACTIVE_MODULE`
/// * `groups/ADD_MODULE_TO_GROUP`, `groups/REMOVE_MODULE_FROM_GROUP`
/// * `modules/updateMeta` → `modules/UPDATE_ACTIVE_MODULE_META`
/// * `modules/updateProp` → `modules/UPDATE_ACTIVE_MODULE_PROP` (coerced value)
/// * `groups/UPDATE_GROUP_BY_KEY` (alpha, compositeOperation, inheritFrom)
/// * `modules/removeActiveModule` → `modules/REMOVE_ACTIVE_MODULE`
///
/// `"Not A Real Module"` is activated on purpose: an unregistered module still
/// becomes an active record carrying the `$status` error entry the render loop
/// skips, and the mirror must reproduce that record. It is removed again so the
/// preset keeps only real modules — both the add and the remove are replayed.
pub fn run_session(store: &mut Store) -> Result<Session, FixtureError> {
    let background = create_group(store, json!({ "name": "Background" }))?;
    let gallery = create_group(store, json!({ "name": GALLERY_GROUP_NAME, "hidden": true }))?;
    let effects = create_group(store, json!({ "name": "Effects" }))?;

    // One shader module, one Point10 shader effect, one 2D module and one
    // module that is not registered at all.
    let bands = make_active_module(store, "Bands")?;
    let invert = make_active_module(store, "Point10 Invert")?;
    let ball = make_active_module(store, "Ball")?;
    let missing = make_active_module(store, "Not A Real Module")?;

    add_module_to_group(store, &background, &bands)?;
    add_module_to_group(store, &effects, &invert)?;
    add_module_to_group(store, &background, &ball)?;

    update_meta(store, &bands, "enabled", json!(true))?;
    update_meta(store, &invert, "enabled", json!(true))?;

    // `u_ringWidth` is Bands' only prop.
    update_prop(store, &bands, "u_ringWidth", json!(0.31))?;
    // `Ball.amount` is `int`, `strict`, range 1..=300 (`Ball.js:13-23`), so 500
    // exercises the coercion pipeline: clamp → 300, round → 300. The delta
    // carries the coerced value, which the mirror stores verbatim.
    update_prop(store, &ball, "amount", json!(500))?;

    update_group_key(store, &background, "alpha", json!(0.35))?;
    update_group_key(store, &effects, "compositeOperation", json!("multiply"))?;
    update_group_key(store, &effects, "inheritFrom", json!(-1))?;

    remove_active_module(store, &missing)?;
    remove_module_from_group(store, &background, &ball)?;

    Ok(Session {
        background,
        gallery,
        effects,
        bands,
        invert,
        ball,
        missing,
    })
}

/// The ids the session created, in script order (tests / diagnostics).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    /// `"Background"` group id.
    pub background: String,
    /// Gallery group id.
    pub gallery: String,
    /// `"Effects"` group id.
    pub effects: String,
    /// `"Bands"` module id.
    pub bands: String,
    /// `"Point10 Invert"` module id.
    pub invert: String,
    /// `"Ball"` module id.
    pub ball: String,
    /// `"Not A Real Module"` module id (removed again).
    pub missing: String,
}

impl Session {
    /// Every generated id, in first-appearance order.
    pub fn ids(&self) -> [&String; 7] {
        [
            &self.background,
            &self.gallery,
            &self.effects,
            &self.bands,
            &self.invert,
            &self.ball,
            &self.missing,
        ]
    }
}

/// `groups/createGroup`, returning the new Group id.
fn create_group(store: &mut Store, payload: Value) -> Result<String, FixtureError> {
    let group = store.dispatch("groups/createGroup", payload)?;
    string_at(&group, "id", "groups/createGroup")
}

/// `modules/makeActiveModule`, returning the new module `$id`.
fn make_active_module(store: &mut Store, module_name: &str) -> Result<String, FixtureError> {
    let module = store.dispatch(
        "modules/makeActiveModule",
        json!({ "moduleName": module_name }),
    )?;
    string_at(&module, "$id", "modules/makeActiveModule")
}

fn add_module_to_group(
    store: &mut Store,
    group_id: &str,
    module_id: &str,
) -> Result<(), FixtureError> {
    store.dispatch(
        "groups/ADD_MODULE_TO_GROUP",
        json!({ "groupId": group_id, "moduleId": module_id }),
    )?;
    Ok(())
}

fn remove_module_from_group(
    store: &mut Store,
    group_id: &str,
    module_id: &str,
) -> Result<(), FixtureError> {
    store.dispatch(
        "groups/REMOVE_MODULE_FROM_GROUP",
        json!({ "groupId": group_id, "moduleId": module_id }),
    )?;
    Ok(())
}

fn update_group_key(
    store: &mut Store,
    group_id: &str,
    key: &str,
    value: Value,
) -> Result<(), FixtureError> {
    store.dispatch(
        "groups/UPDATE_GROUP_BY_KEY",
        json!({ "groupId": group_id, "key": key, "value": value }),
    )?;
    Ok(())
}

fn update_meta(
    store: &mut Store,
    module_id: &str,
    meta_key: &str,
    value: Value,
) -> Result<(), FixtureError> {
    store.dispatch(
        "modules/updateMeta",
        json!({ "id": module_id, "metaKey": meta_key, "value": value }),
    )?;
    Ok(())
}

fn update_prop(
    store: &mut Store,
    module_id: &str,
    prop: &str,
    value: Value,
) -> Result<(), FixtureError> {
    store.dispatch(
        "modules/updateProp",
        json!({ "moduleId": module_id, "prop": prop, "data": { "value": value } }),
    )?;
    Ok(())
}

fn remove_active_module(store: &mut Store, module_id: &str) -> Result<(), FixtureError> {
    store.dispatch(
        "modules/removeActiveModule",
        json!({ "moduleId": module_id }),
    )?;
    Ok(())
}

/// Read a string field from a session result.
fn string_at(value: &Value, key: &str, action: &str) -> Result<String, FixtureError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| FixtureError::Shape(format!("{action} result has no string \"{key}\"")))
}

// ————————————————— Id normalisation —————————————————

/// Collect the generated ids in issue order and pair them with placeholders.
///
/// Groups are numbered before modules because the session creates them first;
/// the walk is over the delta stream (not the final state) so ids of objects
/// removed during the session — the unregistered module — are mapped too.
fn collect_ids(deltas: &[Mutation]) -> Result<Vec<(String, String)>, FixtureError> {
    let mut groups: Vec<String> = Vec::new();
    let mut modules: Vec<String> = Vec::new();

    for delta in deltas {
        match delta.r#type.as_str() {
            "groups/ADD_GROUP" => {
                let id = string_at(&delta.payload["group"], "id", "groups/ADD_GROUP payload")?;
                if !groups.contains(&id) {
                    groups.push(id);
                }
            }
            "modules/ADD_ACTIVE_MODULE" => {
                let id = string_at(
                    &delta.payload["module"],
                    "$id",
                    "modules/ADD_ACTIVE_MODULE payload",
                )?;
                if !modules.contains(&id) {
                    modules.push(id);
                }
            }
            _ => {}
        }
    }

    let mut mapping = Vec::with_capacity(groups.len() + modules.len());
    for (index, id) in groups.into_iter().enumerate() {
        mapping.push((id, format!("group-{}", index + 1)));
    }
    for (index, id) in modules.into_iter().enumerate() {
        mapping.push((id, format!("module-{}", index + 1)));
    }
    Ok(mapping)
}

/// Replace generated ids in every delta payload.
fn substitute_in_deltas(deltas: Vec<Mutation>, map: &HashMap<String, String>) -> Vec<Mutation> {
    deltas
        .into_iter()
        .map(|mut delta| {
            substitute_ids(&mut delta.payload, map);
            delta
        })
        .collect()
}

/// Replace generated ids in every string value and object key of `value`.
///
/// Object keys are covered because a module id is the key of its entry in the
/// preset's `modules` map. Ids only occur as whole strings, so an exact match is
/// sufficient — and safer than a JSON-text search, which could rewrite a
/// substring inside a name or status message.
fn substitute_ids(value: &mut Value, map: &HashMap<String, String>) {
    match value {
        Value::String(text) => {
            if let Some(placeholder) = map.get(text.as_str()) {
                *text = placeholder.clone();
            }
        }
        Value::Array(entries) => {
            for entry in entries {
                substitute_ids(entry, map);
            }
        }
        Value::Object(object) => {
            for (key, mut entry) in std::mem::take(object) {
                substitute_ids(&mut entry, map);
                let key = map.get(&key).cloned().unwrap_or(key);
                object.insert(key, entry);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

/// The first UUID-shaped string in `value`, if any (substitution safety net).
fn first_uuid_like(value: &Value) -> Option<String> {
    match value {
        Value::String(text) if looks_like_uuid(text) => Some(text.clone()),
        Value::Array(entries) => entries.iter().find_map(first_uuid_like),
        Value::Object(object) => object.iter().find_map(|(key, entry)| {
            if looks_like_uuid(key) {
                Some(key.clone())
            } else {
                first_uuid_like(entry)
            }
        }),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => None,
    }
}

/// `true` for the canonical `8-4-4-4-12` hex UUID shape `crate::ids` emits.
fn looks_like_uuid(text: &str) -> bool {
    const GROUPS: [usize; 5] = [8, 4, 4, 4, 12];
    if text.len() != 36 {
        return false;
    }

    let mut group = 0;
    let mut seen = 0;
    for character in text.chars() {
        if character == '-' {
            if seen != GROUPS[group] {
                return false;
            }
            group += 1;
            seen = 0;
            if group == GROUPS.len() {
                return false;
            }
            continue;
        }
        if !character.is_ascii_hexdigit() {
            return false;
        }
        seen += 1;
    }
    group == GROUPS.len() - 1 && seen == GROUPS[GROUPS.len() - 1]
}

/// Echo the fixture path relative to the working directory when possible.
fn absolute(path: &Path) -> String {
    if path.is_absolute() {
        path.display().to_string()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| PathBuf::from(path))
            .display()
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact delta sequence the committed fixtures contain. Pinned so a
    /// shape/order change in the session is a visible test failure rather than
    /// a silently stale fixture file.
    const EXPECTED_TYPES: [&str; 19] = [
        "groups/ADD_GROUP",
        "groups/ADD_GROUP",
        "groups/ADD_GROUP",
        "modules/ADD_ACTIVE_MODULE",
        "modules/ADD_ACTIVE_MODULE",
        "modules/ADD_ACTIVE_MODULE",
        "modules/ADD_ACTIVE_MODULE",
        "groups/ADD_MODULE_TO_GROUP",
        "groups/ADD_MODULE_TO_GROUP",
        "groups/ADD_MODULE_TO_GROUP",
        "modules/UPDATE_ACTIVE_MODULE_META",
        "modules/UPDATE_ACTIVE_MODULE_META",
        "modules/UPDATE_ACTIVE_MODULE_PROP",
        "modules/UPDATE_ACTIVE_MODULE_PROP",
        "groups/UPDATE_GROUP_BY_KEY",
        "groups/UPDATE_GROUP_BY_KEY",
        "groups/UPDATE_GROUP_BY_KEY",
        "modules/REMOVE_ACTIVE_MODULE",
        "groups/REMOVE_MODULE_FROM_GROUP",
    ];

    fn fixture() -> Fixture {
        let mut store = Store::new();
        capture(&mut store).expect("the scripted session captures")
    }

    #[test]
    fn delta_sequence_is_the_pinned_script() {
        let fixture = fixture();
        let types: Vec<&str> = fixture
            .deltas
            .iter()
            .map(|delta| delta.r#type.as_str())
            .collect();
        assert_eq!(types, EXPECTED_TYPES);
    }

    #[test]
    fn deltas_serialise_as_type_payload_records() {
        let fixture = fixture();
        let value = serde_json::to_value(&fixture.deltas).expect("deltas serialise");
        let records = value.as_array().expect("the delta stream is an array");
        assert_eq!(records.len(), EXPECTED_TYPES.len());

        for record in records {
            let object = record.as_object().expect("each delta is an object");
            let mut keys: Vec<&String> = object.keys().collect();
            keys.sort();
            assert_eq!(keys, vec!["payload", "type"], "exactly type + payload");
            assert!(object["type"].is_string());
            assert!(
                object["payload"].is_object(),
                "sessions payloads are objects"
            );
        }
    }

    #[test]
    fn snapshot_has_the_six_preset_keys_plus_bookkeeping() {
        let fixture = fixture();
        let snapshot = fixture.snapshot.as_object().expect("snapshot is an object");

        for key in [
            "groups",
            "modules",
            "inputs",
            "expressions",
            "tweens",
            "midi",
        ] {
            assert!(snapshot.contains_key(key), "snapshot keeps the {key} key");
        }
        assert_eq!(
            snapshot["version"],
            json!(crate::store_ops::SNAPSHOT_VERSION)
        );
        assert_eq!(snapshot["renderOrder"], json!([0, 2, 1]));

        // Empty subsections keep their canonical default shapes.
        assert_eq!(snapshot["inputs"]["inputs"], json!({}));
        assert_eq!(snapshot["inputs"]["inputLinks"], json!({}));
        assert_eq!(snapshot["expressions"]["assignments"], json!({}));
        assert_eq!(snapshot["tweens"], json!([]));
        assert_eq!(snapshot["midi"], json!({}));
    }

    #[test]
    fn snapshot_groups_and_modules_are_the_expected_counts() {
        let fixture = fixture();

        // Gallery group filtered, Background + Effects kept, Ball removed from
        // Background but the module itself still active.
        let groups = fixture.snapshot["groups"].as_array().unwrap();
        assert_eq!(groups.len(), 2);
        let names: Vec<&str> = groups
            .iter()
            .map(|group| group["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, vec!["Background", "Effects"]);
        assert!(groups.iter().all(|group| group.get("context").is_none()));

        // Four modules created, the unregistered one removed again.
        let modules = fixture.snapshot["modules"].as_object().unwrap();
        assert_eq!(modules.len(), 3);
        assert!(
            modules
                .values()
                .all(|module| module.get("$status").is_none())
        );

        // The mutating mutations landed.
        assert_eq!(groups[0]["alpha"], json!(0.35));
        assert_eq!(groups[1]["compositeOperation"], json!("multiply"));
        assert_eq!(groups[1]["inheritFrom"], json!(-1));
        assert_eq!(groups[0]["modules"], json!(["module-1"]));
        assert_eq!(groups[1]["modules"], json!(["module-2"]));
        assert_eq!(modules["module-1"]["props"]["u_ringWidth"], json!(0.31));
        // Coerced from 500 by `Ball.amount` (`strict`, range 1..=300). The
        // value is a float JSON number (`300.0`); JavaScript parses that as the
        // same `300` the mirror holds, which is what the vitest comparison
        // checks.
        assert_eq!(modules["module-3"]["props"]["amount"], json!(300.0));
        assert_eq!(modules["module-1"]["meta"]["enabled"], json!(true));
        assert_eq!(modules["module-2"]["meta"]["enabled"], json!(true));
    }

    #[test]
    fn generated_ids_become_stable_placeholders() {
        let fixture = fixture();

        let placeholders: Vec<&str> = fixture
            .ids
            .iter()
            .map(|(_generated, placeholder)| placeholder.as_str())
            .collect();
        assert_eq!(
            placeholders,
            vec![
                "group-1", "group-2", "group-3", "module-1", "module-2", "module-3", "module-4",
            ]
        );
        for (generated, placeholder) in &fixture.ids {
            assert!(looks_like_uuid(generated), "{generated} is a UUID");
            assert_ne!(generated, placeholder);
        }

        // The removed module's id is mapped too, so both its records agree.
        let value = serde_json::to_value(&fixture.deltas).unwrap();
        assert!(first_uuid_like(&value).is_none());
        assert!(first_uuid_like(&fixture.snapshot).is_none());
        assert_eq!(fixture.snapshot["groups"][0]["id"], json!(fixture.ids[0].1));
    }

    #[test]
    fn capture_is_deterministic_across_runs() {
        let first = fixture();
        let second = fixture();
        assert_eq!(first.deltas, second.deltas);
        assert_eq!(first.snapshot, second.snapshot);
        // Only the generated ids differ, and they are not in the output.
        assert_ne!(first.ids[0].0, second.ids[0].0);
    }

    #[test]
    fn emit_writes_both_fixture_files() {
        let dir = std::env::temp_dir().join(format!(
            "modv-engine-fixtures-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0)
        ));

        emit(&dir).expect("fixtures are written");

        let deltas: Value =
            serde_json::from_slice(&fs::read(dir.join(DELTAS_FILE)).expect("deltas.json exists"))
                .expect("deltas.json is JSON");
        let snapshot: Value = serde_json::from_slice(
            &fs::read(dir.join(SNAPSHOT_FILE)).expect("snapshot.json exists"),
        )
        .expect("snapshot.json is JSON");

        assert_eq!(deltas.as_array().map(Vec::len), Some(EXPECTED_TYPES.len()));
        assert_eq!(snapshot["groups"].as_array().map(Vec::len), Some(2));

        fs::remove_dir_all(&dir).expect("the temp fixture dir is removed");
    }

    #[test]
    fn looks_like_uuid_accepts_only_the_canonical_shape() {
        assert!(looks_like_uuid("5d9f4b36-8fd0-4a5e-bc1b-db5d0d8a0a01"));
        assert!(!looks_like_uuid("group-1"));
        assert!(!looks_like_uuid("5d9f4b36-8fd0-4a5e-bc1b-db5d0d8a0a0"));
        assert!(!looks_like_uuid("5d9f4b36x8fd0-4a5e-bc1b-db5d0d8a0a01"));
        assert!(!looks_like_uuid("5d9f4b36-8fd0-4a5e-bc1b-db5d0d8a0a0z"));
    }
}
