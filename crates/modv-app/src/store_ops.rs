//! Store semantics — the mutable engine state (WS-1 slice, M2/M3).
//!
//! [`Store`] wraps an [`EngineState`], a [`DeltaStream`], the current
//! [`AudioFrame`] and the combined [`ModuleRegistry`]. Mutations arrive over
//! the control socket as Vuex action names (`groups/createGroup`,
//! `modules/updateProp`, ...), are applied to the typed state, and emit the
//! exact `{ type, payload }` records the renderer's mirror store already
//! replays (`worker/index.worker.js:136-144`).
//!
//! ## Ported behaviour and its JS anchors
//!
//! * `groups/createGroup` — `store/modules/groups.js:131-202`. JS defaults are
//!   reproduced: name `"New Group"`, `clearing: false`, `hidden: false`,
//!   `modules: []`, `inherit: true`, `inheritFrom: -1`, `alpha: 1`,
//!   `pipeline: false`, `compositeOperation: "normal"`. Unknown payload keys
//!   are kept on the Group (the JS action shallow-spreads `groupArgs`).
//!   `writeToSwap`/`readFromSwap` are action arguments and are dropped
//!   (`groups.js:130-138`).
//! * `groups/UPDATE_GROUP_BY_KEY` — `groups.js:559-566`.
//! * `groups/ADD_MODULE_TO_GROUP` / `REMOVE_MODULE_FROM_GROUP` —
//!   `groups.js:511-542`.
//! * `groups/REPLACE_GROUPS` — `groups.js:474-500`, including the Gallery
//!   preservation rule (a layout without a Gallery group keeps the old one).
//! * `modules/makeActiveModule` — `modules.js:226-479`. Known modules get the
//!   registry `meta` and the registry prop defaults; unknown modules get the
//!   `$status: [{type: "error", ...}]` entry that makes the render loop skip
//!   them (`modules.js:253-268`). Both shader (`meta.type: "shader"`) and 2D
//!   (`meta.type: "2d"`) modules are registered (M3, see
//!   [`crate::module_registry`]).
//! * `modules/updateProp` — `modules.js:481-562`, with the coercion pipeline in
//!   [`modv_engine::props::coerce_prop_value`] fed by the same registry for
//!   both module kinds.
//! * `modules/updateMeta` / `modules/updateActiveModule` /
//!   `modules/removeActiveModule` — `modules.js:564-587`, `:858-888`, `:715`.
//! * `meyda/SET_BANDS` — the renderer's audio pump commits the AGC bands
//!   (`meyda.js:88-96`); the engine stores them in [`Store::audio`] for the
//!   compositor. No engine-side analysis yet (WS-9 / M6).
//!
//! ## Known gaps (documented, not silent)
//!
//! * **`data` starts empty.** JS initialises the module's `data` object from
//!   the definition's `data` export (`modules.js:330-338`), but the ported 2D
//!   modules keep that state inside the [`TwoDModule`] instance instead
//!   (`modv-renderers::two_d`), so the store's `data` stays `{}` and nothing
//!   is lost: the compositor owns the instance and its state.
//! * **Inputs are not created.** JS creates one input per Group control and
//!   per module prop/meta field (`groups.js:174-197`, `modules.js:291-318`)
//!   and stores their ids on the object (`alphaInputId`, `$props[x].id`).
//!   The input subsystem is WS-1b; M2 leaves those fields absent (`$props`
//!   stays `{}`), so preset round trips simply omit them.
//! * **No `getNextName` suffixing.** JS renames a second `Bands` instance to
//!   `Bands (1)` (`modules.js:340-342`, `utils/get-next-name.js`). M2 keeps
//!   `meta.name` equal to the registry name; instances are still unique by
//!   `$id`, which is what the state and the render loop key on.
//! * **No `set` hook / renderer side effects.** `updateProp`'s optional
//!   `props[x].set` call and the renderer `updateModule` hooks are renderer
//!   concerns that land with their porting milestones.
//! * **Unknown actions are accepted.** The renderer fires boot-time and
//!   UI actions the engine does not implement yet; those log once and return
//!   `Ok(null)`, exactly like the M1 supervisor did. The Scene actions live in
//!   [`crate::scene_ops`] (WS-8 / M5b); the performance runtime
//!   (`scenes/tick`, transitions, previews) is still no-op until M5c/M5d.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::Path;

use serde_json::{Map, Value, json};

use modv_engine::delta::{DeltaStream, Mutation};
use modv_engine::engine_state::{
    EngineState, GALLERY_GROUP_NAME, GROUP_DISABLED, Group, InheritFrom, ModuleInstance,
};
use modv_engine::preset::{PresetError, create_preset_from_state, load_preset_into_state};
use modv_engine::props::{PropSchema, coerce_prop_value};
use modv_renderers::two_d::AudioFrame;

use crate::module_registry::ModuleRegistry;

/// Version stamped into the on-disk snapshot document.
pub const SNAPSHOT_VERSION: i64 = 1;

/// Key of the engine sidecar inside the snapshot (`framesPresented`, ...).
///
/// The preset loader treats unknown top-level keys as Scene metadata and
/// ignores them (`preset-serializer.js` / [`load_preset_into_state`]), so the
/// sidecar never leaks into the six-key preset.
pub const ENGINE_SIDECAR_KEY: &str = "_engine";

/// Errors produced by [`Store::dispatch`] and [`Store::restore`].
#[derive(Debug)]
pub enum StoreError {
    /// A payload field was missing or had the wrong JSON type.
    InvalidPayload {
        /// The action being applied.
        action: String,
        /// What was wrong, e.g. `missing "groupId"`.
        detail: String,
    },
    /// A preset document could not be decoded.
    Preset(PresetError),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidPayload { action, detail } => write!(f, "{action}: {detail}"),
            Self::Preset(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<PresetError> for StoreError {
    fn from(error: PresetError) -> Self {
        Self::Preset(error)
    }
}

/// The authoritative engine store.
pub struct Store {
    pub(crate) state: EngineState,
    pub(crate) deltas: DeltaStream,
    pub(crate) registry: ModuleRegistry,
    /// The Scene list, performance config and seeding metadata (WS-8 / M5b).
    pub(crate) scenes: crate::scene_ops::ScenesState,
    /// `<state parent>/scenes.json`, resolved from `--state` on construction.
    pub(crate) scenes_path: Option<std::path::PathBuf>,
    /// The Scene this store last applied (`applySceneNow`'s active Scene).
    pub(crate) applied_scene_id: Option<String>,
    /// The current audio frame handed to modules and shader uniforms.
    ///
    /// Filled by `meyda/SET_BANDS` from the renderer's audio pump (see
    /// [`Store::set_bands`]). The engine performs no analysis yet (WS-9 / M6),
    /// so wave/FFT/features stay zeroed.
    audio: AudioFrame,
    pub(crate) warned: HashSet<String>,
}

impl Default for Store {
    fn default() -> Self {
        Self::new()
    }
}

impl Store {
    /// An empty store with the built-in shader registry and no Scene storage.
    pub fn new() -> Self {
        Self::with_state_path(None)
    }

    /// A store whose Scene list lives beside the engine's `--state` file
    /// (`<state parent>/scenes.json`, WS-13).
    ///
    /// The path is only resolved here; the renderer's boot dispatch
    /// (`scenes/hydrateScenes` + `scenes/ensureDefaultScenes`) loads or seeds
    /// it. A store without a state path keeps Scenes in memory only.
    pub fn with_state_path(state_path: Option<&std::path::Path>) -> Self {
        let scenes_path = state_path
            .and_then(std::path::Path::parent)
            .map(|parent| parent.join(crate::scene_ops::SCENES_STORAGE_FILENAME));
        Self {
            state: EngineState::new(),
            deltas: DeltaStream::new(),
            registry: ModuleRegistry::builtin(),
            scenes: crate::scene_ops::ScenesState::default(),
            scenes_path,
            applied_scene_id: None,
            audio: AudioFrame::default(),
            warned: HashSet::new(),
        }
    }

    /// The current state (read-only; the compositor renders from this).
    pub fn state(&self) -> &EngineState {
        &self.state
    }

    /// The current audio frame (read-only; the compositor clones it into
    /// [`crate::compositor::FrameInfo`]).
    pub fn audio(&self) -> &AudioFrame {
        &self.audio
    }

    /// Remove and return the pending `{ type, payload }` records, in issue
    /// order. The supervisor drains this after every request and broadcasts
    /// each record as a `state/delta` event.
    pub fn drain_deltas(&mut self) -> Vec<Mutation> {
        self.deltas.drain()
    }

    /// Number of queued delta records (tests / diagnostics).
    #[cfg(test)]
    pub fn pending_delta_count(&self) -> usize {
        self.deltas.len()
    }

    /// Apply one Vuex-style action/mutation and emit its mirror delta.
    ///
    /// [`Store::dispatch`] is used for both `state/dispatch` and
    /// `state/commit`: the renderer sends both kinds over the same channels
    /// and M2 has no separate "action" layer, so a commit is simply a
    /// dispatch whose return value the caller ignores. This is documented in
    /// the M2 report as a deliberate simplification; the split matters only
    /// once inputs/expressions (WS-1b, WS-12) subscribe to specific mutation
    /// types.
    ///
    /// Unknown actions log once and return `Ok(null)` — the renderer
    /// legitimately fires boot actions the engine does not implement yet.
    pub fn dispatch(&mut self, action: &str, payload: Value) -> Result<Value, StoreError> {
        let empty = Map::new();
        let params = payload.as_object().unwrap_or(&empty);

        match action {
            "groups/createGroup" => self.create_group(params),
            "groups/UPDATE_GROUP_BY_KEY" => self.update_group_by_key(params),
            "groups/ADD_MODULE_TO_GROUP" => self.add_module_to_group(params),
            "groups/REMOVE_MODULE_FROM_GROUP" => self.remove_module_from_group(params),
            "groups/REPLACE_GROUPS" => self.replace_groups(params),
            "modules/makeActiveModule" => self.make_active_module(params),
            "modules/updateProp" => self.update_prop(params),
            "modules/updateMeta" => self.update_meta(params),
            "modules/updateActiveModule" => self.update_active_module(params),
            "modules/removeActiveModule" => self.remove_active_module(params),
            "meyda/SET_BANDS" => self.set_bands(params),
            // ————————————————— Scenes (WS-8 / M5b) —————————————————
            "scenes/hydrateScenes" => self.hydrate_scenes(),
            "scenes/ensureDefaultScenes" => self.ensure_default_scenes(),
            "scenes/loadScenesFromDisk" => self.load_scenes_from_disk(),
            "scenes/loadSceneFromJson" => self.load_scene_from_json(params),
            "scenes/importPlaylist" => self.import_playlist_action(params),
            "scenes/exportPlaylist" => Ok(self.export_playlist_action()),
            "scenes/removeScene" => self.remove_scene_action(params),
            "scenes/setRowNames" => self.set_row_names(params),
            "scenes/updatePerformance" => self.update_performance_action(params),
            "scenes/setPerformanceEnabled" => self.set_performance_enabled(params),
            "scenes/applyScene" => self.apply_scene_action(params),
            "scenes/captureLiveStateToScene" => self.capture_live_state_to_scene(params),
            "scenes/persistScenes" => {
                if let Err(error) = self.persist_scenes() {
                    eprintln!("engine: WARN could not save the Scene list: {error}");
                }
                Ok(Value::Null)
            }
            other => {
                if self.warned.insert(format!("action:{other}")) {
                    eprintln!("engine: WARN dispatch {other} not implemented yet (M2+)");
                }
                Ok(Value::Null)
            }
        }
    }

    // ————————————————— Groups —————————————————

    /// `groups/createGroup` (`groups.js:131-202`).
    fn create_group(&mut self, params: &Map<String, Value>) -> Result<Value, StoreError> {
        // `groups.js:143-149`: creating a Group with an id that already exists
        // returns the existing record instead of duplicating it.
        if let Some(id) = params.get("id").and_then(Value::as_str) {
            if let Some(existing) = self.state.find_group(id) {
                return Ok(json_value(existing));
            }
        }

        let id = params
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(crate::ids::uuid_v4);
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("New Group")
            .to_string();

        let group = Group {
            id,
            name,
            hidden: bool_field(params, "hidden").unwrap_or(false),
            modules: string_array_field(params, "modules").unwrap_or_default(),
            // JS writes `Number(args.enabled) ?? GROUP_DISABLED`; the `??`
            // never fires for the `NaN` produced by `Number(undefined)`, but
            // the intent is the disabled constant and that is what the engine
            // persists (documented deviation, see the module docs).
            enabled: int_field(params, "enabled").unwrap_or(GROUP_DISABLED),
            alpha: number_field(params, "alpha").unwrap_or(1.0),
            inherit: bool_field(params, "inherit").unwrap_or(true),
            inherit_from: inherit_from_value(params.get("inheritFrom")),
            pipeline: bool_field(params, "pipeline").unwrap_or(false),
            clearing: bool_field(params, "clearing").unwrap_or(false),
            composite_operation: params
                .get("compositeOperation")
                .and_then(Value::as_str)
                .unwrap_or("normal")
                .to_string(),
            scene_id: optional_string(params, "sceneId"),
            scene_row: optional_usize(params, "sceneRow"),
            transition_ghost: optional_bool(params, "transitionGhost"),
            draw_to_canvas_id: optional_string(params, "drawToCanvasId"),
            alpha_input_id: optional_string(params, "alphaInputId"),
            enabled_input_id: optional_string(params, "enabledInputId"),
            clearing_input_id: optional_string(params, "clearingInputId"),
            inherit_input_id: optional_string(params, "inheritInputId"),
            composite_operation_input_id: optional_string(params, "compositeOperationInputId"),
            pipeline_input_id: optional_string(params, "pipelineInputId"),
            extra: unknown_keys(params, CREATE_GROUP_KEYS),
        };

        let value = json_value(&group);
        self.state.groups.push(group);
        self.deltas
            .push("groups/ADD_GROUP", json!({ "group": value }));
        Ok(value)
    }

    /// `groups/UPDATE_GROUP_BY_KEY` (`groups.js:559-566`).
    fn update_group_by_key(&mut self, params: &Map<String, Value>) -> Result<Value, StoreError> {
        let action = "groups/UPDATE_GROUP_BY_KEY";
        let group_id = required_str(params, action, "groupId")?.to_string();
        let key = required_str(params, action, "key")?.to_string();
        // The JS action passes `data`; the task-level dispatch shape uses
        // `value`. Accept both so either caller works.
        let value = params
            .get("value")
            .or_else(|| params.get("data"))
            .cloned()
            .unwrap_or(Value::Null);

        let Some(group) = self.state.find_group_mut(&group_id) else {
            self.warn_once(
                format!("missing-group:{group_id}"),
                &format!("engine: WARN {action}: no such group {group_id}"),
            );
            return Ok(Value::Null);
        };

        match key.as_str() {
            "name" => group.name = string_value(&value, &group.name),
            "hidden" => group.hidden = bool_value(&value, group.hidden),
            "enabled" => group.enabled = int_value(&value, group.enabled),
            "alpha" => group.alpha = number_value(&value, group.alpha),
            "inherit" => group.inherit = bool_value(&value, group.inherit),
            "inheritFrom" => group.inherit_from = inherit_from_value(Some(&value)),
            "pipeline" => group.pipeline = bool_value(&value, group.pipeline),
            "clearing" => group.clearing = bool_value(&value, group.clearing),
            "compositeOperation" => {
                group.composite_operation = string_value(&value, &group.composite_operation)
            }
            "modules" => group.modules = string_array_value(&value).unwrap_or_default(),
            "sceneId" => group.scene_id = value.as_str().map(str::to_string),
            "sceneRow" => group.scene_row = value.as_u64().map(|row| row as usize),
            "transitionGhost" => group.transition_ghost = value.as_bool(),
            "drawToCanvasId" => group.draw_to_canvas_id = value.as_str().map(str::to_string),
            "alphaInputId" => group.alpha_input_id = value.as_str().map(str::to_string),
            "enabledInputId" => group.enabled_input_id = value.as_str().map(str::to_string),
            "clearingInputId" => group.clearing_input_id = value.as_str().map(str::to_string),
            "inheritInputId" => group.inherit_input_id = value.as_str().map(str::to_string),
            "compositeOperationInputId" => {
                group.composite_operation_input_id = value.as_str().map(str::to_string)
            }
            "pipelineInputId" => group.pipeline_input_id = value.as_str().map(str::to_string),
            // JS writes any key (`writeTo.groups[index][key] = data`); for the
            // typed model anything not modelled above is preserved verbatim in
            // `extra` so a round trip loses nothing.
            other => {
                group.extra.insert(other.to_string(), value.clone());
            }
        }

        self.deltas.push(
            action,
            json!({ "groupId": group_id, "key": key, "data": value }),
        );
        Ok(Value::Null)
    }

    /// `groups/ADD_MODULE_TO_GROUP` (`groups.js:511-526`).
    fn add_module_to_group(&mut self, params: &Map<String, Value>) -> Result<Value, StoreError> {
        let action = "groups/ADD_MODULE_TO_GROUP";
        let group_id = required_str(params, action, "groupId")?.to_string();
        let module_id = required_str(params, action, "moduleId")?.to_string();
        let position = params
            .get("position")
            .and_then(Value::as_u64)
            .map(|pos| pos as usize);

        let Some(group) = self.state.find_group_mut(&group_id) else {
            self.warn_once(
                format!("missing-group:{group_id}"),
                &format!("engine: WARN {action}: no such group {group_id}"),
            );
            return Ok(Value::Null);
        };

        let index = position
            .unwrap_or(group.modules.len())
            .min(group.modules.len());
        group.modules.insert(index, module_id.clone());

        self.deltas.push(
            action,
            json!({ "moduleId": module_id, "groupId": group_id, "position": index }),
        );
        Ok(Value::Null)
    }

    /// `groups/REMOVE_MODULE_FROM_GROUP` (`groups.js:528-542`).
    fn remove_module_from_group(
        &mut self,
        params: &Map<String, Value>,
    ) -> Result<Value, StoreError> {
        let action = "groups/REMOVE_MODULE_FROM_GROUP";
        let group_id = required_str(params, action, "groupId")?.to_string();
        let module_id = required_str(params, action, "moduleId")?.to_string();

        let Some(group) = self.state.find_group_mut(&group_id) else {
            self.warn_once(
                format!("missing-group:{group_id}"),
                &format!("engine: WARN {action}: no such group {group_id}"),
            );
            return Ok(Value::Null);
        };
        let Some(index) = group.modules.iter().position(|id| id == &module_id) else {
            return Ok(Value::Null);
        };
        group.modules.remove(index);

        self.deltas.push(
            action,
            json!({ "moduleId": module_id, "groupId": group_id }),
        );
        Ok(Value::Null)
    }

    /// `groups/REPLACE_GROUPS` (`groups.js:474-500`).
    fn replace_groups(&mut self, params: &Map<String, Value>) -> Result<Value, StoreError> {
        let action = "groups/REPLACE_GROUPS";
        let Some(entries) = params.get("groups").and_then(Value::as_array) else {
            return Err(invalid(action, "missing \"groups\" array"));
        };

        let mut groups = Vec::with_capacity(entries.len());
        for entry in entries {
            let group: Group = serde_json::from_value(entry.clone())
                .map_err(|error| invalid(action, format!("group did not decode: {error}")))?;
            groups.push(group);
        }

        // `groups.js:480-499`: preserve the Gallery group when the incoming
        // list has none. A Gallery-less layout genuinely has none to preserve.
        if !groups.iter().any(|group| group.name == GALLERY_GROUP_NAME) {
            if let Some(gallery) = self
                .state
                .groups
                .iter()
                .find(|group| group.name == GALLERY_GROUP_NAME)
            {
                groups.push(gallery.clone());
            }
        }

        let value = Value::Array(groups.iter().map(json_value).collect());
        self.state.groups = groups;
        // The mirror applies the same preservation rule to whatever list it is
        // given; sending the final list means it sees the Gallery group and
        // does not append a second copy.
        self.deltas.push(action, json!({ "groups": value }));
        Ok(Value::Null)
    }

    // ————————————————— Modules —————————————————

    /// `modules/makeActiveModule` (`modules.js:226-479`).
    ///
    /// Both registry kinds are accepted; `meta.type` comes from the registry
    /// (`"shader"` / `"2d"`). `data` starts empty: the ported 2D modules keep
    /// their mutable state inside the module instance the compositor owns, so
    /// unlike JS there is no module `data` export to copy
    /// (`modules.js:330-338`).
    fn make_active_module(&mut self, params: &Map<String, Value>) -> Result<Value, StoreError> {
        let action = "modules/makeActiveModule";
        let module_name = required_str(params, action, "moduleName")?.to_string();
        let module_meta = params
            .get("moduleMeta")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let is_gallery = module_meta
            .get("isGallery")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let id = crate::ids::uuid_v4();

        let (meta, props, status) = match self.registry.meta_fields(&module_name) {
            Some(registry_meta) => {
                let props = self
                    .registry
                    .default_props(&module_name)
                    .unwrap_or_else(|| json!({}));
                (
                    known_module_meta(registry_meta, &module_meta, is_gallery),
                    props,
                    Vec::new(),
                )
            }
            None => {
                // `modules.js:253-268`: an unregistered module still becomes an
                // active record, marked with an error status the render loop
                // (and the UI) can see.
                eprintln!(
                    "engine: ERROR Could not find registered module with name {module_name}."
                );
                let mut meta = Map::new();
                meta.insert("name".to_string(), Value::String(module_name.clone()));
                meta.insert("alpha".to_string(), json!(1));
                meta.insert("enabled".to_string(), json!(false));
                meta.insert("compositeOperation".to_string(), json!("normal"));
                // JS re-applies `moduleMeta` after its defaults, so caller meta
                // wins (`modules.js:350-359`).
                for (key, value) in module_meta {
                    meta.insert(key.clone(), value.clone());
                }
                meta.insert("isGallery".to_string(), json!(is_gallery));
                let status = vec![json!({
                    "type": "error",
                    "message": format!(
                        "Module \"{module_name}\" is not registered. VmodV will skip this while rendering"
                    ),
                })];
                (Value::Object(meta), json!({}), status)
            }
        };

        let module = ModuleInstance {
            id: id.clone(),
            module_name,
            meta,
            props,
            data: json!({}),
            dollar_props: json!({}),
            status,
            extra: Map::new(),
        };

        let value = module_json(&module);
        self.state.modules.insert(id, module);
        self.deltas
            .push("modules/ADD_ACTIVE_MODULE", json!({ "module": value }));
        Ok(value)
    }

    /// `modules/updateProp` (`modules.js:481-562`).
    fn update_prop(&mut self, params: &Map<String, Value>) -> Result<Value, StoreError> {
        let action = "modules/updateProp";
        let module_id = required_str(params, action, "moduleId")?.to_string();
        let prop = required_str(params, action, "prop")?.to_string();

        // `data` is `{ value, type, path }` (`modules.js:528-536`). A bare
        // value is accepted too — the coercion pipeline still runs.
        let data = params.get("data").cloned().unwrap_or(Value::Null);
        let (value, path, type_hint) = match &data {
            Value::Object(map) => (
                map.get("value").cloned().unwrap_or(Value::Null),
                map.get("path").and_then(Value::as_str).map(str::to_string),
                map.get("type").and_then(Value::as_str).map(str::to_string),
            ),
            other => (other.clone(), None, None),
        };
        let path = path.filter(|path| !path.is_empty());

        let Some(module) = self.state.modules.get_mut(&module_id) else {
            self.warn_once(
                format!("missing-module:{module_id}"),
                &format!(
                    "engine: WARN {action}: no module with id {module_id} (updateProp ignored)"
                ),
            );
            return Ok(Value::Null);
        };

        let module_name = module.module_name.clone();
        // Unknown props fall back to a schema that carries only the caller's
        // type hint and coerces nothing — JS would throw on a missing
        // `state.registered[name].props[prop]`; logging and storing the value is
        // the resilient equivalent.
        let schema = self
            .registry
            .prop_schema(&module_name, &prop)
            .or_else(|| {
                type_hint.as_ref().map(|type_| PropSchema {
                    type_: type_.clone(),
                    ..PropSchema::default()
                })
            })
            .unwrap_or_default();

        let coerced = coerce_prop_value(&schema, &value);
        set_prop_value(&mut module.props, &prop, path.as_deref(), coerced.clone());

        self.deltas.push(
            "modules/UPDATE_ACTIVE_MODULE_PROP",
            json!({
                "moduleId": module_id,
                "prop": prop,
                "data": {
                    "value": coerced,
                    "type": schema.type_,
                    "path": path.unwrap_or_default(),
                },
            }),
        );
        Ok(Value::Null)
    }

    /// `modules/updateMeta` (`modules.js:564-587`).
    fn update_meta(&mut self, params: &Map<String, Value>) -> Result<Value, StoreError> {
        let action = "modules/updateMeta";
        let id = params
            .get("id")
            .or_else(|| params.get("moduleId"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| invalid(action, "missing \"id\""))?;
        let meta_key = required_str(params, action, "metaKey")?.to_string();
        let data = params
            .get("value")
            .or_else(|| params.get("data"))
            .cloned()
            .unwrap_or(Value::Null);

        let Some(module) = self.state.modules.get_mut(&id) else {
            self.warn_once(
                format!("missing-module:{id}"),
                &format!("engine: WARN {action}: no module with id {id} (updateMeta ignored)"),
            );
            return Ok(Value::Null);
        };

        if !module.meta.is_object() {
            module.meta = Value::Object(Map::new());
        }
        if let Value::Object(meta) = &mut module.meta {
            meta.insert(meta_key.clone(), data.clone());
        }

        self.deltas.push(
            "modules/UPDATE_ACTIVE_MODULE_META",
            json!({ "id": id, "metaKey": meta_key, "data": data }),
        );
        Ok(Value::Null)
    }

    /// `modules/updateActiveModule` (`modules.js:858-862`).
    fn update_active_module(&mut self, params: &Map<String, Value>) -> Result<Value, StoreError> {
        let action = "modules/updateActiveModule";
        // The mirror replays MUTATIONS, so the delta uses the mutation name.
        let mutation_type = "modules/UPDATE_ACTIVE_MODULE";
        let id = params
            .get("id")
            .or_else(|| params.get("moduleId"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| invalid(action, "missing \"id\""))?;
        let key = required_str(params, action, "key")?.to_string();
        let value = params
            .get("value")
            .or_else(|| params.get("data"))
            .cloned()
            .unwrap_or(Value::Null);

        let Some(module) = self.state.modules.get_mut(&id) else {
            self.warn_once(
                format!("missing-module:{id}"),
                &format!(
                    "engine: WARN {action}: no module with id {id} (updateActiveModule ignored)"
                ),
            );
            return Ok(Value::Null);
        };

        match key.as_str() {
            "$id" => {
                if let Some(new_id) = value.as_str() {
                    module.id = new_id.to_string();
                }
            }
            "$moduleName" => {
                if let Some(name) = value.as_str() {
                    module.module_name = name.to_string();
                }
            }
            "meta" => module.meta = value.clone(),
            "props" => module.props = value.clone(),
            "data" => module.data = value.clone(),
            "$props" => module.dollar_props = value.clone(),
            "$status" => {
                if let Ok(status) = serde_json::from_value::<Vec<Value>>(value.clone()) {
                    module.status = status;
                }
            }
            other => {
                module.extra.insert(other.to_string(), value.clone());
            }
        }

        self.deltas.push(
            mutation_type,
            json!({ "id": id, "key": key, "value": value }),
        );
        Ok(Value::Null)
    }

    /// `modules/removeActiveModule` (`modules.js:715-...`).
    fn remove_active_module(&mut self, params: &Map<String, Value>) -> Result<Value, StoreError> {
        let action = "modules/removeActiveModule";
        let id = params
            .get("moduleId")
            .or_else(|| params.get("id"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| invalid(action, "missing \"moduleId\""))?;

        // Removing an already-gone module is a no-op rather than a fatal error
        // (`modules.js:720-724`).
        if self.state.modules.shift_remove(&id).is_none() {
            return Ok(Value::Null);
        }

        self.deltas
            .push("modules/REMOVE_ACTIVE_MODULE", json!({ "moduleId": id }));
        Ok(Value::Null)
    }

    // ————————————————— Audio —————————————————

    /// `meyda/SET_BANDS` (`meyda.js:88-96`): the renderer's audio pump commits
    /// `{ bass, mid, treble, cells }` over the control socket.
    ///
    /// Only the three AGC bands are taken. `cells` (the 16 log-spaced band
    /// cells `audio-bands.js` publishes) has no slot in
    /// [`modv_renderers::two_d::AudioFrame`] and no M3 module reads it; the
    /// cells stay a renderer-side visualisation (WS-9). Wave, FFT and the
    /// Meyda feature bag remain zeroed — real analysis lands with M6.
    ///
    /// No delta is emitted: the renderer's mirror computed these values
    /// itself, so echoing every audio frame back over the socket would be
    /// pure traffic. Unknown keys are ignored so a richer band payload is
    /// forward-compatible.
    fn set_bands(&mut self, params: &Map<String, Value>) -> Result<Value, StoreError> {
        // The renderer sends the bands object as the payload; accept a
        // `{ bands: {...} }` wrapper too.
        let bands = params
            .get("bands")
            .and_then(Value::as_object)
            .unwrap_or(params);

        if let Some(bass) = finite_f32(bands.get("bass")) {
            self.audio.bass = bass;
        }
        if let Some(mid) = finite_f32(bands.get("mid")) {
            self.audio.mid = mid;
        }
        if let Some(treble) = finite_f32(bands.get("treble")) {
            self.audio.treble = treble;
        }
        Ok(Value::Null)
    }

    // ————————————————— Snapshot / restore —————————————————

    /// The six-key preset document plus `version` and the render order.
    ///
    /// `renderOrder` is the `groupIndexRenderOrder` getter's output
    /// (`groups.js:111-123`) — indices into `groups` with the Gallery group
    /// moved last. It is metadata for the UI/debugging, not part of the preset
    /// format; [`Store::restore`] ignores it.
    pub fn snapshot(&self) -> Value {
        let mut preset = create_preset_from_state(&self.state);
        if let Value::Object(map) = &mut preset {
            map.insert("version".to_string(), json!(SNAPSHOT_VERSION));
            map.insert(
                "renderOrder".to_string(),
                json!(self.state.group_index_render_order()),
            );
        }
        preset
    }

    /// The on-disk snapshot: [`Store::snapshot`] plus the engine sidecar.
    ///
    /// `framesPresented` lives in `_engine` rather than in the preset so a
    /// Scene saved from this document never carries engine bookkeeping.
    pub fn snapshot_with_engine(&self, frames_presented: u64) -> Value {
        let mut snapshot = self.snapshot();
        if let Value::Object(map) = &mut snapshot {
            map.insert(
                ENGINE_SIDECAR_KEY.to_string(),
                json!({ "framesPresented": frames_presented }),
            );
        }
        snapshot
    }

    /// Write the snapshot atomically (tmp file + rename), the M1 behaviour.
    pub fn save_snapshot(&self, path: &Path, frames_presented: u64) -> io::Result<()> {
        let payload = serde_json::to_vec_pretty(&self.snapshot_with_engine(frames_presented))
            .map_err(io::Error::other)?;
        let tmp = Path::new(&format!("{}.tmp", path.display())).to_path_buf();
        fs::write(&tmp, payload)?;
        fs::rename(&tmp, path)
    }

    /// Replace the state from a preset/snapshot document
    /// (`load_preset_into_state`).
    ///
    /// Pending deltas are discarded: replaying pre-restore mutations into the
    /// mirror after its state has been replaced would resurrect stale objects.
    /// There is no "replace everything" delta in the M2 protocol; the UI
    /// re-reads `state/snapshot` after a restore (documented gap).
    pub fn restore(&mut self, preset: &Value) -> Result<(), StoreError> {
        load_preset_into_state(&mut self.state, preset)?;
        self.deltas.drain();
        Ok(())
    }

    /// Load a snapshot file into the state (startup path).
    ///
    /// Returns `Ok(false)` when the file is absent; a malformed document is an
    /// error so a corrupt snapshot is reported instead of silently starting
    /// empty.
    pub fn restore_from_file(&mut self, path: &Path) -> io::Result<bool> {
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error),
        };
        let value: Value = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        self.restore(&value).map_err(io::Error::other)?;
        Ok(true)
    }

    /// Log `message` at most once per `key`.
    pub(crate) fn warn_once(&mut self, key: String, message: &str) {
        if self.warned.insert(key) {
            eprintln!("{message}");
        }
    }
}

/// `framesPresented` from a snapshot document's `_engine` sidecar.
pub fn frames_presented_in(snapshot: &Value) -> Option<u64> {
    snapshot
        .get(ENGINE_SIDECAR_KEY)
        .and_then(|engine| engine.get("framesPresented"))
        .and_then(Value::as_u64)
}

// ————————————————— Helpers —————————————————

/// Payload keys consumed by [`Store::create_group`]; everything else is kept on
/// the Group as an unknown key, mirroring the JS shallow spread.
const CREATE_GROUP_KEYS: &[&str] = &[
    "id",
    "name",
    "hidden",
    "modules",
    "enabled",
    "alpha",
    "inherit",
    "inheritFrom",
    "pipeline",
    "clearing",
    "compositeOperation",
    "sceneId",
    "sceneRow",
    "transitionGhost",
    "drawToCanvasId",
    "alphaInputId",
    "enabledInputId",
    "clearingInputId",
    "inheritInputId",
    "compositeOperationInputId",
    "pipelineInputId",
    // Action arguments, never Group properties (`groups.js:130-138`).
    "writeToSwap",
    "readFromSwap",
    // Passed to `outputs/getAuxillaryOutput`, not stored on the Group.
    "reactToResize",
    "context",
];

/// The metadata of a known module, matching the JS construction order in
/// `modules.js:247-359`: registry meta, then the JS defaults for
/// name/alpha/enabled/compositeOperation, then the caller's `moduleMeta`.
fn known_module_meta(
    registry_meta: Map<String, Value>,
    module_meta: &Map<String, Value>,
    is_gallery: bool,
) -> Value {
    let mut meta = registry_meta;

    // JS defaults applied after the registry meta (`modules.js:344-346`).
    meta.insert("alpha".to_string(), json!(1));
    meta.insert("enabled".to_string(), json!(false));
    meta.insert("compositeOperation".to_string(), json!("normal"));

    // Caller-supplied meta wins (`modules.js:350-359`).
    for (key, value) in module_meta {
        meta.insert(key.clone(), value.clone());
    }
    meta.insert("isGallery".to_string(), json!(is_gallery));

    Value::Object(meta)
}

fn invalid(action: &str, detail: impl Into<String>) -> StoreError {
    StoreError::InvalidPayload {
        action: action.to_string(),
        detail: detail.into(),
    }
}

fn required_str<'a>(
    params: &'a Map<String, Value>,
    action: &str,
    key: &str,
) -> Result<&'a str, StoreError> {
    params
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(action, format!("missing string \"{key}\"")))
}

fn unknown_keys(params: &Map<String, Value>, known: &[&str]) -> Map<String, Value> {
    params
        .iter()
        .filter(|(key, _)| !known.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

/// Interpret a JSON value as a boolean with a fallback for the shapes the UI
/// sends (`true`, `1`, `"true"`).
fn bool_value(value: &Value, fallback: bool) -> bool {
    value
        .as_bool()
        .or_else(|| value.as_f64().map(|number| number != 0.0))
        .or_else(|| value.as_str().map(|text| text == "true" || text == "1"))
        .unwrap_or(fallback)
}

fn bool_field(params: &Map<String, Value>, key: &str) -> Option<bool> {
    params.get(key).map(|value| bool_value(value, false))
}

fn optional_bool(params: &Map<String, Value>, key: &str) -> Option<bool> {
    params.get(key).map(|value| bool_value(value, false))
}

fn number_value(value: &Value, fallback: f64) -> f64 {
    value.as_f64().unwrap_or(fallback)
}

fn number_field(params: &Map<String, Value>, key: &str) -> Option<f64> {
    params.get(key).and_then(Value::as_f64)
}

/// A finite `f32` from a JSON value (`None` for absent/NaN/±∞).
fn finite_f32(value: Option<&Value>) -> Option<f32> {
    value
        .and_then(Value::as_f64)
        .filter(|number| number.is_finite())
        .map(|number| number as f32)
}

fn int_value(value: &Value, fallback: i64) -> i64 {
    value
        .as_i64()
        .or_else(|| value.as_f64().map(|number| number as i64))
        .or_else(|| value.as_bool().map(i64::from))
        .unwrap_or(fallback)
}

fn int_field(params: &Map<String, Value>, key: &str) -> Option<i64> {
    params
        .get(key)
        .map(|value| int_value(value, GROUP_DISABLED))
}

fn string_value(value: &Value, fallback: &str) -> String {
    value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| fallback.to_string())
}

fn optional_string(params: &Map<String, Value>, key: &str) -> Option<String> {
    params.get(key).and_then(Value::as_str).map(str::to_string)
}

fn optional_usize(params: &Map<String, Value>, key: &str) -> Option<usize> {
    params
        .get(key)
        .and_then(Value::as_u64)
        .map(|value| value as usize)
}

fn string_array_value(value: &Value) -> Option<Vec<String>> {
    value.as_array().map(|entries| {
        entries
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect()
    })
}

fn string_array_field(params: &Map<String, Value>, key: &str) -> Option<Vec<String>> {
    params.get(key).and_then(string_array_value)
}

/// `inheritFrom` carries `-1` (previous Group in render order) or a Group UUID.
fn inherit_from_value(value: Option<&Value>) -> InheritFrom {
    match value {
        Some(Value::String(id)) => InheritFrom::Group(id.clone()),
        Some(other) => other
            .as_i64()
            .or_else(|| other.as_f64().map(|number| number as i64))
            .map(InheritFrom::Previous)
            .unwrap_or_default(),
        None => InheritFrom::default(),
    }
}

/// Serialise a value, falling back to `null` for the impossible failure.
pub(crate) fn json_value(value: &impl serde::Serialize) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

/// Serialise a live module for the renderer/mirror.
///
/// [`ModuleInstance::status`] is skipped when empty (the preset shape), but a
/// live JS module always carries `$status: []` (`modules.js:251-258`) and the
/// mirror's `ADD_ACTIVE_MODULE` payload must match the object the UI expects.
pub(crate) fn module_json(module: &ModuleInstance) -> Value {
    let mut value = json_value(module);
    if let Value::Object(map) = &mut value {
        map.entry("$status".to_string())
            .or_insert_with(|| json!([]));
    }
    value
}

/// Set `props[prop]` (optionally at a nested `path`) inside a prop bag.
///
/// The JS mutation writes `props[prop] = value`, or `lodash.set(props[prop],
/// path, value)` when a path is present (`modules.js:865-877`). Paths are
/// produced by `dataTypes.js` inputs as `[key]` / `[index]` selectors.
fn set_prop_value(props: &mut Value, prop: &str, path: Option<&str>, value: Value) {
    if !props.is_object() {
        *props = Value::Object(Map::new());
    }
    let Some(object) = props.as_object_mut() else {
        return;
    };

    match path {
        Some(path) => {
            let entry = object
                .entry(prop.to_string())
                .or_insert_with(|| Value::Object(Map::new()));
            set_json_path(entry, path, value);
        }
        None => {
            object.insert(prop.to_string(), value);
        }
    }
}

/// One segment of a lodash-style property path.
#[derive(Clone, Debug, PartialEq, Eq)]
enum PathSegment {
    /// Object key.
    Key(String),
    /// Array index.
    Index(usize),
}

/// Parse `"a.b"`, `"[0]"`, `"[0].x"`, `"[1][2]"` into segments.
fn parse_path(path: &str) -> Vec<PathSegment> {
    let mut segments = Vec::new();
    let mut key = String::new();
    let mut chars = path.chars().peekable();

    while let Some(character) = chars.next() {
        match character {
            '.' => {
                if !key.is_empty() {
                    segments.push(PathSegment::Key(std::mem::take(&mut key)));
                }
            }
            '[' => {
                if !key.is_empty() {
                    segments.push(PathSegment::Key(std::mem::take(&mut key)));
                }
                let mut inside = String::new();
                for next in chars.by_ref() {
                    if next == ']' {
                        break;
                    }
                    inside.push(next);
                }
                // `[0]` is an array index; `[key]` is the object-key form the
                // texture data types use (`dataTypes.js` inputs are `[${key}]`).
                if let Ok(index) = inside.parse::<usize>() {
                    segments.push(PathSegment::Index(index));
                } else if !inside.is_empty() {
                    segments.push(PathSegment::Key(inside));
                }
            }
            other => key.push(other),
        }
    }
    if !key.is_empty() {
        segments.push(PathSegment::Key(key));
    }
    segments
}

/// Apply `value` at `path` inside `root`, creating containers as needed.
fn set_json_path(root: &mut Value, path: &str, value: Value) {
    set_segments(root, &parse_path(path), value);
}

fn set_segments(root: &mut Value, segments: &[PathSegment], value: Value) {
    let Some((first, rest)) = segments.split_first() else {
        *root = value;
        return;
    };

    match first {
        PathSegment::Key(key) => {
            if !root.is_object() {
                *root = Value::Object(Map::new());
            }
            if let Value::Object(map) = root {
                let entry = map.entry(key.clone()).or_insert(Value::Null);
                set_segments(entry, rest, value);
            }
        }
        PathSegment::Index(index) => {
            if !root.is_array() {
                *root = Value::Array(Vec::new());
            }
            if let Value::Array(entries) = root {
                while entries.len() <= *index {
                    entries.push(Value::Null);
                }
                set_segments(&mut entries[*index], rest, value);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_with_group(name: &str) -> (Store, String) {
        let mut store = Store::new();
        let group = store
            .dispatch("groups/createGroup", json!({ "name": name }))
            .expect("createGroup dispatches");
        let id = group["id"].as_str().expect("id").to_string();
        (store, id)
    }

    fn gallery_group() -> Value {
        json!({
            "id": "gallery-id",
            "name": GALLERY_GROUP_NAME,
            "hidden": false,
            "modules": ["gallery-module"],
            "enabled": 1,
            "alpha": 1,
            "inherit": true,
            "inheritFrom": -1,
            "pipeline": false,
            "clearing": false,
            "compositeOperation": "normal",
        })
    }

    fn plain_group(id: &str, name: &str) -> Value {
        json!({
            "id": id,
            "name": name,
            "hidden": false,
            "modules": [],
            "enabled": 1,
            "alpha": 1,
            "inherit": true,
            "inheritFrom": -1,
            "pipeline": false,
            "clearing": false,
            "compositeOperation": "normal",
        })
    }

    #[test]
    fn create_group_defaults_and_insertion() {
        let mut store = Store::new();
        let group = store
            .dispatch("groups/createGroup", json!({}))
            .expect("createGroup dispatches");

        assert_eq!(group["name"], json!("New Group"));
        assert_eq!(group["modules"], json!([]));
        assert_eq!(group["enabled"], json!(GROUP_DISABLED));
        assert_eq!(group["alpha"], json!(1));
        assert_eq!(group["hidden"], json!(false));
        assert_eq!(group["inherit"], json!(true));
        assert_eq!(group["inheritFrom"], json!(-1));
        assert_eq!(group["pipeline"], json!(false));
        assert_eq!(group["clearing"], json!(false));
        assert_eq!(group["compositeOperation"], json!("normal"));
        let id = group["id"].as_str().expect("id is a string");
        assert_eq!(id.len(), 36, "UUID v4 length");
        assert_eq!(store.state().groups.len(), 1);
        assert_eq!(store.state().groups[0].id, id);
    }

    #[test]
    fn create_group_accepts_arguments_and_keeps_unknown_keys() {
        let mut store = Store::new();
        let group = store
            .dispatch(
                "groups/createGroup",
                json!({
                    "id": "fixed-id",
                    "name": "Effects",
                    "enabled": 1,
                    "alpha": 0.5,
                    "inherit": false,
                    "inheritFrom": "other-group",
                    "pipeline": true,
                    "clearing": true,
                    "compositeOperation": "multiply",
                    "modules": ["m1"],
                    "reactToResize": false,
                    "writeToSwap": true,
                    "futureKey": {"nested": [1, 2]},
                }),
            )
            .expect("createGroup dispatches");

        assert_eq!(group["id"], json!("fixed-id"));
        assert_eq!(group["enabled"], json!(1));
        assert_eq!(group["alpha"], json!(0.5));
        assert_eq!(group["inherit"], json!(false));
        assert_eq!(group["inheritFrom"], json!("other-group"));
        assert_eq!(group["pipeline"], json!(true));
        assert_eq!(group["clearing"], json!(true));
        assert_eq!(group["compositeOperation"], json!("multiply"));
        assert_eq!(group["modules"], json!(["m1"]));
        // Action arguments are not Group properties; unknown keys survive.
        assert!(group.get("reactToResize").is_none());
        assert!(group.get("writeToSwap").is_none());
        assert_eq!(group["futureKey"], json!({"nested": [1, 2]}));
    }

    #[test]
    fn create_group_with_existing_id_returns_existing_group() {
        let mut store = Store::new();
        let first = store
            .dispatch("groups/createGroup", json!({"id": "g1", "name": "A"}))
            .expect("createGroup dispatches");
        let second = store
            .dispatch("groups/createGroup", json!({"id": "g1", "name": "B"}))
            .expect("createGroup dispatches");

        assert_eq!(first, second);
        assert_eq!(store.state().groups.len(), 1);
        assert_eq!(store.state().groups[0].name, "A");
    }

    #[test]
    fn update_group_by_key_sets_typed_fields() {
        let (mut store, id) = store_with_group("Group");
        for (key, value) in [
            ("name", json!("Renamed")),
            ("hidden", json!(true)),
            ("enabled", json!(1)),
            ("alpha", json!(0.25)),
            ("inherit", json!(false)),
            ("pipeline", json!(true)),
            ("clearing", json!(true)),
            ("compositeOperation", json!("lighter")),
        ] {
            store
                .dispatch(
                    "groups/UPDATE_GROUP_BY_KEY",
                    json!({"groupId": id, "key": key, "value": value}),
                )
                .expect("update dispatches");
        }

        let group = store.state().find_group(&id).expect("group exists");
        assert_eq!(group.name, "Renamed");
        assert!(group.hidden);
        assert_eq!(group.enabled, 1);
        assert_eq!(group.alpha, 0.25);
        assert!(!group.inherit);
        assert!(group.pipeline);
        assert!(group.clearing);
        assert_eq!(group.composite_operation, "lighter");
    }

    #[test]
    fn update_group_by_key_inherit_from_number_and_string() {
        let (mut store, id) = store_with_group("Group");
        store
            .dispatch(
                "groups/UPDATE_GROUP_BY_KEY",
                json!({"groupId": id, "key": "inheritFrom", "value": "group-b"}),
            )
            .expect("update dispatches");
        assert_eq!(
            store.state().find_group(&id).unwrap().inherit_from,
            InheritFrom::Group("group-b".to_string())
        );

        store
            .dispatch(
                "groups/UPDATE_GROUP_BY_KEY",
                json!({"groupId": id, "key": "inheritFrom", "data": -1}),
            )
            .expect("update dispatches");
        assert_eq!(
            store.state().find_group(&id).unwrap().inherit_from,
            InheritFrom::Previous(-1)
        );
    }

    #[test]
    fn update_group_by_key_unknown_key_goes_to_extra() {
        let (mut store, id) = store_with_group("Group");
        store
            .dispatch(
                "groups/UPDATE_GROUP_BY_KEY",
                json!({"groupId": id, "key": "futureThing", "value": {"a": 1}}),
            )
            .expect("update dispatches");

        let group = store.state().find_group(&id).unwrap();
        assert_eq!(group.extra.get("futureThing"), Some(&json!({"a": 1})));
        // The unknown key round-trips through the preset document.
        let saved = store.snapshot();
        assert_eq!(saved["groups"][0]["futureThing"], json!({"a": 1}));
    }

    #[test]
    fn update_group_by_key_missing_group_is_a_noop() {
        let mut store = Store::new();
        let result = store
            .dispatch(
                "groups/UPDATE_GROUP_BY_KEY",
                json!({"groupId": "nope", "key": "alpha", "value": 0.5}),
            )
            .expect("no-op dispatches");
        assert_eq!(result, Value::Null);
        assert_eq!(store.pending_delta_count(), 0);
    }

    #[test]
    fn add_and_remove_module_to_group() {
        let (mut store, id) = store_with_group("Group");
        store
            .dispatch(
                "groups/ADD_MODULE_TO_GROUP",
                json!({"groupId": id, "moduleId": "m1"}),
            )
            .expect("add dispatches");
        store
            .dispatch(
                "groups/ADD_MODULE_TO_GROUP",
                json!({"groupId": id, "moduleId": "m0", "position": 0}),
            )
            .expect("add dispatches");
        assert_eq!(
            store.state().find_group(&id).unwrap().modules,
            vec!["m0".to_string(), "m1".to_string()]
        );

        store
            .dispatch(
                "groups/REMOVE_MODULE_FROM_GROUP",
                json!({"groupId": id, "moduleId": "m1"}),
            )
            .expect("remove dispatches");
        assert_eq!(
            store.state().find_group(&id).unwrap().modules,
            vec!["m0".to_string()]
        );
    }

    #[test]
    fn remove_module_from_group_unknown_is_a_noop() {
        let (mut store, id) = store_with_group("Group");
        store.drain_deltas(); // discard the fixture's createGroup delta
        let result = store
            .dispatch(
                "groups/REMOVE_MODULE_FROM_GROUP",
                json!({"groupId": id, "moduleId": "missing"}),
            )
            .expect("no-op dispatches");
        assert_eq!(result, Value::Null);
        assert_eq!(store.pending_delta_count(), 0);
    }

    #[test]
    fn replace_groups_preserves_the_gallery_group() {
        let mut store = Store::new();
        store
            .dispatch(
                "groups/REPLACE_GROUPS",
                json!({"groups": [gallery_group()]}),
            )
            .expect("replace dispatches");
        store
            .dispatch(
                "groups/REPLACE_GROUPS",
                json!({"groups": [plain_group("a", "Effects"), plain_group("b", "Foreground")]}),
            )
            .expect("replace dispatches");

        let names: Vec<&str> = store
            .state()
            .groups
            .iter()
            .map(|group| group.name.as_str())
            .collect();
        assert_eq!(names, vec!["Effects", "Foreground", GALLERY_GROUP_NAME]);
    }

    #[test]
    fn replace_groups_without_an_existing_gallery_keeps_the_incoming_list() {
        let mut store = Store::new();
        store
            .dispatch(
                "groups/REPLACE_GROUPS",
                json!({"groups": [plain_group("a", "Effects")]}),
            )
            .expect("replace dispatches");
        assert_eq!(store.state().groups.len(), 1);
        assert_eq!(store.state().groups[0].name, "Effects");
    }

    #[test]
    fn replace_groups_delta_carries_the_final_list_including_the_gallery() {
        let mut store = Store::new();
        store
            .dispatch(
                "groups/createGroup",
                json!({"id": "gallery", "name": GALLERY_GROUP_NAME}),
            )
            .expect("createGroup dispatches");
        store.drain_deltas();

        store
            .dispatch(
                "groups/REPLACE_GROUPS",
                json!({"groups": [plain_group("a", "Effects")]}),
            )
            .expect("replace dispatches");

        let deltas: Vec<Value> = store
            .drain_deltas()
            .into_iter()
            .map(|mutation| serde_json::to_value(mutation).unwrap())
            .collect();
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0]["type"], json!("groups/REPLACE_GROUPS"));
        let groups = deltas[0]["payload"]["groups"].as_array().unwrap();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0]["id"], json!("a"));
        assert_eq!(groups[1]["name"], json!(GALLERY_GROUP_NAME));
    }

    #[test]
    fn make_active_module_known_uses_registry_props_and_meta() {
        let mut store = Store::new();
        let module = store
            .dispatch("modules/makeActiveModule", json!({"moduleName": "Bands"}))
            .expect("makeActiveModule dispatches");

        assert_eq!(module["$moduleName"], json!("Bands"));
        assert_eq!(module["meta"]["name"], json!("Bands"));
        assert_eq!(module["meta"]["author"], json!("modV"));
        assert_eq!(module["meta"]["type"], json!("shader"));
        assert_eq!(module["meta"]["category"], json!("2d-scenes"));
        assert_eq!(
            module["meta"]["audioFeatures"],
            json!(["bass", "mid", "treble"])
        );
        assert_eq!(module["meta"]["previewWithOutput"], json!(false));
        assert_eq!(module["meta"]["enabled"], json!(false));
        assert_eq!(module["meta"]["alpha"], json!(1));
        assert_eq!(module["meta"]["compositeOperation"], json!("normal"));
        assert_eq!(module["meta"]["isGallery"], json!(false));
        assert_eq!(module["props"]["u_ringWidth"], json!(0.05));
        assert_eq!(module["data"], json!({}));
        assert_eq!(module["$props"], json!({}));
        assert!(module["$status"].as_array().unwrap().is_empty());

        let id = module["$id"].as_str().expect("id");
        assert!(store.state().modules.contains_key(id));
    }

    #[test]
    fn make_active_module_two_d_uses_registry_meta_and_props() {
        let mut store = Store::new();
        let module = store
            .dispatch("modules/makeActiveModule", json!({"moduleName": "Ball"}))
            .expect("makeActiveModule dispatches");

        assert_eq!(module["$moduleName"], json!("Ball"));
        assert_eq!(module["meta"]["name"], json!("Ball"));
        assert_eq!(module["meta"]["type"], json!("2d"));
        assert_eq!(module["meta"]["author"], json!("2xAA"));
        assert_eq!(module["meta"]["version"], json!("1.0.1"));
        assert_eq!(module["meta"]["audioFeatures"], json!(["zcr", "rms"]));
        // Ball declares no category and does not set `previewWithOutput`.
        assert!(module["meta"].get("category").is_none());
        assert!(module["meta"].get("previewWithOutput").is_none());
        // The JS defaults and gallery flag still apply.
        assert_eq!(module["meta"]["alpha"], json!(1));
        assert_eq!(module["meta"]["enabled"], json!(false));
        assert_eq!(module["meta"]["compositeOperation"], json!("normal"));
        assert_eq!(module["meta"]["isGallery"], json!(false));

        // Prop defaults come from the 2D descriptors.
        assert_eq!(module["props"]["amount"], json!(10));
        assert_eq!(module["props"]["speed"], json!(2.0));
        assert_eq!(module["props"]["wrap"], json!(false));
        assert_eq!(module["props"]["size"], json!(2));
        assert!(module["props"]["color"]["data"].is_array());

        // `data` is empty: the module instance owns that state (documented).
        assert_eq!(module["data"], json!({}));
        assert_eq!(module["$props"], json!({}));
        assert!(module["$status"].as_array().unwrap().is_empty());
    }

    #[test]
    fn make_active_module_two_d_keeps_preview_with_output() {
        let mut store = Store::new();
        let module = store
            .dispatch(
                "modules/makeActiveModule",
                json!({"moduleName": "Pixelate"}),
            )
            .expect("makeActiveModule dispatches");
        assert_eq!(module["meta"]["type"], json!("2d"));
        assert_eq!(module["meta"]["category"], json!("effects"));
        assert_eq!(module["meta"]["previewWithOutput"], json!(true));

        // Caller meta still wins over the registry.
        let custom = store
            .dispatch(
                "modules/makeActiveModule",
                json!({"moduleName": "Pixelate", "moduleMeta": {"alpha": 0.25}}),
            )
            .expect("makeActiveModule dispatches");
        assert_eq!(custom["meta"]["alpha"], json!(0.25));
    }

    #[test]
    fn update_prop_coerces_two_d_strict_props() {
        let mut store = Store::new();
        let module = store
            .dispatch("modules/makeActiveModule", json!({"moduleName": "Polygon"}))
            .expect("makeActiveModule dispatches");
        let id = module["$id"].as_str().unwrap().to_string();

        // Polygon.js:29-37 sets `strict: true`, `min: 1`, `max: 20` on an int
        // prop: clamp then round (`modules.js:512-526`).
        store
            .dispatch(
                "modules/updateProp",
                json!({"moduleId": id, "prop": "strokeWeight", "data": {"value": 99}}),
            )
            .expect("updateProp dispatches");
        // The coercion pipeline stores numbers as `f64` values (`Number::Float`),
        // so compare with float literals.
        assert_eq!(
            store.state().modules[&id].props["strokeWeight"],
            json!(20.0)
        );

        store
            .dispatch(
                "modules/updateProp",
                json!({"moduleId": id, "prop": "strokeWeight", "data": {"value": -2.5}}),
            )
            .expect("updateProp dispatches");
        assert_eq!(store.state().modules[&id].props["strokeWeight"], json!(1.0));

        // The delta carries the 2D prop's data type.
        let deltas = store.drain_deltas();
        let last = deltas.last().expect("a delta was emitted");
        assert_eq!(last.r#type, "modules/UPDATE_ACTIVE_MODULE_PROP");
        assert_eq!(last.payload["data"]["type"], json!("int"));
    }

    #[test]
    fn set_bands_updates_the_stored_audio_frame() {
        let mut store = Store::new();
        assert_eq!(store.audio().bass, 0.0);

        store
            .dispatch(
                "meyda/SET_BANDS",
                json!({"bass": 0.5, "mid": 0.25, "treble": 0.125, "cells": [0.1]}),
            )
            .expect("meyda/SET_BANDS dispatches");

        assert_eq!(store.audio().bass, 0.5);
        assert_eq!(store.audio().mid, 0.25);
        assert_eq!(store.audio().treble, 0.125);
        // The renderer computed the bands, so no delta is echoed back.
        assert!(store.drain_deltas().is_empty());
    }

    #[test]
    fn make_active_module_unknown_sets_error_status() {
        let mut store = Store::new();
        let module = store
            .dispatch(
                "modules/makeActiveModule",
                json!({"moduleName": "Not A Module"}),
            )
            .expect("makeActiveModule dispatches");

        assert_eq!(module["$moduleName"], json!("Not A Module"));
        assert_eq!(module["meta"]["name"], json!("Not A Module"));
        assert_eq!(module["meta"]["alpha"], json!(1));
        assert_eq!(module["meta"]["enabled"], json!(false));
        assert_eq!(module["meta"]["compositeOperation"], json!("normal"));
        assert_eq!(module["props"], json!({}));
        let status = module["$status"].as_array().expect("status array");
        assert_eq!(status.len(), 1);
        assert_eq!(status[0]["type"], json!("error"));
        assert!(
            status[0]["message"]
                .as_str()
                .unwrap()
                .contains("is not registered")
        );
    }

    #[test]
    fn update_prop_stores_the_registry_default_shape() {
        let mut store = Store::new();
        let module = store
            .dispatch("modules/makeActiveModule", json!({"moduleName": "Wobble"}))
            .expect("makeActiveModule dispatches");
        let id = module["$id"].as_str().unwrap().to_string();

        store
            .dispatch(
                "modules/updateProp",
                json!({"moduleId": id, "prop": "strength", "data": {"value": 0.02}}),
            )
            .expect("updateProp dispatches");

        let module = &store.state().modules[&id];
        assert_eq!(module.props["strength"], json!(0.02));
    }

    #[test]
    fn update_prop_coerces_with_a_synthetic_strict_schema() {
        // No in-tree shader prop sets `strict`; pin the coercion call itself
        // with a synthetic schema so the pipeline is exercised.
        let mut schema = PropSchema {
            type_: "float".to_string(),
            min: Some(0.0),
            max: Some(1.0),
            strict: true,
            ..PropSchema::default()
        };
        assert_eq!(coerce_prop_value(&schema, &json!(42.0)), json!(1.0));
        assert_eq!(coerce_prop_value(&schema, &json!(-5.0)), json!(0.0));

        schema.type_ = "int".to_string();
        schema.strict = false;
        assert_eq!(coerce_prop_value(&schema, &json!(-2.5)), json!(-2.0));

        // The store applies the same pipeline for registry props (no strict):
        // values outside [min, max] pass through untouched.
        let mut store = Store::new();
        let module = store
            .dispatch("modules/makeActiveModule", json!({"moduleName": "Bands"}))
            .expect("makeActiveModule dispatches");
        let id = module["$id"].as_str().unwrap().to_string();
        store
            .dispatch(
                "modules/updateProp",
                json!({"moduleId": id, "prop": "u_ringWidth", "data": {"value": 9.0}}),
            )
            .expect("updateProp dispatches");
        assert_eq!(store.state().modules[&id].props["u_ringWidth"], json!(9.0));
    }

    #[test]
    fn update_prop_with_path_writes_the_nested_value() {
        let mut store = Store::new();
        let module = store
            .dispatch(
                "modules/makeActiveModule",
                json!({
                    "moduleName": "Not A Module",
                    "moduleMeta": {"name": "Custom"},
                }),
            )
            .expect("makeActiveModule dispatches");
        let id = module["$id"].as_str().unwrap().to_string();

        // The module has no registry schema; the fallback schema stores the
        // value as-is, and the path writes through a created array.
        store
            .dispatch(
                "modules/updateProp",
                json!({"moduleId": id, "prop": "colour", "data": {"value": 0.5, "path": "[1]"}}),
            )
            .expect("updateProp dispatches");

        assert_eq!(
            store.state().modules[&id].props["colour"],
            json!([null, 0.5])
        );
    }

    #[test]
    fn update_meta_sets_the_field() {
        let mut store = Store::new();
        let module = store
            .dispatch("modules/makeActiveModule", json!({"moduleName": "Bands"}))
            .expect("makeActiveModule dispatches");
        let id = module["$id"].as_str().unwrap().to_string();

        store
            .dispatch(
                "modules/updateMeta",
                json!({"id": id, "metaKey": "alpha", "data": 0.5}),
            )
            .expect("updateMeta dispatches");

        assert_eq!(store.state().modules[&id].meta["alpha"], json!(0.5));
    }

    #[test]
    fn update_active_module_sets_typed_keys() {
        let mut store = Store::new();
        let module = store
            .dispatch("modules/makeActiveModule", json!({"moduleName": "Bands"}))
            .expect("makeActiveModule dispatches");
        let id = module["$id"].as_str().unwrap().to_string();

        store
            .dispatch(
                "modules/updateActiveModule",
                json!({"id": id, "key": "data", "value": {"seed": 7}}),
            )
            .expect("updateActiveModule dispatches");
        store
            .dispatch(
                "modules/updateActiveModule",
                json!({"id": id, "key": "$status", "value": [{"type": "warn", "message": "hm"}]}),
            )
            .expect("updateActiveModule dispatches");

        let module = &store.state().modules[&id];
        assert_eq!(module.data, json!({"seed": 7}));
        assert_eq!(module.status.len(), 1);
    }

    #[test]
    fn remove_active_module_deletes_the_instance() {
        let mut store = Store::new();
        let module = store
            .dispatch("modules/makeActiveModule", json!({"moduleName": "Bands"}))
            .expect("makeActiveModule dispatches");
        let id = module["$id"].as_str().unwrap().to_string();

        store
            .dispatch("modules/removeActiveModule", json!({"moduleId": id}))
            .expect("remove dispatches");
        assert!(!store.state().modules.contains_key(&id));

        // Removing it again is a no-op, not an error.
        let result = store
            .dispatch("modules/removeActiveModule", json!({"moduleId": id}))
            .expect("second remove dispatches");
        assert_eq!(result, Value::Null);
    }

    #[test]
    fn every_mutation_pushes_one_delta_with_the_mirror_payload() {
        let mut store = Store::new();

        let group = store
            .dispatch("groups/createGroup", json!({"name": "Effects"}))
            .expect("createGroup dispatches");
        let group_id = group["id"].as_str().unwrap().to_string();
        let module = store
            .dispatch("modules/makeActiveModule", json!({"moduleName": "Bands"}))
            .expect("makeActiveModule dispatches");
        let module_id = module["$id"].as_str().unwrap().to_string();
        store
            .dispatch(
                "groups/ADD_MODULE_TO_GROUP",
                json!({"groupId": group_id, "moduleId": module_id}),
            )
            .expect("add dispatches");
        store
            .dispatch(
                "groups/UPDATE_GROUP_BY_KEY",
                json!({"groupId": group_id, "key": "alpha", "value": 0.5}),
            )
            .expect("update dispatches");
        store
            .dispatch(
                "modules/updateProp",
                json!({"moduleId": module_id, "prop": "u_ringWidth", "data": {"value": 0.2}}),
            )
            .expect("updateProp dispatches");
        store
            .dispatch(
                "modules/updateMeta",
                json!({"id": module_id, "metaKey": "alpha", "data": 1}),
            )
            .expect("updateMeta dispatches");
        store
            .dispatch(
                "modules/updateActiveModule",
                json!({"id": module_id, "key": "data", "value": {}}),
            )
            .expect("updateActiveModule dispatches");
        store
            .dispatch(
                "groups/REMOVE_MODULE_FROM_GROUP",
                json!({"groupId": group_id, "moduleId": module_id}),
            )
            .expect("remove-module dispatches");
        store
            .dispatch("modules/removeActiveModule", json!({"moduleId": module_id}))
            .expect("remove dispatches");

        let deltas: Vec<Value> = store
            .drain_deltas()
            .into_iter()
            .map(|mutation| serde_json::to_value(mutation).unwrap())
            .collect();

        assert_eq!(deltas.len(), 9);
        assert_eq!(deltas[0]["type"], json!("groups/ADD_GROUP"));
        assert_eq!(deltas[0]["payload"]["group"], group);
        assert_eq!(deltas[1]["type"], json!("modules/ADD_ACTIVE_MODULE"));
        assert_eq!(deltas[1]["payload"]["module"], module);
        assert_eq!(
            deltas[2],
            json!({
                "type": "groups/ADD_MODULE_TO_GROUP",
                "payload": {"moduleId": module_id, "groupId": group_id, "position": 0}
            })
        );
        assert_eq!(
            deltas[3],
            json!({
                "type": "groups/UPDATE_GROUP_BY_KEY",
                "payload": {"groupId": group_id, "key": "alpha", "data": 0.5}
            })
        );
        assert_eq!(
            deltas[4],
            json!({
                "type": "modules/UPDATE_ACTIVE_MODULE_PROP",
                "payload": {
                    "moduleId": module_id,
                    "prop": "u_ringWidth",
                    "data": {"value": 0.2, "type": "float", "path": ""}
                }
            })
        );
        assert_eq!(
            deltas[5],
            json!({
                "type": "modules/UPDATE_ACTIVE_MODULE_META",
                "payload": {"id": module_id, "metaKey": "alpha", "data": 1}
            })
        );
        assert_eq!(
            deltas[6],
            json!({
                "type": "modules/UPDATE_ACTIVE_MODULE",
                "payload": {"id": module_id, "key": "data", "value": {}}
            })
        );
        assert_eq!(deltas[7]["type"], json!("groups/REMOVE_MODULE_FROM_GROUP"));
        assert_eq!(deltas[8]["type"], json!("modules/REMOVE_ACTIVE_MODULE"));
        assert_eq!(deltas[8]["payload"], json!({"moduleId": module_id}));
    }

    #[test]
    fn unknown_action_returns_null_and_emits_nothing() {
        let mut store = Store::new();
        let result = store
            .dispatch("scenes/hydrateScenes", json!({"anything": true}))
            .expect("unknown actions are accepted");
        assert_eq!(result, Value::Null);
        assert!(store.drain_deltas().is_empty());
    }

    #[test]
    fn missing_required_field_is_a_structured_error() {
        let mut store = Store::new();
        let error = store
            .dispatch("groups/ADD_MODULE_TO_GROUP", json!({"groupId": "g1"}))
            .expect_err("missing moduleId must error");
        match error {
            StoreError::InvalidPayload { action, detail } => {
                assert_eq!(action, "groups/ADD_MODULE_TO_GROUP");
                assert!(detail.contains("moduleId"));
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn snapshot_has_six_keys_plus_version_and_render_order() {
        let mut store = Store::new();
        store
            .dispatch(
                "groups/createGroup",
                json!({"id": "a", "name": "Effects", "enabled": 1}),
            )
            .expect("createGroup dispatches");
        store
            .dispatch(
                "groups/createGroup",
                json!({"id": "gallery", "name": GALLERY_GROUP_NAME}),
            )
            .expect("createGroup dispatches");

        let snapshot = store.snapshot();
        for key in [
            "groups",
            "modules",
            "inputs",
            "expressions",
            "tweens",
            "midi",
            "version",
            "renderOrder",
        ] {
            assert!(snapshot.get(key).is_some(), "snapshot is missing key {key}");
        }
        assert_eq!(snapshot["version"], json!(1));
        // Gallery group is dropped from the preset, and the render order moves
        // it to the end of the live list: [a, gallery] -> [0, 1].
        assert_eq!(snapshot["groups"].as_array().unwrap().len(), 1);
        assert_eq!(snapshot["renderOrder"], json!([0, 1]));
    }

    #[test]
    fn snapshot_round_trips_through_the_preset_loader() {
        let mut store = Store::new();
        store
            .dispatch(
                "groups/createGroup",
                json!({"id": "a", "name": "Effects", "enabled": 1, "clearing": true}),
            )
            .expect("createGroup dispatches");
        let module = store
            .dispatch("modules/makeActiveModule", json!({"moduleName": "Bands"}))
            .expect("makeActiveModule dispatches");
        let module_id = module["$id"].as_str().unwrap().to_string();
        store
            .dispatch(
                "groups/ADD_MODULE_TO_GROUP",
                json!({"groupId": "a", "moduleId": module_id}),
            )
            .expect("add dispatches");
        store
            .dispatch(
                "modules/updateProp",
                json!({"moduleId": module_id, "prop": "u_ringWidth", "data": {"value": 0.3}}),
            )
            .expect("updateProp dispatches");

        let snapshot = store.snapshot();
        let mut restored = Store::new();
        restored
            .restore(&snapshot)
            .expect("the engine snapshot loads");

        assert_eq!(restored.state().groups, store.state().groups);
        assert_eq!(restored.state().modules, store.state().modules);
        assert_eq!(
            restored.state().modules[&module_id].props["u_ringWidth"],
            json!(0.3)
        );
    }

    #[test]
    fn snapshot_with_engine_sidecar_round_trips_and_is_ignored_by_the_loader() {
        let mut store = Store::new();
        store
            .dispatch("groups/createGroup", json!({"id": "a", "name": "Effects"}))
            .expect("createGroup dispatches");

        let snapshot = store.snapshot_with_engine(42);
        assert_eq!(snapshot[ENGINE_SIDECAR_KEY], json!({"framesPresented": 42}));
        assert_eq!(frames_presented_in(&snapshot), Some(42));

        // The sidecar is not a preset key: loading it leaves the state intact
        // and the loader treats it as (ignored) metadata.
        let mut restored = Store::new();
        restored.restore(&snapshot).expect("snapshot loads");
        assert_eq!(restored.state().groups.len(), 1);
        assert!(restored.snapshot().get(ENGINE_SIDECAR_KEY).is_none());
    }

    #[test]
    fn restore_replaces_the_state_and_drops_pending_deltas() {
        let mut store = Store::new();
        store
            .dispatch("groups/createGroup", json!({"id": "old", "name": "Old"}))
            .expect("createGroup dispatches");
        assert!(store.pending_delta_count() > 0);

        store
            .restore(&json!({"groups": [plain_group("new", "New")]}))
            .expect("restore loads");
        assert_eq!(store.state().groups.len(), 1);
        assert_eq!(store.state().groups[0].id, "new");
        assert_eq!(store.pending_delta_count(), 0);
    }

    #[test]
    fn prop_path_writer_handles_lodash_style_paths() {
        let mut value = json!({"a": [1, 2], "b": {"c": 0}});
        set_json_path(&mut value, "b.c", json!(5));
        assert_eq!(value["b"]["c"], json!(5));

        set_json_path(&mut value, "a[1]", json!(9));
        assert_eq!(value["a"], json!([1, 9]));

        let mut colour = json!({"r": 0, "g": 0, "b": 0, "a": 0});
        set_json_path(&mut colour, "[g]", json!(1.0));
        assert_eq!(colour["g"], json!(1.0));

        let mut nested = Value::Null;
        set_json_path(&mut nested, "[0].x", json!(true));
        assert_eq!(nested, json!([{"x": true}]));
    }

    #[test]
    fn module_registry_schema_for_bands_ring_width() {
        let registry = ModuleRegistry::builtin();
        let schema = registry
            .prop_schema("Bands", "u_ringWidth")
            .expect("Bands declares u_ringWidth");
        assert_eq!(schema.type_, "float");
        assert_eq!(schema.min, Some(0.01));
        assert_eq!(schema.max, Some(0.5));
        assert!(!schema.strict);
        assert_eq!(schema.default, Some(json!(0.05)));
        assert!(registry.prop_schema("Bands", "nope").is_none());
        assert!(registry.prop_schema("Nope", "u_ringWidth").is_none());
    }
}
