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
//!   them (`modules.js:253-268`). Shader (`meta.type: "shader"`), 2D
//!   (`meta.type: "2d"`) and Point10 (also `meta.type: "2d"`, the JS wrapper
//!   meta) modules are registered (M3/M4, see [`crate::module_registry`]);
//!   the compositor picks the render path from the registry kind.
//! * `modules/updateProp` — `modules.js:481-562`, with the coercion pipeline in
//!   [`modv_engine::props::coerce_prop_value`] fed by the same registry for
//!   every module kind. Point10 props come from the scene's
//!   [`modv_renderers::point10::SceneProp`] schema, whose numeric bounds are
//!   strict (the scene binder clamps on read, so clamping on write is the same
//!   stored value).
//! * `modules/updateMeta` / `modules/updateActiveModule` /
//!   `modules/removeActiveModule` — `modules.js:564-587`, `:858-888`, `:715`.
//! * `meyda/SET_BANDS` — the renderer's audio pump commits the AGC bands
//!   (`meyda.js:88-96`); the engine stores them in [`Store::audio`] for the
//!   compositor. When the engine owns capture (`--audio-source
//!   pipewire|input`, WS-9) the renderer's frames are ignored instead —
//!   [`Store::set_engine_audio`] is then the authoritative writer.
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
//!   [`crate::scene_ops`] (WS-8 / M5b) and the performance runtime in
//!   [`crate::performance`] (M5c); Scene hover previews remain M5d.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

use modv_engine::delta::{DeltaStream, Mutation};
use modv_engine::engine_state::{
    EngineState, GALLERY_GROUP_NAME, GROUP_DISABLED, Group, InheritFrom, ModuleInstance,
};
use modv_engine::inputs::{Input, InputLink};
use modv_engine::preset::{
    ExpressionAssignment, PresetError, create_preset_from_state, load_preset_into_state,
};
use modv_engine::props::{PropSchema, coerce_prop_value};
use modv_media::audio::analyser::AudioSnapshot;
use modv_renderers::two_d::AudioFrame;

use crate::expressions::{ExpressionCache, ExpressionFailure, resolve_input_value};
use crate::input_links::{
    BeatsState, LINK_DELTA_EPSILON, LINK_DELTA_INTERVAL, LinkTarget, LinkWrite,
};
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
    /// The module registry, shared with the compositor (which needs the same
    /// ISF definitions to compile a shader on first draw).
    pub(crate) registry: Arc<ModuleRegistry>,
    /// The Scene list, performance config and seeding metadata (WS-8 / M5b).
    pub(crate) scenes: crate::scene_ops::ScenesState,
    /// `<state parent>/scenes.json`, resolved from `--state` on construction.
    pub(crate) scenes_path: Option<std::path::PathBuf>,
    /// The Scene this store last applied (`applySceneNow`'s active Scene).
    pub(crate) applied_scene_id: Option<String>,
    /// Monotonic counter bumped by every mutation of the Scene list or its row
    /// mapping. Scene hover-preview caches compare it so an edited Scene never
    /// shows a stale frame.
    scene_list_revision: u64,
    /// Performance-mode runtime: per-row dwells, row crossfades and the
    /// whole-scene fade (`scenes.js` module-level `rowRuntime`,
    /// `rowTransitions` and `transition`).
    pub(crate) performance: crate::performance::PerformanceRuntime,
    /// The shader effect a just-started whole-scene transition needs the
    /// window loop to capture (`scenes/applyScene` → `Compositor::
    /// capture_transition_source`). Cleared on capture, completion, fallback,
    /// reset or a later change.
    pub(crate) pending_transition_capture: Option<crate::scene_transition::TransitionEffect>,
    /// Tween runtime (WS-12): descriptors from `state.tweens` pre-rendered at
    /// [`Store::fps`] and the current BPM. Input Links read and advance them.
    pub(crate) tweens: modv_engine::tweens::TweenSet,
    /// Tween frame rate (`fps/SET_FPS`; the JS `store.state.fps.fps`).
    pub(crate) fps: f64,
    /// The current audio frame handed to modules and shader uniforms.
    ///
    /// Filled by `meyda/SET_BANDS` from the renderer's audio pump (see
    /// [`Store::set_bands`]), or by [`Store::set_engine_audio`] when the engine
    /// owns capture (WS-9). Either way the engine stores the full payload —
    /// bands, damping lanes, the 16-cell grid, wave, FFT magnitudes, sample
    /// rate and the Meyda scalars.
    audio: AudioFrame,
    /// Whether the engine owns audio capture/analysis (WS-9).
    ///
    /// While set, the renderer's `meyda/SET_BANDS`, `beats/SET_BPM` and
    /// `beats/SET_KICK` writes are ignored (logged once) so the engine's
    /// analyser stays authoritative and the mirror is not churned with
    /// conflicting tempo/meter writes. Cleared by
    /// [`Store::disable_engine_audio`] when a capture feed fails and the
    /// renderer takes over again.
    engine_audio_active: bool,
    /// Throttled mirror publish for engine-owned audio (≤ 10 Hz).
    engine_meter: EngineMeter,
    /// The tempo/kick state the Input Link evaluator reads (`beats.bpm`,
    /// `beats.kick`).
    beats: BeatsState,
    /// Per-smoothing-id state for `meyda/getFeature` smoothing links
    /// (`audio-features.js` `smoothedFeatures`), keyed by `args[1]`.
    link_smoothing: HashMap<String, f64>,
    /// Delta-throttle state for Input Link writes, keyed by input id.
    link_deltas: HashMap<String, LinkDeltaState>,
    /// Compiled JS expressions (WS-12), one per input, rebuilt when the
    /// preset/Scene or the `expressions/*` actions change the assignments.
    expression_cache: ExpressionCache,
    pub(crate) warned: HashSet<String>,
}

/// The newest value an Input Link wrote and when its delta was last sent.
#[derive(Clone, Debug)]
struct LinkDeltaState {
    /// Timestamp (`Store::update_input_links`'s `now`) of the last delta.
    last_emit: Option<Duration>,
    /// The value carried by that delta.
    last_value: f64,
    /// The newest un-emitted delta payload (with its mutation type, flushed on
    /// quit).
    pending: Option<(&'static str, Value)>,
}

/// Mirror-meter publish state for engine-owned audio (WS-9).
///
/// The renderer's UI meter reads `meyda/SET_BANDS` + `beats/*` from its mirror
/// store. When the engine owns capture it must publish those back, but not at
/// 60 Hz — one `meyda/SET_BANDS` per [`LINK_DELTA_INTERVAL`] keeps the meters
/// live at ~10 Hz, with the newest payload flushed on quit.
#[derive(Clone, Debug, Default)]
struct EngineMeter {
    /// Timestamp (`Store::set_engine_audio`'s `now`) of the last publish.
    last_emit: Option<Duration>,
    /// The newest un-emitted meter payload (flushed on quit).
    pending: Option<Value>,
}

impl Default for Store {
    fn default() -> Self {
        Self::new()
    }
}

impl Store {
    /// An empty store with the built-in module registry and no Scene storage.
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
        Self::with_state_path_and_isf(state_path, None)
    }

    /// [`Self::with_state_path`] plus the ISF corpus directory.
    ///
    /// [`ModuleRegistry::build`] scans `isf_dir` once; unreadable or
    /// unparsable shaders are skipped and logged. `None` registers no ISF
    /// module, matching [`Store::with_state_path`].
    pub fn with_state_path_and_isf(
        state_path: Option<&std::path::Path>,
        isf_dir: Option<&std::path::Path>,
    ) -> Self {
        let scenes_path = state_path
            .and_then(std::path::Path::parent)
            .map(|parent| parent.join(crate::scene_ops::SCENES_STORAGE_FILENAME));
        // Seed the performance picks from the wall clock, matching the JS
        // `Math.random()` call sites. Tests replace this with a fixed seed.
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos() as u64)
            .unwrap_or(0);
        let mut performance = crate::performance::PerformanceRuntime::default();
        performance.rng = crate::performance::Rng::seeded(seed);
        Self {
            state: EngineState::new(),
            deltas: DeltaStream::new(),
            registry: Arc::new(ModuleRegistry::build(isf_dir)),
            scenes: crate::scene_ops::ScenesState::default(),
            scenes_path,
            applied_scene_id: None,
            scene_list_revision: 0,
            performance,
            pending_transition_capture: None,
            tweens: modv_engine::tweens::TweenSet::new(),
            fps: 60.0,
            audio: AudioFrame::default(),
            engine_audio_active: false,
            engine_meter: EngineMeter::default(),
            beats: BeatsState::default(),
            link_smoothing: HashMap::new(),
            link_deltas: HashMap::new(),
            expression_cache: ExpressionCache::new(),
            warned: HashSet::new(),
        }
    }

    /// A shared handle to the registry, for the compositor to build against the
    /// same module definitions (including the scanned ISF corpus).
    pub fn registry_arc(&self) -> Arc<ModuleRegistry> {
        Arc::clone(&self.registry)
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

    /// Revision of the Scene list and its row mapping.
    ///
    /// Bumped by every Scene mutation (hydrate, seed, import, remove, load,
    /// capture, row rename). [`crate::scene_preview`] stores it with each
    /// cached hover preview so an edited Scene rebuilds instead of serving a
    /// stale frame.
    pub fn scene_list_revision(&self) -> u64 {
        self.scene_list_revision
    }

    /// Push one delta to the UI mirror (status metrics, resize events).
    pub(crate) fn push_delta(&mut self, type_: &str, payload: Value) {
        self.deltas.push(type_, payload);
    }

    /// Mark the Scene list (or the row-name mapping that selects its rows) as
    /// changed.
    pub(crate) fn touch_scene_list(&mut self) {
        self.scene_list_revision = self.scene_list_revision.wrapping_add(1);
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
            // ————————————————— Beats (WS-9) —————————————————
            "beats/SET_BPM" => self.set_bpm(params),
            "beats/setBpm" => self.set_bpm_action(params),
            "beats/SET_KICK" => self.set_kick(params),
            // ————————————————— FPS / tweens (WS-12) —————————————————
            "fps/SET_FPS" => self.set_fps(params),
            "tweens/ADD_TWEEN" => self.add_tween(params),
            "tweens/REMOVE_TWEEN" => self.remove_tween_action(params),
            "tweens/createTween" => self.create_tween(params),
            // ————————————————— Expressions (WS-12) —————————————————
            "expressions/create" => self.create_expression(params),
            "expressions/update" => self.update_expression(params),
            "expressions/updateEvaluation" => self.update_expression_evaluation(params),
            "expressions/remove" => self.remove_expression(params),
            // ————————————————— Inputs (WS-1b) —————————————————
            "inputs/addInput" => self.add_input(params),
            "inputs/removeInput" => self.remove_input_action(params),
            "inputs/createInputLink" => self.create_input_link(params),
            "inputs/updateInputLink" => self.update_input_link(params),
            "inputs/removeInputLink" => self.remove_input_link(params),
            "inputs/setFocusedInput" => self.set_focused_input(params),
            "inputs/clearFocusedInput" => self.clear_focused_input(),
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
            // ————————————————— Performance runtime (WS-8 / M5c) ————————
            "scenes/startPerformance" => self.start_performance(),
            "scenes/stopPerformance" => self.stop_performance(),
            "scenes/tick" => {
                let delta = params.get("delta").and_then(Value::as_f64).unwrap_or(0.0);
                self.tick_scenes(delta)
            }
            "scenes/applySceneRow" => {
                let Some(scene_id) = params.get("sceneId").and_then(Value::as_str) else {
                    return Ok(Value::Null);
                };
                let Some(row) = params
                    .get("rowIndex")
                    .or_else(|| params.get("row"))
                    .and_then(Value::as_u64)
                else {
                    return Ok(Value::Null);
                };
                let outcome = self.apply_scene_row_transition(scene_id, row as usize)?;
                Ok(outcome
                    .map(|outcome| {
                        json!({
                            "sceneId": scene_id,
                            "rowIndex": row,
                            "groupId": outcome.group_id,
                            "ghostGroupId": outcome.ghost_group_id,
                        })
                    })
                    .unwrap_or(Value::Null))
            }
            "scenes/nextScene" => {
                let scene_id = self.pick_next_whole_scene();
                match scene_id {
                    Some(scene_id) => {
                        let mut params = Map::new();
                        params.insert("sceneId".to_string(), json!(scene_id));
                        self.apply_scene_action(&params)
                    }
                    None => Ok(Value::Null),
                }
            }
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

        // WS-12: `applyExpression` runs before the coercion/clamp block in
        // `modules.js:510-526`, keyed by the prop's input id.
        let input_id = self.module_prop_input_id(&module_id, &prop);
        let value = match input_id {
            Some(input_id) => self.apply_expression(&input_id, value),
            None => value,
        };

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

        // WS-12: `modules/updateMeta` calls `applyExpression` with the meta
        // field's input id (`modules.js:577-579`).
        let input_id = self.module_meta_input_id(&id, &meta_key);
        let data = match input_id {
            Some(input_id) => self.apply_expression(&input_id, data),
            None => data,
        };

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

    /// `meyda/SET_BANDS` (WS-9): the renderer's audio pump commits the full
    /// analyser payload over the control socket.
    ///
    /// Accepted keys (camelCase wire names; unknown keys are ignored):
    ///
    /// | key | shape |
    /// |-----|-------|
    /// | `bass`/`mid`/`treble` | finite number |
    /// | `beat` | finite number |
    /// | `lanes`/`rawLanes` | exactly 12 finite numbers |
    /// | `cells`/`rawCells` | exactly 16 finite numbers |
    /// | `wave` | up to 1024 finite numbers (-1..1) |
    /// | `freqMag` | up to 1024 finite numbers |
    /// | `byteFrequencyData` | up to 1024 numbers, clamped 0..255 |
    /// | `byteTimeDomainData` | up to 1024 numbers; derives `wave` when no wave is sent |
    /// | `sampleRate` | finite positive number, fallback 44 100 |
    /// | Meyda scalars | `rms`, `zcr`, `energy`, `spectralCentroid`, `spectralFlatness`, `spectralSlope`, `spectralRolloff`, `spectralSpread`, `spectralSkewness`, `spectralKurtosis`, `perceptualSpread`, `perceptualSharpness` |
    ///
    /// A present-but-invalid number (`NaN`/`±∞`) becomes `0`; a wrong-length
    /// `lanes`/`cells` array is ignored entirely so a malformed frame cannot
    /// corrupt the live grid. Arrays shorter than their target are padded with
    /// zeros; longer ones are truncated. No delta is emitted — the renderer's
    /// mirror computed these values itself.
    ///
    /// While [`Store::engine_audio_active`] is set (the engine owns capture,
    /// WS-9) the frame is ignored: the engine's analyser is authoritative, and
    /// letting the renderer's (usually silent or absent) graph overwrite it
    /// would stall every audio-reactive module. The first ignored frame logs
    /// once so a misconfigured deployment is visible in the engine log.
    fn set_bands(&mut self, params: &Map<String, Value>) -> Result<Value, StoreError> {
        if self.reject_renderer_audio_write("meyda/SET_BANDS") {
            return Ok(Value::Null);
        }

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
        if let Some(beat) = scalar_field(bands.get("beat")) {
            self.audio.beat = beat;
        }

        if let Some(lanes) = fixed_f32_array::<12>(bands.get("lanes")) {
            self.audio.lanes = lanes;
        }
        if let Some(cells) = fixed_f32_array::<16>(bands.get("cells")) {
            self.audio.cells = cells;
        }
        // The `raw` twin has no separate analyser frame in the wire payload
        // today; the pump forwards `features.p10.raw.lanes/cells` explicitly.
        if let Some(raw_lanes) = fixed_f32_array::<12>(bands.get("rawLanes")) {
            self.audio.raw_lanes = raw_lanes;
        } else if let Some(lanes) = fixed_f32_array::<12>(bands.get("lanes")) {
            self.audio.raw_lanes = lanes;
        }
        if let Some(raw_cells) = fixed_f32_array::<16>(bands.get("rawCells")) {
            self.audio.raw_cells = raw_cells;
        } else if let Some(cells) = fixed_f32_array::<16>(bands.get("cells")) {
            self.audio.raw_cells = cells;
        }

        let wave_changed = if let Some(wave) = padded_f32_array::<1024>(bands.get("wave")) {
            self.audio.wave = wave;
            true
        } else if let Some(byte_wave) = padded_u8_array::<1024>(bands.get("byteTimeDomainData")) {
            // The analyser's time-domain byte stream maps 0..255 to -1..1,
            // exactly like `audio-bands.js` `_fillWave`.
            for (sample, byte) in self.audio.wave.iter_mut().zip(byte_wave) {
                *sample = (f32::from(byte) / 255.0) * 2.0 - 1.0;
            }
            true
        } else {
            false
        };
        if wave_changed {
            self.audio.features.buffer = self.audio.wave.to_vec();
        }

        if let Some(freq_mag) = padded_f32_array::<1024>(bands.get("freqMag")) {
            self.audio.freq_mag = freq_mag;
        }
        if let Some(frequency) = padded_u8_array::<1024>(bands.get("byteFrequencyData")) {
            self.audio.byte_frequency_data = frequency;
        }

        if let Some(sample_rate) = scalar_field(bands.get("sampleRate")) {
            if sample_rate > 0.0 {
                self.audio.sample_rate = sample_rate;
            }
        }

        let features = &mut self.audio.features;
        if let Some(value) = scalar_field(bands.get("rms")) {
            features.rms = value;
        }
        if let Some(value) = scalar_field(bands.get("zcr")) {
            features.zcr = value;
        }
        if let Some(value) = scalar_field(bands.get("energy")) {
            features.energy = value;
        }
        if let Some(value) = scalar_field(bands.get("spectralCentroid")) {
            features.spectral_centroid = value;
        }
        if let Some(value) = scalar_field(bands.get("spectralFlatness")) {
            features.spectral_flatness = value;
        }
        if let Some(value) = scalar_field(bands.get("spectralSlope")) {
            features.spectral_slope = value;
        }
        if let Some(value) = scalar_field(bands.get("spectralRolloff")) {
            features.spectral_rolloff = value;
        }
        if let Some(value) = scalar_field(bands.get("spectralSpread")) {
            features.spectral_spread = value;
        }
        if let Some(value) = scalar_field(bands.get("spectralSkewness")) {
            features.spectral_skewness = value;
        }
        if let Some(value) = scalar_field(bands.get("spectralKurtosis")) {
            features.spectral_kurtosis = value;
        }
        if let Some(value) = scalar_field(bands.get("perceptualSpread")) {
            features.perceptual_spread = value;
        }
        if let Some(value) = scalar_field(bands.get("perceptualSharpness")) {
            features.perceptual_sharpness = value;
        }

        Ok(Value::Null)
    }

    /// `beats/SET_BPM`: store the reported tempo (echoed to the mirror).
    ///
    /// Ignored while the engine owns capture ([`Store::set_engine_audio`] is
    /// then the tempo source), preferring a silent no-op over an ever-changing
    /// tempo that would rebuild every BPM tween each frame.
    ///
    /// A non-positive tempo is a no-op like any other invalid parameter: a BPM
    /// tween's duration is `(3600 / bpm) * division`, so `0` would be `Inf`
    /// and corrupt every descriptor in `state.tweens` (R-5). The tween clock
    /// keeps its 120 BPM fallback instead (`effective_duration`).
    fn set_bpm(&mut self, params: &Map<String, Value>) -> Result<Value, StoreError> {
        if self.reject_renderer_audio_write("beats/SET_BPM") {
            return Ok(Value::Null);
        }

        let bpm = params
            .get("bpm")
            .and_then(Value::as_f64)
            .filter(|bpm| bpm.is_finite() && *bpm > 0.0);
        if let Some(bpm) = bpm {
            self.store_bpm(bpm);
        }
        Ok(Value::Null)
    }

    /// `beats/setBpm` (`beats.js:9-19`): only the active source may set BPM.
    ///
    /// The engine keeps `beatdetektor` as the default source; the renderer
    /// only dispatches this action from that detector today.
    fn set_bpm_action(&mut self, params: &Map<String, Value>) -> Result<Value, StoreError> {
        let source = params.get("source").and_then(Value::as_str);
        if source.is_some_and(|source| source != "beatdetektor") {
            return Ok(Value::Null);
        }
        self.set_bpm(params)
    }

    /// `beats/SET_KICK`: store the kick flag (echoed to the mirror).
    ///
    /// Ignored while the engine owns capture, like [`Self::set_bpm`].
    fn set_kick(&mut self, params: &Map<String, Value>) -> Result<Value, StoreError> {
        if self.reject_renderer_audio_write("beats/SET_KICK") {
            return Ok(Value::Null);
        }

        let Some(kick) = params.get("kick").map(|value| bool_value(value, false)) else {
            return Ok(Value::Null);
        };
        self.store_kick(kick);
        Ok(Value::Null)
    }

    /// Store a tempo, rebuilding BPM tween durations and echoing one delta
    /// (only when it actually moved). Returns whether the tempo changed.
    ///
    /// Non-finite/non-positive values are rejected, so an analyser without a
    /// confident estimate can pass `None`/garbage (or the BeatDetektor's `0`)
    /// without corrupting the tween clock: `3600 / 0` is `Inf`, which no
    /// descriptor can serialise (R-5).
    fn store_bpm(&mut self, bpm: f64) -> bool {
        if !bpm.is_finite() || bpm <= 0.0 || (self.beats.bpm - bpm).abs() <= f64::EPSILON {
            return false;
        }

        self.beats.bpm = bpm;
        self.update_tween_bpm(bpm);
        self.deltas.push("beats/SET_BPM", json!({ "bpm": bpm }));
        true
    }

    /// Store the kick flag, echoing one delta when it flips. Returns whether
    /// the flag changed.
    fn store_kick(&mut self, kick: bool) -> bool {
        if self.beats.kick == kick {
            return false;
        }

        self.beats.kick = kick;
        self.deltas.push("beats/SET_KICK", json!({ "kick": kick }));
        true
    }

    /// Whether a renderer audio write must be dropped because the engine owns
    /// capture. Logs the first ignored write per mutation type (no allocation
    /// once the warning has fired, so a 60 Hz renderer pump stays cheap).
    fn reject_renderer_audio_write(&mut self, action: &'static str) -> bool {
        if !self.engine_audio_active {
            return false;
        }

        if !self.warned.contains(action) {
            self.warned.insert(action.to_string());
            eprintln!(
                "engine: WARN ignoring the renderer's {action} while engine audio capture owns the analysis"
            );
        }

        true
    }

    /// Apply one engine-owned analysis frame (WS-9 capture end state).
    ///
    /// This is the authoritative write when `--audio-source` is not
    /// `renderer`: the window loop pumps the [`crate::audio::open_feed`] feed
    /// each frame and calls this before Input Links are evaluated, so the
    /// compositor, props and links see the engine's analysis in the same
    /// frame. The call:
    ///
    /// 1. maps the [`AudioSnapshot`] onto [`Store::audio`]
    ///    ([`crate::audio::snapshot_to_frame`]) and marks the engine as the
    ///    audio owner (renderer writes are dropped from then on);
    /// 2. publishes the analyser's confident tempo through
    ///    [`Self::store_bpm`] (deltas only on change, so tween durations are
    ///    rebuilt once per tempo change rather than per frame);
    /// 3. publishes the kick flag through [`Self::store_kick`] (delta only on
    ///    a flip);
    /// 4. throttles `meyda/SET_BANDS` to the mirror so the UI meters keep
    ///    moving without one control-socket write per frame.
    ///
    /// `kick` is the BeatDetektor flag from
    /// [`modv_media::audio::analyser::AudioAnalyser::kick`] — not derived from
    /// the beat envelope — and `now` is the loop's monotonic clock for the
    /// meter throttle.
    pub fn set_engine_audio(&mut self, snapshot: &AudioSnapshot, kick: bool, now: Duration) {
        self.engine_audio_active = true;
        self.audio = crate::audio::snapshot_to_frame(snapshot);

        if let Some(bpm) = snapshot.bpm {
            self.store_bpm(f64::from(bpm));
        }
        self.store_kick(kick);
        self.publish_engine_meter(now);
    }

    /// Hand audio ownership back to the renderer's pump.
    ///
    /// The window loop calls this when a capture feed errors out: the last
    /// engine frame stays in [`Store::audio`] (renderers keep drawing) until
    /// the renderer's next `meyda/SET_BANDS` replaces it.
    pub fn disable_engine_audio(&mut self) {
        if !self.engine_audio_active {
            return;
        }

        self.engine_audio_active = false;
        self.engine_meter.pending = None;
        eprintln!("engine: INFO audio ownership returned to the renderer's pump");
    }

    /// Whether the engine owns audio capture/analysis (WS-9).
    pub fn engine_audio_active(&self) -> bool {
        self.engine_audio_active
    }

    /// Publish the current bands to the UI mirror at ≤ 10 Hz (WS-9).
    ///
    /// The payload is exactly what the mirror's `SET_BANDS` mutation reads
    /// (`{ bass, mid, treble, cells }`), matching the renderer pump's own
    /// commit, so the meters read the same shape from either source.
    fn publish_engine_meter(&mut self, now: Duration) {
        let payload = json!({
            "bass": self.audio.bass,
            "mid": self.audio.mid,
            "treble": self.audio.treble,
            "cells": self.audio.cells.to_vec(),
        });

        let due = self
            .engine_meter
            .last_emit
            .is_none_or(|last| now.saturating_sub(last) >= LINK_DELTA_INTERVAL);
        if due {
            self.engine_meter.last_emit = Some(now);
            self.engine_meter.pending = None;
            self.deltas.push("meyda/SET_BANDS", payload);
        } else {
            self.engine_meter.pending = Some(payload);
        }
    }

    /// Current tempo in BPM (0 until the beat detector reports one).
    pub fn bpm(&self) -> f64 {
        self.beats.bpm
    }

    /// The Input Link view of the kick: the explicit `beats/SET_KICK` flag, or
    /// the analyser's kick envelope when no explicit flag has arrived yet.
    pub fn kick(&self) -> bool {
        self.beats.kick || self.audio.beat >= 0.5
    }

    /// The beat state an evaluator reads.
    pub fn beats(&self) -> BeatsState {
        BeatsState {
            bpm: self.bpm(),
            kick: self.kick(),
        }
    }

    // ————————————————— Inputs (WS-1b) —————————————————

    /// `inputs/addInput` (`inputs.js:106-113`) / the `inputs/ADD_INPUT`
    /// mutation.
    ///
    /// Accepts either the action payload (`{ type, getLocation, location,
    /// data }`) or a mutation-shaped `{ input: {...} }`. The UI omits `id`
    /// (the Vuex action generates one with `uuidv4`), so the engine does the
    /// same and returns the stored record.
    fn add_input(&mut self, params: &Map<String, Value>) -> Result<Value, StoreError> {
        const ACTION: &str = "inputs/addInput";
        let payload = params
            .get("input")
            .and_then(Value::as_object)
            .unwrap_or(params);
        let mut input = payload.clone();
        input
            .entry("id".to_string())
            .or_insert_with(|| json!(crate::ids::uuid_v4()));
        input
            .entry("type".to_string())
            .or_insert_with(|| json!("action"));
        input
            .entry("data".to_string())
            .or_insert_with(|| Value::Null);

        let record: Input = serde_json::from_value(Value::Object(input))
            .map_err(|error| invalid(ACTION, format!("invalid input record: {error}")))?;
        let value = serde_json::to_value(&record)
            .map_err(|error| invalid(ACTION, format!("input record: {error}")))?;
        let id = record.id.clone();
        self.state.inputs.insert(id, record);
        self.deltas
            .push("inputs/ADD_INPUT", json!({ "input": value }));
        Ok(value)
    }

    /// `inputs/removeInput` (`inputs.js:116-134`).
    fn remove_input_action(&mut self, params: &Map<String, Value>) -> Result<Value, StoreError> {
        let Some(input_id) = params.get("inputId").and_then(Value::as_str) else {
            return Ok(json!(false));
        };
        if self.state.inputs.shift_remove(input_id).is_none() {
            if !bool_field(params, "silent").unwrap_or(false) {
                self.warn_once(
                    format!("input:{input_id}"),
                    &format!(
                        "engine: WARN inputs/removeInput: no input with id {input_id} (ignored)"
                    ),
                );
            }
            return Ok(json!(false));
        }
        self.deltas
            .push("inputs/REMOVE_INPUT", json!({ "inputId": input_id }));
        Ok(json!(true))
    }

    /// `inputs/createInputLink` (`inputs.js:136-176`).
    ///
    /// JS defaults `min`/`max` to `0`/`1` in the action's destructuring, so an
    /// absent bound is written as `0`/`1`; an explicit `null` (OSC links)
    /// stays `null` and disables the range mapping.
    fn create_input_link(&mut self, params: &Map<String, Value>) -> Result<Value, StoreError> {
        const ACTION: &str = "inputs/createInputLink";
        let Some(input_id) = params.get("inputId").and_then(Value::as_str) else {
            return Ok(json!(false));
        };
        let Some(source) = params.get("source").and_then(Value::as_str) else {
            self.warn_once(
                format!("input-link-source:{input_id}"),
                &format!("engine: WARN {ACTION}: no source for input {input_id} (link ignored)"),
            );
            return Ok(json!(false));
        };
        if !self.state.inputs.contains_key(input_id) {
            self.warn_once(
                format!("input-link-target:{input_id}"),
                &format!("engine: WARN {ACTION}: no input with id {input_id} (link ignored)"),
            );
            return Ok(json!(false));
        }

        let mut link = Map::new();
        link.insert("id".to_string(), json!(input_id));
        link.insert(
            "type".to_string(),
            params
                .get("type")
                .cloned()
                .unwrap_or_else(|| json!("state")),
        );
        if let Some(location) = params.get("location").filter(|value| !value.is_null()) {
            link.insert("location".to_string(), location.clone());
        }
        if let Some(args) = params.get("args").filter(|value| !value.is_null()) {
            link.insert("args".to_string(), args.clone());
        }
        link.insert("source".to_string(), json!(source));
        link.insert(
            "min".to_string(),
            params.get("min").cloned().unwrap_or_else(|| json!(0)),
        );
        link.insert(
            "max".to_string(),
            params.get("max").cloned().unwrap_or_else(|| json!(1)),
        );
        if let Some(matcher) = params.get("match").filter(|value| !value.is_null()) {
            link.insert("match".to_string(), matcher.clone());
        }

        let record: InputLink = serde_json::from_value(Value::Object(link))
            .map_err(|error| invalid(ACTION, format!("invalid link record: {error}")))?;
        let value = serde_json::to_value(&record)
            .map_err(|error| invalid(ACTION, format!("link record: {error}")))?;
        self.state.input_links.insert(input_id.to_string(), record);
        self.deltas
            .push("inputs/ADD_INPUT_LINK", json!({ "inputLink": value }));
        Ok(json!(true))
    }

    /// `inputs/updateInputLink` (`inputs.js:178-180`).
    fn update_input_link(&mut self, params: &Map<String, Value>) -> Result<Value, StoreError> {
        const ACTION: &str = "inputs/updateInputLink";
        let Some(input_id) = params.get("inputId").and_then(Value::as_str) else {
            return Ok(Value::Null);
        };
        let Some(key) = params.get("key").and_then(Value::as_str) else {
            return Ok(Value::Null);
        };
        let value = params.get("value").cloned().unwrap_or(Value::Null);
        let Some(link) = self.state.input_links.get(input_id).cloned() else {
            return Ok(Value::Null);
        };

        let mut raw = serde_json::to_value(&link)
            .map_err(|error| invalid(ACTION, format!("link record: {error}")))?;
        let Some(map) = raw.as_object_mut() else {
            return Ok(Value::Null);
        };
        map.insert(key.to_string(), value.clone());
        let updated: InputLink = serde_json::from_value(raw)
            .map_err(|error| invalid(ACTION, format!("invalid link update: {error}")))?;
        self.state.input_links.insert(input_id.to_string(), updated);
        self.deltas.push(
            "inputs/UPDATE_INPUT_LINK",
            json!({ "inputId": input_id, "key": key, "value": value }),
        );
        Ok(Value::Null)
    }

    /// `inputs/removeInputLink` (`inputs.js:182-198`): removing a `tween`
    /// source also drops its descriptor from the tween list.
    fn remove_input_link(&mut self, params: &Map<String, Value>) -> Result<Value, StoreError> {
        let Some(input_id) = params.get("inputId").and_then(Value::as_str) else {
            return Ok(json!(false));
        };
        if !self.state.inputs.contains_key(input_id) {
            return Ok(json!(false));
        }
        let link = self.state.input_links.shift_remove(input_id);
        self.deltas
            .push("inputs/REMOVE_INPUT_LINK", json!({ "inputId": input_id }));

        if link.as_ref().and_then(|link| link.source.as_deref()) == Some("tween") {
            self.remove_tween(input_id);
            self.deltas
                .push("tweens/REMOVE_TWEEN", json!({ "id": input_id }));
        }
        Ok(json!(true))
    }

    /// Remove a tween descriptor by id from the opaque `tweens` slot.
    fn remove_tween(&mut self, id: &str) {
        match &mut self.state.tweens {
            Value::Array(entries) => {
                entries.retain(|entry| entry.get("id").and_then(Value::as_str) != Some(id));
            }
            Value::Object(map) => {
                if let Some(Value::Array(entries)) = map.get_mut("tweens") {
                    entries.retain(|entry| entry.get("id").and_then(Value::as_str) != Some(id));
                } else {
                    map.remove(id);
                }
            }
            _ => {}
        }
    }

    /// `inputs/setFocusedInput` (`inputs.js:98-104`).
    fn set_focused_input(&mut self, params: &Map<String, Value>) -> Result<Value, StoreError> {
        let id = params.get("id").cloned().unwrap_or(Value::Null);
        let title = params.get("title").cloned().unwrap_or(Value::Null);
        self.state.focused_input = json!({ "id": id, "title": title });
        self.deltas.push(
            "inputs/SET_FOCUSED_INPUT",
            json!({ "id": id, "title": title }),
        );
        Ok(Value::Null)
    }

    /// `inputs/clearFocusedInput` (`inputs.js:102-104`).
    fn clear_focused_input(&mut self) -> Result<Value, StoreError> {
        self.set_focused_input(&Map::new())
    }

    // ————————————————— FPS / tweens (WS-12) —————————————————

    /// `fps/SET_FPS`: the tween frame rate (`store.state.fps.fps`).
    ///
    /// Rebuilds every tween so the recorded frame count matches the new rate,
    /// preserving each tween's position fraction (`buildFrames`' `seek`). The
    /// current BPM is passed through unchanged: `effective_duration` applies
    /// the JS `store.state.beats.bpm || 120` fallback when no tempo is known,
    /// so a `max(1.0)` here would rebuild a division-32 tween to ~57 s (R-6).
    fn set_fps(&mut self, params: &Map<String, Value>) -> Result<Value, StoreError> {
        let Some(fps) = params
            .get("fps")
            .and_then(Value::as_f64)
            .filter(|fps| fps.is_finite() && *fps > 0.0)
        else {
            return Ok(Value::Null);
        };
        if (self.fps - fps).abs() <= f64::EPSILON {
            return Ok(Value::Null);
        }
        self.fps = fps;
        let bpm = self.bpm();
        for (_, tween) in self.tweens.iter_mut() {
            tween.rebuild(fps, bpm);
        }
        Ok(Value::Null)
    }

    /// `tweens/addTween`/`tweens/ADD_TWEEN`: store a descriptor and pre-render
    /// it. The payload is either the descriptor itself or `{ tween }`.
    fn add_tween(&mut self, params: &Map<String, Value>) -> Result<Value, StoreError> {
        let payload = params
            .get("tween")
            .and_then(Value::as_object)
            .unwrap_or(params);
        let value = Value::Object(payload.clone());
        if value.get("id").and_then(Value::as_str).is_none() {
            return Ok(Value::Null);
        }
        push_tween_json(&mut self.state.tweens, value.clone());
        match self.tweens.upsert_json(&value, self.fps, self.bpm()) {
            Ok(_) => {}
            Err(error) => {
                self.warn_once(
                    "tween:add".to_string(),
                    &format!("engine: WARN tweens/ADD_TWEEN ignored: {error}"),
                );
            }
        }
        Ok(value)
    }

    /// `tweens/REMOVE_TWEEN`.
    fn remove_tween_action(&mut self, params: &Map<String, Value>) -> Result<Value, StoreError> {
        let Some(id) = params.get("id").and_then(Value::as_str) else {
            return Ok(Value::Null);
        };
        self.remove_tween(id);
        self.tweens.remove(id);
        Ok(Value::Null)
    }

    /// `tweens/createTween` (`tweens.js:135-177`): the action the UI dispatches
    /// when a link switches to a tween source. Builds the same descriptor the
    /// JS stores (minus the pre-recorded `frames`, which stay in the runtime)
    /// and returns it.
    fn create_tween(&mut self, params: &Map<String, Value>) -> Result<Value, StoreError> {
        let id = params
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(crate::ids::uuid_v4);
        let data = params
            .get("data")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_else(|| vec![json!([0.0]), json!([1.0])]);
        let mut descriptor = modv_engine::tweens::TweenDescriptor::new(&id, value_to_rows(&data));
        if let Some(duration) = params.get("duration").and_then(Value::as_f64) {
            descriptor.duration = duration;
        }
        if let Some(direction) = params.get("direction").and_then(Value::as_str) {
            descriptor.direction = direction.to_string();
        }
        if let Some(easing) = params.get("easing").and_then(Value::as_str) {
            descriptor.easing = easing.to_string();
        }
        if let Some(looping) = params.get("loop").and_then(Value::as_bool) {
            descriptor.looping = looping;
        }
        if let Some(use_bpm) = params.get("useBpm").and_then(Value::as_bool) {
            descriptor.use_bpm = use_bpm;
        }
        if let Some(division) = params.get("bpmDivision").and_then(Value::as_f64) {
            descriptor.bpm_division = division;
        }
        if let Some(total) = params.get("durationAsTotalTime").and_then(Value::as_bool) {
            descriptor.duration_as_total_time = total;
        }
        if let Some(steps) = params.get("steps").and_then(Value::as_f64) {
            descriptor.steps = steps;
        }
        if let Some(is_gallery) = params.get("isGallery").and_then(Value::as_bool) {
            descriptor.is_gallery = Some(is_gallery);
        }

        let value = descriptor.to_json();
        push_tween_json(&mut self.state.tweens, value.clone());
        self.tweens
            .insert_descriptor(descriptor, self.fps, self.bpm());
        self.deltas.push("tweens/ADD_TWEEN", value.clone());
        Ok(value)
    }

    /// Register one tween descriptor in the runtime (Scene restore).
    pub(crate) fn register_tween(&mut self, value: &Value) {
        if let Err(error) = self.tweens.upsert_json(value, self.fps, self.bpm()) {
            self.warn_once(
                "tween:register".to_string(),
                &format!("engine: WARN tween ignored: {error}"),
            );
        }
    }

    /// Reload the runtime from `state.tweens` (preset/scene replacement).
    ///
    /// Decoded **per entry**: the tween crate's `replace_from_json` fails
    /// wholesale on one bad descriptor, which used to empty the entire runtime
    /// while the offender stayed in `state.tweens` (R-9). A descriptor that
    /// does not decode keeps its previous runtime, if it had one, so the other
    /// tweens (and the bad one's playhead) survive; the failure is warned once.
    pub(crate) fn reload_tweens(&mut self) {
        let entries = tween_state_entries(&self.state.tweens);
        let previous = std::mem::take(&mut self.tweens);
        let mut tweens = modv_engine::tweens::TweenSet::new();

        for (key, entry) in entries {
            let id = entry
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or(key);
            match tweens.upsert_json(&entry, self.fps, self.bpm()) {
                Ok(_) => {}
                Err(error) => {
                    if let Some(runtime) = id.as_deref().and_then(|id| previous.get(id)).cloned() {
                        tweens.upsert(runtime);
                    }
                    self.warn_once(
                        "tween:load".to_string(),
                        &format!("engine: WARN skipping an undecodable tween: {error}"),
                    );
                }
            }
        }

        self.tweens = tweens;
    }

    /// Drop runtime tweens `state.tweens` no longer lists (Scene apply).
    ///
    /// The per-entry [`Store::register_tween`] already upserts every tween a
    /// Scene document carries, so a binding rebuild after a Scene apply would
    /// only reset the playheads of the tweens that did not change — the JS
    /// keeps those running (`scenes.js` never rebuilds them). Ids that left the
    /// authoritative list are removed here instead (R-8).
    pub(crate) fn prune_tweens(&mut self) {
        let live = tween_state_ids(&self.state.tweens);
        let stale: Vec<String> = self
            .tweens
            .ids()
            .filter(|id| !live.contains(*id))
            .map(str::to_string)
            .collect();
        for id in stale {
            self.tweens.remove(&id);
        }
    }

    /// `tweens/updateBpm` (`tweens.js:183-216`): BPM tweens recompute their
    /// recorded duration/frames; wall-clock tweens are untouched.
    fn update_tween_bpm(&mut self, bpm: f64) {
        let mut changed = false;
        for (_, tween) in self.tweens.iter_mut() {
            if tween.descriptor().use_bpm {
                tween.update_bpm(bpm);
                changed = true;
            }
        }
        if changed {
            self.write_tween_state();
        }
    }

    /// Serialise the runtime's descriptors back into `state.tweens`.
    ///
    /// Per entry: a descriptor with a non-finite number (a duration computed
    /// from a corrupt tempo) is skipped with one warning instead of being
    /// written as `null` — `serde_json` turns a non-finite `f64` into `null`,
    /// and that entry then fails to decode on the next reload, wiping the
    /// whole runtime (R-5/R-9). The runtime keeps the descriptor, so the tween
    /// still drives its linked props for this session.
    fn write_tween_state(&mut self) {
        let mut entries = Vec::with_capacity(self.tweens.len());
        let mut failure: Option<(String, String)> = None;
        for (id, tween) in self.tweens.iter() {
            let descriptor = tween.descriptor();
            let finite = descriptor.duration.is_finite()
                && descriptor.bpm_division.is_finite()
                && descriptor.steps.is_finite();
            let serialized = if finite {
                serde_json::to_value(descriptor).map_err(|error| error.to_string())
            } else {
                Err("a descriptor number is not finite".to_string())
            };
            match serialized {
                Ok(value) => entries.push(value),
                Err(error) => {
                    if failure.is_none() {
                        failure = Some((
                            format!("tween:serialize:{id}"),
                            format!("engine: WARN skipping tween '{id}': {error}"),
                        ));
                    }
                }
            }
        }
        if let Some((key, message)) = failure {
            self.warn_once(key, &message);
        }
        self.state.tweens = Value::Array(entries);
    }

    // ————————————————— Expressions (WS-12) —————————————————

    /// Input id bound to a Module prop (`$props[prop].id`).
    fn module_prop_input_id(&self, module_id: &str, prop: &str) -> Option<String> {
        self.state
            .modules
            .get(module_id)?
            .dollar_props
            .get(prop)?
            .get("id")?
            .as_str()
            .map(str::to_string)
    }

    /// Input id bound to a Module meta field (`meta.<key>InputId`).
    fn module_meta_input_id(&self, module_id: &str, meta_key: &str) -> Option<String> {
        self.state
            .modules
            .get(module_id)?
            .meta
            .get(format!("{meta_key}InputId"))?
            .as_str()
            .map(str::to_string)
    }

    /// The `inputValue` for an expression: the current value at the input's
    /// `getLocation` (`utils/apply-expression.js:16`).
    fn expression_input_value(&self, input_id: &str) -> Value {
        let Some(input) = self.state.inputs.get(input_id) else {
            return Value::Null;
        };
        match &input.get_location {
            Some(path) => resolve_input_value(&self.state, &self.beats, path),
            None => Value::Null,
        }
    }

    /// Run the expression assigned to `input_id`, if any.
    ///
    /// Returns the incoming `value` unchanged when the input has no
    /// expression, when the expression throws (the previous value is kept),
    /// or when the result cannot be represented (the evaluator maps
    /// `undefined` to null, which the caller may still write — matching JS
    /// enough for prop coercion).
    fn apply_expression(&mut self, input_id: &str, value: Value) -> Value {
        if !self.expression_cache.has(input_id) {
            return value;
        }

        let input_value = self.expression_input_value(input_id);
        let osc = json!({});
        let time_ms = wall_clock_ms();

        match self
            .expression_cache
            .apply(input_id, &value, &input_value, &osc, time_ms)
        {
            Ok(Some(result)) => result,
            Ok(None) => value,
            Err(failure) => {
                self.warn_expression_failure(&failure, "runtime");
                value
            }
        }
    }

    /// Recompile the expression cache from `state.expressions`, warning once
    /// per failing assignment. Returns the failures so action handlers can
    /// reject an invalid create/update.
    pub(crate) fn rebuild_expressions(&mut self) -> Vec<ExpressionFailure> {
        let failures = self.expression_cache.rebuild(&self.state);
        for failure in &failures {
            self.warn_expression_failure(failure, "compile");
        }
        failures
    }

    fn warn_expression_failure(&mut self, failure: &ExpressionFailure, kind: &str) {
        self.warn_once(
            format!("expression:{kind}:{}", failure.assignment_id),
            &format!(
                "engine: WARN expression {} {kind} failed: {}",
                failure.assignment_id, failure.message
            ),
        );
    }

    /// Push the mirror mutation `expressions/ADD_EXPRESSION` for `id`.
    fn push_expression_delta(&mut self, id: &str) {
        let Some(assignment) = self.state.expressions.get(id) else {
            return;
        };
        let value = serde_json::to_value(assignment).unwrap_or(Value::Null);
        self.deltas.push(
            "expressions/ADD_EXPRESSION",
            json!({ "assignment": value, "writeToSwap": false }),
        );
    }

    /// `expressions/create` (`expressions.js:49-89`).
    ///
    /// An expression of exactly `value` is the identity JS stores as "no
    /// assignment" (`create` returns `null`), and an invalid source is
    /// rejected the way the JS action's `throw new Error("Unable to compile
    /// Expression")` prevents the assignment from reaching the state.
    ///
    /// `writeToSwap` is accepted and ignored: the engine has no separate swap
    /// buffer for expressions, because preset loading goes through
    /// [`Store::restore`] rather than the six `loadPresetData` + `SWAP` steps.
    fn create_expression(&mut self, params: &Map<String, Value>) -> Result<Value, StoreError> {
        let action = "expressions/create";
        let input_id = required_str(params, action, "inputId")?.to_string();
        let expression = params
            .get("expression")
            .and_then(Value::as_str)
            .unwrap_or("value")
            .to_string();
        if expression.trim() == "value" {
            return Ok(Value::Null);
        }

        let id = params
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(crate::ids::uuid_v4);

        let assignment = ExpressionAssignment {
            id: id.clone(),
            input_id,
            expression,
            evaluate_every_frame: params
                .get("evaluateEveryFrame")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            extra: Map::new(),
        };
        let previous = self.state.expressions.get(&id).cloned();
        self.state.expressions.insert(id.clone(), assignment);

        let failures = self.rebuild_expressions();
        if failures.iter().any(|failure| failure.assignment_id == id) {
            // Reject like the JS action's `throw` would: leave any assignment
            // that already used this id untouched.
            match previous {
                Some(previous) => {
                    self.state.expressions.insert(id.clone(), previous);
                }
                None => {
                    self.state.expressions.shift_remove(&id);
                }
            }
            self.rebuild_expressions();
            return Ok(Value::Null);
        }

        self.push_expression_delta(&id);
        Ok(json!(id))
    }

    /// `expressions/update` (`expressions.js:91-123`).
    fn update_expression(&mut self, params: &Map<String, Value>) -> Result<Value, StoreError> {
        let action = "expressions/update";
        let id = required_str(params, action, "id")?.to_string();
        let Some(existing) = self.state.expressions.get(&id).cloned() else {
            return Err(invalid(
                action,
                format!("no expression with id {id} (update ignored)"),
            ));
        };

        let expression = params
            .get("expression")
            .and_then(Value::as_str)
            .unwrap_or("value")
            .to_string();

        if expression.trim() == "value" {
            self.state.expressions.shift_remove(&id);
            self.rebuild_expressions();
            self.deltas
                .push("expressions/REMOVE_EXPRESSION", json!({ "id": id }));
            return Ok(Value::Null);
        }

        let mut updated = existing.clone();
        updated.expression = expression;
        self.state.expressions.insert(id.clone(), updated);

        let failures = self.rebuild_expressions();
        if failures.iter().any(|failure| failure.assignment_id == id) {
            // Keep the previous expression, like the JS action throwing before
            // it mutates `existingExpression`.
            self.state.expressions.insert(id.clone(), existing);
            self.rebuild_expressions();
            return Ok(Value::Null);
        }

        self.push_expression_delta(&id);
        Ok(json!(id))
    }

    /// `expressions/updateEvaluation` (`expressions.js:125-136`).
    fn update_expression_evaluation(
        &mut self,
        params: &Map<String, Value>,
    ) -> Result<Value, StoreError> {
        let action = "expressions/updateEvaluation";
        let id = required_str(params, action, "id")?.to_string();
        let Some(mut assignment) = self.state.expressions.get(&id).cloned() else {
            return Err(invalid(
                action,
                format!("no expression with id {id} (update ignored)"),
            ));
        };

        assignment.evaluate_every_frame = params
            .get("evaluateEveryFrame")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        self.state.expressions.insert(id.clone(), assignment);
        self.rebuild_expressions();
        self.push_expression_delta(&id);
        Ok(json!(id))
    }

    /// `expressions/remove` (`expressions.js:138-140`).
    fn remove_expression(&mut self, params: &Map<String, Value>) -> Result<Value, StoreError> {
        let action = "expressions/remove";
        let id = required_str(params, action, "id")?.to_string();
        self.state.expressions.shift_remove(&id);
        self.rebuild_expressions();
        self.deltas
            .push("expressions/REMOVE_EXPRESSION", json!({ "id": id }));
        Ok(Value::Null)
    }

    /// Evaluate every `evaluateEveryFrame` assignment once, after the Input
    /// Links and before the frame renders.
    ///
    /// The JS worker stores the flag but never uses it; the engine implements
    /// the documented intent. Writes go straight to the target state (never
    /// back through `updateProp`/`updateMeta`), so one input is evaluated at
    /// most once per frame and cannot recurse into itself.
    fn apply_every_frame_expressions(&mut self, now: Duration) {
        for input_id in self.expression_cache.every_frame_inputs() {
            let Some(input) = self.state.inputs.get(&input_id) else {
                continue;
            };
            let Some(target) = crate::input_links::link_target(&self.state, input) else {
                continue;
            };

            let current = self.expression_target_value(&target);
            let value = self.apply_expression(&input_id, current);
            // `undefined`/no value: keep the previous state. A null write would
            // wipe a prop instead of leaving it where the expression found it.
            if value.is_null() {
                continue;
            }
            self.write_expression_target(&input_id, &target, value, now);
        }
    }

    /// The current value of an expression's write target.
    fn expression_target_value(&self, target: &LinkTarget) -> Value {
        match target {
            LinkTarget::Module {
                module_id,
                prop,
                path,
            } => self
                .state
                .modules
                .get(module_id)
                .and_then(|module| prop_value(&module.props, prop, path.as_deref()).cloned())
                .unwrap_or(Value::Null),
            LinkTarget::ModuleMeta {
                module_id,
                meta_key,
            } => self
                .state
                .modules
                .get(module_id)
                .and_then(|module| module.meta.get(meta_key).cloned())
                .unwrap_or(Value::Null),
            LinkTarget::Group { group_id, key } => {
                let Some(group) = self.state.find_group(group_id) else {
                    return Value::Null;
                };
                match key.as_str() {
                    "alpha" => json!(group.alpha),
                    "hidden" => json!(group.hidden),
                    "enabled" => json!(group.enabled),
                    "clearing" => json!(group.clearing),
                    "inherit" => json!(group.inherit),
                    "pipeline" => json!(group.pipeline),
                    "compositeOperation" => json!(group.composite_operation),
                    _ => Value::Null,
                }
            }
        }
    }

    /// Write an every-frame expression result to its target and emit the
    /// corresponding mirror mutation (throttled like an Input Link write).
    fn write_expression_target(
        &mut self,
        input_id: &str,
        target: &LinkTarget,
        value: Value,
        now: Duration,
    ) {
        match target {
            LinkTarget::Module {
                module_id,
                prop,
                path,
            } => {
                let Some(module) = self.state.modules.get(module_id) else {
                    return;
                };
                let module_name = module.module_name.clone();
                let schema = self
                    .registry
                    .prop_schema(&module_name, prop)
                    .unwrap_or_default();
                let coerced = coerce_prop_value(&schema, &value);
                let unchanged = match prop_value(&module.props, prop, path.as_deref()) {
                    Some(current) => match (current.as_f64(), coerced.as_f64()) {
                        (Some(current), Some(next)) => (current - next).abs() <= LINK_DELTA_EPSILON,
                        _ => *current == coerced,
                    },
                    None => false,
                };
                if unchanged {
                    return;
                }

                let Some(module) = self.state.modules.get_mut(module_id) else {
                    return;
                };
                set_prop_value(&mut module.props, prop, path.as_deref(), coerced.clone());
                let payload = json!({
                    "moduleId": module_id,
                    "prop": prop,
                    "data": {
                        "value": coerced,
                        "type": schema.type_,
                        "path": path.clone().unwrap_or_default(),
                    },
                });
                self.record_link_delta(
                    input_id,
                    coerced.as_f64().unwrap_or(0.0),
                    "modules/UPDATE_ACTIVE_MODULE_PROP",
                    payload,
                    now,
                );
            }
            LinkTarget::ModuleMeta {
                module_id,
                meta_key,
            } => {
                let Some(module) = self.state.modules.get(module_id) else {
                    return;
                };
                if module.meta.get(meta_key) == Some(&value) {
                    return;
                }
                let value = match meta_key.as_str() {
                    "enabled" => json!(
                        value
                            .as_bool()
                            .unwrap_or_else(|| value.as_f64().is_some_and(|value| value != 0.0))
                    ),
                    _ => value,
                };
                let Some(module) = self.state.modules.get_mut(module_id) else {
                    return;
                };
                let Some(meta) = module.meta.as_object_mut() else {
                    return;
                };
                meta.insert(meta_key.clone(), value.clone());
                self.record_link_delta(
                    input_id,
                    value.as_f64().unwrap_or(0.0),
                    "modules/UPDATE_ACTIVE_MODULE_META",
                    json!({ "id": module_id, "metaKey": meta_key, "data": value }),
                    now,
                );
            }
            LinkTarget::Group { group_id, key } => {
                let Some(number) = value.as_f64() else {
                    return;
                };
                let Some(group) = self.state.find_group_mut(group_id) else {
                    return;
                };
                let applied = match key.as_str() {
                    "alpha" => {
                        group.alpha = number;
                        true
                    }
                    "hidden" => {
                        group.hidden = number != 0.0;
                        true
                    }
                    "enabled" => {
                        group.enabled = number as i64;
                        true
                    }
                    _ => false,
                };
                if !applied {
                    return;
                }
                self.record_link_delta(
                    input_id,
                    number,
                    "groups/UPDATE_GROUP_BY_KEY",
                    json!({ "groupId": group_id, "key": key, "data": value }),
                    now,
                );
            }
        }
    }

    /// Evaluate every Input Link against the current audio frame and apply the
    /// resulting writes (WS-9).
    ///
    /// `now` is a monotonic timestamp (the window loop passes its frame clock);
    /// it drives the 10 Hz delta throttle. The compositor renders from the
    /// store immediately, so props react at the full frame rate; only the UI
    /// mirror updates are throttled.
    pub fn update_input_links(&mut self, audio: &AudioFrame, now: Duration) {
        let beats = self.beats();
        let mut writes = Vec::new();
        crate::input_links::evaluate_input_links(
            &self.state,
            audio,
            &beats,
            &mut self.tweens,
            &mut self.link_smoothing,
            &mut writes,
        );
        self.apply_link_writes(writes, now);
        self.apply_every_frame_expressions(now);
    }

    /// Apply resolved writes: coerce, skip unchanged values, and emit at most
    /// one delta per input every [`LINK_DELTA_INTERVAL`].
    fn apply_link_writes(&mut self, writes: Vec<LinkWrite>, now: Duration) {
        for write in writes {
            match &write.target {
                LinkTarget::Module {
                    module_id,
                    prop,
                    path,
                } => self.apply_module_link_write(&write, module_id, prop, path.as_deref(), now),
                LinkTarget::ModuleMeta {
                    module_id,
                    meta_key,
                } => self.apply_meta_link_write(&write, module_id, meta_key, now),
                LinkTarget::Group { group_id, key } => {
                    self.apply_group_link_write(&write, group_id, key, now)
                }
            }
        }
    }

    fn apply_module_link_write(
        &mut self,
        write: &LinkWrite,
        module_id: &str,
        prop: &str,
        path: Option<&str>,
        now: Duration,
    ) {
        let Some(module) = self.state.modules.get(module_id) else {
            return;
        };
        // `modules.js:499-503`: the raw incoming value is compared with the
        // stored prop *before* `applyExpression`.
        let unchanged = prop_value(&module.props, prop, path)
            .and_then(Value::as_f64)
            .is_some_and(|current| (current - write.value).abs() <= LINK_DELTA_EPSILON);
        if unchanged {
            self.clear_link_pending(&write.input_id);
            return;
        }

        // WS-12: JS delivers link writes through `modules/updateProp`, which
        // runs `applyExpression` for the prop's input before coercion.
        let value = self.apply_expression(&write.input_id, json!(write.value));

        let Some(module) = self.state.modules.get(module_id) else {
            return;
        };
        let module_name = module.module_name.clone();
        let schema = self
            .registry
            .prop_schema(&module_name, prop)
            .unwrap_or_default();

        let coerced = coerce_prop_value(&schema, &value);

        // Skip a write the prop already carries after the expression (f32
        // tolerance): the audio frames repeat values constantly and writing
        // the same value burns no GPU time but does churn the mirror.
        let unchanged = match (
            prop_value(&module.props, prop, path),
            coerced.as_f64().filter(|_| !coerced.is_array()),
        ) {
            (Some(current), Some(next)) => current
                .as_f64()
                .is_some_and(|current| (current - next).abs() <= LINK_DELTA_EPSILON),
            (Some(current), None) => *current == coerced,
            _ => false,
        };
        if unchanged {
            self.clear_link_pending(&write.input_id);
            return;
        }

        let Some(module) = self.state.modules.get_mut(module_id) else {
            return;
        };
        set_prop_value(&mut module.props, prop, path, coerced.clone());

        let payload = json!({
            "moduleId": module_id,
            "prop": prop,
            "data": {
                "value": coerced,
                "type": schema.type_,
                "path": path.unwrap_or_default(),
            },
        });
        self.record_link_delta(
            &write.input_id,
            coerced.as_f64().unwrap_or(write.value),
            "modules/UPDATE_ACTIVE_MODULE_PROP",
            payload,
            now,
        );
    }

    /// Apply a link that targets a Module meta value (`alpha`, `enabled`, ...).
    ///
    /// `modules.js:564-590` (`updateMeta`) applies the expression and writes
    /// the value through unchanged, so no clamping happens here beyond the
    /// `enabled` boolean conversion.
    fn apply_meta_link_write(
        &mut self,
        write: &LinkWrite,
        module_id: &str,
        meta_key: &str,
        now: Duration,
    ) {
        // `modules.js:568-571`: the raw link value is compared with the stored
        // meta *before* `applyExpression` (numbers only; `enabled` is a bool in
        // state and never equal to the incoming f64).
        let Some(module) = self.state.modules.get(module_id) else {
            return;
        };
        if module
            .meta
            .get(meta_key)
            .and_then(Value::as_f64)
            .is_some_and(|current| (current - write.value).abs() <= LINK_DELTA_EPSILON)
        {
            self.clear_link_pending(&write.input_id);
            return;
        }

        // WS-12: meta links dispatch `modules/updateMeta`, which is another
        // `applyExpression` call site.
        let value = self.apply_expression(&write.input_id, json!(write.value));

        let Some(module) = self.state.modules.get(module_id) else {
            return;
        };
        let unchanged = match module.meta.get(meta_key) {
            Some(current) => match (current.as_f64(), value.as_f64()) {
                (Some(current), Some(next)) => (current - next).abs() <= LINK_DELTA_EPSILON,
                _ => *current == value,
            },
            None => false,
        };
        if unchanged {
            self.clear_link_pending(&write.input_id);
            return;
        }

        let value = match meta_key {
            "enabled" => json!(
                value
                    .as_bool()
                    .unwrap_or_else(|| value.as_f64().is_some_and(|value| value != 0.0))
            ),
            _ => value,
        };
        let Some(module) = self.state.modules.get_mut(module_id) else {
            return;
        };
        let Some(meta) = module.meta.as_object_mut() else {
            return;
        };
        meta.insert(meta_key.to_string(), value.clone());

        self.record_link_delta(
            &write.input_id,
            value.as_f64().unwrap_or(write.value),
            "modules/UPDATE_ACTIVE_MODULE_META",
            json!({ "id": module_id, "metaKey": meta_key, "data": value }),
            now,
        );
    }

    fn apply_group_link_write(
        &mut self,
        write: &LinkWrite,
        group_id: &str,
        key: &str,
        now: Duration,
    ) {
        // WS-12: group control links get the same expression treatment as the
        // `groups/updateGroupInput` action, keyed by the control's input id.
        let value = self.apply_expression(&write.input_id, json!(write.value));
        let Some(number) = value.as_f64() else {
            return;
        };

        let unchanged = key == "alpha"
            && self
                .state
                .find_group(group_id)
                .is_some_and(|group| (group.alpha - number).abs() <= LINK_DELTA_EPSILON);
        if unchanged {
            self.clear_link_pending(&write.input_id);
            return;
        }

        let Some(group) = self.state.find_group_mut(group_id) else {
            return;
        };

        let measured = number;
        let applied = match key {
            "alpha" => {
                group.alpha = number;
                true
            }
            "hidden" => {
                group.hidden = number != 0.0;
                true
            }
            "enabled" => {
                group.enabled = number as i64;
                true
            }
            _ => false,
        };
        if !applied {
            return;
        }

        let payload = json!({ "groupId": group_id, "key": key, "data": value });
        self.record_link_delta(
            &write.input_id,
            measured,
            "groups/UPDATE_GROUP_BY_KEY",
            payload,
            now,
        );
    }

    /// Store the newest delta and push it when the throttle allows.
    fn record_link_delta(
        &mut self,
        input_id: &str,
        value: f64,
        delta_type: &'static str,
        payload: Value,
        now: Duration,
    ) {
        let entry = self
            .link_deltas
            .entry(input_id.to_string())
            .or_insert_with(|| LinkDeltaState {
                last_emit: None,
                last_value: value,
                pending: None,
            });

        let due = match entry.last_emit {
            None => true,
            Some(last) => {
                now.saturating_sub(last) >= LINK_DELTA_INTERVAL
                    && (entry.last_value - value).abs() > LINK_DELTA_EPSILON
            }
        };

        if due {
            entry.last_emit = Some(now);
            entry.last_value = value;
            entry.pending = None;
            self.deltas.push(delta_type, payload);
        } else {
            entry.pending = Some((delta_type, payload));
        }
    }

    /// Drop a throttled delta whose value the store already carries again.
    fn clear_link_pending(&mut self, input_id: &str) {
        if let Some(entry) = self.link_deltas.get_mut(input_id) {
            entry.pending = None;
        }
    }

    /// Emit any deltas the throttle held back. Called when the engine is about
    /// to snapshot/save, so the renderer mirror sees the final link values
    /// (and the final engine-owned meter bands, WS-9).
    pub fn flush_link_deltas(&mut self) {
        for entry in self.link_deltas.values_mut() {
            let Some((delta_type, payload)) = entry.pending.take() else {
                continue;
            };
            self.deltas.push(delta_type, payload);
        }

        if let Some(payload) = self.engine_meter.pending.take() {
            self.deltas.push("meyda/SET_BANDS", payload);
        }
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

    /// The full mirror-relevant state for a late-joining UI: the preset
    /// document plus the Scene list, performance config and seeding metadata.
    ///
    /// Sent as `state/sync` to each newly connected client: the engine seeds
    /// default Scenes (and can mutate state) before the renderer's socket
    /// client exists, so deltas alone would leave the UI mirror stale.
    pub fn sync_state(&self) -> Value {
        let mut snapshot = self.snapshot();
        if let Value::Object(map) = &mut snapshot {
            // The legacy worker registered module definitions locally; in
            // engine mode nothing does, so the Gallery (and any surface that
            // reads `state.modules.registered[name].meta`) is empty without
            // this map. Sent once per connection, not per delta.
            map.insert("registered".into(), self.registry.registered_modules());
            map.insert("scenes".into(), json!(self.scenes.scenes));
            map.insert("performance".into(), self.scenes.performance.clone());
            map.insert("defaultsSeeded".into(), json!(self.scenes.defaults_seeded));
            map.insert(
                "defaultsVersion".into(),
                json!(self.scenes.defaults_version),
            );
            map.insert(
                "seededDefaultScenes".into(),
                json!(self.scenes.seeded_default_scenes),
            );
            map.insert("appliedSceneId".into(), json!(self.applied_scene_id));
        }
        snapshot
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
    ///
    /// The performance runtime is reset with the Groups it pointed at: a
    /// restore drops the live Groups/Modules a Scene or crossfade was using, so
    /// row runtimes, row transitions, the whole-scene transition and the
    /// applied-Scene stamp would all reference dead ids. They are cleared
    /// outright (a crossfade cannot be completed — its Groups are already gone)
    /// and the rows implied by the restored Groups are re-published, so the
    /// mirror does not keep drawing fades for objects that no longer exist.
    pub fn restore(&mut self, preset: &Value) -> Result<(), StoreError> {
        load_preset_into_state(&mut self.state, preset)?;
        self.deltas.drain();
        // Link smoothing/throttle state belongs to the replaced inputs.
        self.link_smoothing.clear();
        self.link_deltas.clear();
        // A capture request belongs to Groups the restore just replaced.
        self.pending_transition_capture = None;

        // Performance bookkeeping (R-29): drop the runtimes/crossfades rather
        // than finalise them, and tell the mirror so it clears its row-progress
        // meters and its transitioning flag.
        let had_row_progress = !self.performance.row_transitions.is_empty();
        let had_transition = self.performance.transition.is_some();
        self.performance.rows.clear();
        self.performance.row_transitions.clear();
        self.performance.transition = None;
        self.performance.progress_emit_elapsed = 0.0;
        self.applied_scene_id = None;
        if had_row_progress {
            self.deltas.push("scenes/CLEAR_ALL_ROW_PROGRESS", json!({}));
        }
        if had_transition {
            self.deltas.push("scenes/SET_TRANSITIONING", json!(false));
        }

        // Re-emit the active rows from the post-restore Groups: the preset may
        // still carry `sceneId`/`sceneRow` stamps, and whatever it does not
        // carry must not leave the mirror pointing at the deleted ids.
        self.deltas
            .push("scenes/SET_ACTIVE_ROWS", self.live_scene_rows());

        self.reload_tweens();
        // WS-12: preset/Scene documents carry expression source text; compile
        // it after the inputs/modules it targets are in state.
        self.rebuild_expressions();
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

/// Append or replace a tween descriptor in the opaque `tweens` slot.
fn push_tween_json(tweens: &mut Value, tween: Value) {
    let id = tween.get("id").and_then(Value::as_str).map(str::to_string);
    match tweens {
        Value::Array(entries) => {
            if let Some(id) = &id {
                if let Some(existing) = entries
                    .iter_mut()
                    .find(|entry| entry.get("id").and_then(Value::as_str) == Some(id.as_str()))
                {
                    *existing = tween;
                    return;
                }
            }
            entries.push(tween);
        }
        Value::Object(map) => {
            if let Some(entries) = map.get_mut("tweens") {
                push_tween_json(entries, tween);
            } else if let Some(id) = id {
                map.insert(id, tween);
            }
        }
        _ => {
            *tweens = Value::Array(vec![tween]);
        }
    }
}

/// Split `state.tweens` into `(fallback id, descriptor)` pairs.
///
/// Accepts the same shapes the tween crate does (an array of descriptors, the
/// live id-keyed map, or the `{ tweens: ... }` worker root); the fallback id is
/// the map key, used when an entry carries no `id` of its own.
fn tween_state_entries(value: &Value) -> Vec<(Option<String>, Value)> {
    match value {
        Value::Array(entries) => entries.iter().map(|entry| (None, entry.clone())).collect(),
        Value::Object(map) => match map.get("tweens") {
            Some(inner) => tween_state_entries(inner),
            None => map
                .iter()
                .map(|(key, entry)| (Some(key.clone()), entry.clone()))
                .collect(),
        },
        _ => Vec::new(),
    }
}

/// Every tween id `state.tweens` lists, in any of its accepted shapes.
fn tween_state_ids(value: &Value) -> HashSet<String> {
    tween_state_entries(value)
        .into_iter()
        .filter_map(|(key, entry)| {
            entry
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or(key)
        })
        .collect()
}

/// `data: [[...]]` JSON → the descriptor's rows.
fn value_to_rows(data: &[Value]) -> Vec<Vec<f64>> {
    data.iter()
        .map(|row| {
            row.as_array()
                .map(|values| {
                    values
                        .iter()
                        .filter_map(Value::as_f64)
                        .collect::<Vec<f64>>()
                })
                .unwrap_or_default()
        })
        .collect()
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

/// A scalar audio feature: absent → `None`; present but invalid → `Some(0)`.
fn scalar_field(value: Option<&Value>) -> Option<f32> {
    value.map(|value| {
        value
            .as_f64()
            .filter(|number| number.is_finite())
            .map(|number| number as f32)
            .unwrap_or(0.0)
    })
}

/// A fixed-length `f32` array; a different length is ignored entirely.
fn fixed_f32_array<const N: usize>(value: Option<&Value>) -> Option<[f32; N]> {
    let entries = value?.as_array()?;
    if entries.len() != N {
        return None;
    }
    let mut out = [0.0; N];
    for (slot, entry) in out.iter_mut().zip(entries) {
        *slot = entry
            .as_f64()
            .filter(|number| number.is_finite())
            .map(|number| number as f32)
            .unwrap_or(0.0);
    }
    Some(out)
}

/// A `f32` array of up to `N` entries, padded/truncated to `N`.
fn padded_f32_array<const N: usize>(value: Option<&Value>) -> Option<[f32; N]> {
    let entries = value?.as_array()?;
    let mut out = [0.0; N];
    for (slot, entry) in out.iter_mut().zip(entries) {
        *slot = entry
            .as_f64()
            .filter(|number| number.is_finite())
            .map(|number| number as f32)
            .unwrap_or(0.0);
    }
    Some(out)
}

/// A `u8` array of up to `N` entries, clamped to `0..=255` and padded.
fn padded_u8_array<const N: usize>(value: Option<&Value>) -> Option<[u8; N]> {
    let entries = value?.as_array()?;
    let mut out = [0u8; N];
    for (slot, entry) in out.iter_mut().zip(entries) {
        let number = entry
            .as_f64()
            .filter(|number| number.is_finite())
            .unwrap_or(0.0);
        *slot = number.clamp(0.0, 255.0) as u8;
    }
    Some(out)
}

/// Read `props[prop]` or the value at `path` inside `props[prop]`.
/// Wall-clock milliseconds, the value JS passes as `time` (`Date.now()`).
fn wall_clock_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

fn prop_value<'a>(props: &'a Value, prop: &str, path: Option<&str>) -> Option<&'a Value> {
    let mut current = props.get(prop)?;
    let Some(path) = path else {
        return Some(current);
    };
    for segment in parse_path(path) {
        current = match segment {
            PathSegment::Key(key) => current.get(key)?,
            PathSegment::Index(index) => current.get(index)?,
        };
    }
    Some(current)
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
pub(crate) fn set_prop_value(props: &mut Value, prop: &str, path: Option<&str>, value: Value) {
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
    fn make_active_module_point10_uses_the_scene_meta_and_props() {
        let mut store = Store::new();
        let module = store
            .dispatch(
                "modules/makeActiveModule",
                json!({"moduleName": "Point10 Chromatic Wave"}),
            )
            .expect("makeActiveModule dispatches");

        // The JS wrapper meta (`adapter.js:857-866`): a `2d` module in the
        // `2d-scenes` Gallery category.
        assert_eq!(module["meta"]["name"], json!("Point10 Chromatic Wave"));
        assert_eq!(module["meta"]["author"], json!("Point10"));
        assert_eq!(module["meta"]["version"], json!("1.0.0"));
        assert_eq!(module["meta"]["type"], json!("2d"));
        assert_eq!(module["meta"]["category"], json!("2d-scenes"));
        assert_eq!(
            module["meta"]["audioFeatures"],
            json!(["bass", "mid", "treble"])
        );
        assert_eq!(module["$status"], json!([]), "registered, no error status");

        // Prop defaults: the levers then the scene schema.
        assert_eq!(module["props"]["gain"], json!(1.0));
        assert_eq!(module["props"]["smoothing"], json!(0.0));
        assert_eq!(module["props"]["useAgc"], json!(true));
        assert_eq!(module["props"]["distortion"], json!(0.05));
        assert_eq!(module["props"]["amplitude"], json!(1.0));

        let id = module["$id"].as_str().unwrap().to_string();
        // A Scene document can reference the wrapper name and the apply path
        // rebuilds it through this same action (`scene_ops::build_live_module`).
        assert_eq!(
            store.state().modules[&id].module_name,
            "Point10 Chromatic Wave"
        );
    }

    #[test]
    fn update_prop_coerces_point10_scene_props() {
        let mut store = Store::new();
        let module = store
            .dispatch(
                "modules/makeActiveModule",
                json!({"moduleName": "Point10 Chromatic Wave"}),
            )
            .expect("makeActiveModule dispatches");
        let id = module["$id"].as_str().unwrap().to_string();

        // The scene schema clamps on read, so the engine marks its numeric
        // props strict: the stored value is already inside `[min, max]`.
        store
            .dispatch(
                "modules/updateProp",
                json!({"moduleId": id, "prop": "gain", "data": {"value": 9.0}}),
            )
            .expect("updateProp dispatches");
        assert_eq!(store.state().modules[&id].props["gain"], json!(4.0));

        store
            .dispatch(
                "modules/updateProp",
                json!({"moduleId": id, "prop": "distortion", "data": {"value": -1.0}}),
            )
            .expect("updateProp dispatches");
        assert_eq!(store.state().modules[&id].props["distortion"], json!(0.0));

        store
            .dispatch(
                "modules/updateProp",
                json!({"moduleId": id, "prop": "useAgc", "data": {"value": false}}),
            )
            .expect("updateProp dispatches");
        assert_eq!(store.state().modules[&id].props["useAgc"], json!(false));

        let deltas = store.drain_deltas();
        let prop_delta = deltas
            .iter()
            .find(|delta| delta.r#type == "modules/UPDATE_ACTIVE_MODULE_PROP")
            .expect("a prop delta was emitted");
        assert_eq!(prop_delta.payload["data"]["type"], json!("float"));
    }

    // ————————————————— ISF corpus modules (WS-4) —————————————————

    /// Write one inline ISF shader into a fresh temporary directory.
    fn write_inline_isf(source: &str) -> std::path::PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let dir =
            std::env::temp_dir().join(format!("modv-isf-store-{}-{unique}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp corpus directory");
        std::fs::write(dir.join("Inline Test.fs"), source).expect("inline shader written");
        dir
    }

    const INLINE_ISF: &str = r#"/*{
        "CREDIT": "by test",
        "VSN": "1.0.0",
        "ISFVSN": "2",
        "INPUTS": [
            { "NAME": "gain", "TYPE": "float", "MIN": 0.0, "MAX": 2.0, "DEFAULT": 0.5 },
            { "NAME": "flip", "TYPE": "bool", "DEFAULT": false },
            { "NAME": "mode", "TYPE": "long", "DEFAULT": 1,
              "VALUES": [0, 1, 2], "LABELS": ["a", "b", "c"] }
        ]
    }*/
    void main() { gl_FragColor = vec4(1.0); }
    "#;

    #[test]
    fn make_active_module_isf_uses_the_registry_meta_and_props() {
        let dir = write_inline_isf(INLINE_ISF);
        let mut store = Store::with_state_path_and_isf(None, Some(&dir));

        let module = store
            .dispatch(
                "modules/makeActiveModule",
                json!({"moduleName": "Inline Test"}),
            )
            .expect("makeActiveModule dispatches");
        assert_eq!(module["meta"]["name"], json!("Inline Test"));
        assert_eq!(module["meta"]["type"], json!("isf"));
        assert_eq!(module["meta"]["author"], json!("by test"));
        assert_eq!(module["meta"]["version"], json!("1.0.0"));
        assert_eq!(module["$status"], json!([]), "registered, no error status");

        assert_eq!(module["props"]["gain"], json!(0.5));
        assert_eq!(module["props"]["flip"], json!(false));
        assert_eq!(module["props"]["mode"], json!(1));

        let id = module["$id"].as_str().unwrap().to_string();
        assert_eq!(store.state().modules[&id].module_name, "Inline Test");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn update_prop_isf_does_not_clamp_declared_ranges() {
        let dir = write_inline_isf(INLINE_ISF);
        let mut store = Store::with_state_path_and_isf(None, Some(&dir));
        let module = store
            .dispatch(
                "modules/makeActiveModule",
                json!({"moduleName": "Inline Test"}),
            )
            .expect("makeActiveModule dispatches");
        let id = module["$id"].as_str().unwrap().to_string();

        // ISF synthesis never sets `strict`, so the JS renderer stores the
        // out-of-range value unchanged; the shader clamps if it wants to.
        store
            .dispatch(
                "modules/updateProp",
                json!({"moduleId": id, "prop": "gain", "data": {"value": 99.0}}),
            )
            .expect("updateProp dispatches");
        assert_eq!(store.state().modules[&id].props["gain"], json!(99.0));

        store
            .dispatch(
                "modules/updateProp",
                json!({"moduleId": id, "prop": "gain", "data": {"value": -3.0}}),
            )
            .expect("updateProp dispatches");
        assert_eq!(store.state().modules[&id].props["gain"], json!(-3.0));

        // The delta still reports the float data type from the schema.
        let deltas = store.drain_deltas();
        let prop_delta = deltas
            .iter()
            .rev()
            .find(|delta| delta.r#type == "modules/UPDATE_ACTIVE_MODULE_PROP")
            .expect("a prop delta was emitted");
        assert_eq!(prop_delta.payload["data"]["type"], json!("float"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn make_active_module_isf_corpus_shader_has_its_props() {
        let corpus = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../src/renderer/src/application/renderers/isf/modules/isf");
        if !corpus.is_dir() {
            eprintln!("skipping ISF store test: {} is missing", corpus.display());
            return;
        }
        let mut store = Store::with_state_path_and_isf(None, Some(&corpus));
        let module = store
            .dispatch("modules/makeActiveModule", json!({"moduleName": "Bloom"}))
            .expect("makeActiveModule dispatches");

        assert_eq!(module["meta"]["name"], json!("Bloom"));
        assert_eq!(module["meta"]["type"], json!("isf"));
        assert_eq!(module["meta"]["author"], json!("by VIDVOX"));
        assert_eq!(module["meta"]["previewWithOutput"], json!(true));
        assert_eq!(module["$status"], json!([]));

        // `Bloom.fs` declares the image input and the two float props with
        // their header defaults.
        assert_eq!(module["props"]["inputImage"], json!(null));
        assert_eq!(module["props"]["blurAmount"], json!(12));
        assert_eq!(module["props"]["intensity"], json!(0));
        assert!(
            module["props"].get("inputImage").is_some(),
            "the texture prop is registered so panels can show it"
        );
    }

    #[test]
    fn tween_links_resolve_from_the_runtime_and_advance_per_read() {
        let mut store = Store::new();
        store.state.modules.insert(
            "m-tw".to_string(),
            csv_module("m-tw", json!({ "enabled": true })),
        );
        store.state.modules.get_mut("m-tw").expect("module").props = json!({ "amount": 0.0 });
        store
            .dispatch(
                "inputs/addInput",
                json!({
                    "id": "in-tw",
                    "type": "action",
                    "location": "modules/updateProp",
                    "data": {"moduleId": "m-tw", "prop": "amount"},
                }),
            )
            .expect("addInput");
        store
            .dispatch(
                "inputs/createInputLink",
                json!({
                    "inputId": "in-tw",
                    "type": "state",
                    "location": "tweens.tweens['t1'].value[0]",
                    "source": "tween",
                    "min": null,
                    "max": null,
                }),
            )
            .expect("createInputLink");

        let mut descriptor =
            modv_engine::tweens::TweenDescriptor::new("t1", vec![vec![0.0], vec![1.0]]);
        descriptor.use_bpm = false;
        descriptor.duration = 100.0;
        descriptor.looping = false;
        descriptor.direction = "normal".to_string();
        descriptor.easing = "linear".to_string();
        store.state.tweens = json!([descriptor.to_json()]);
        store.reload_tweens();

        // Frame 0 is 0 (the prop is already 0); the next read advances to 1.
        store.update_input_links(&AudioFrame::default(), Duration::ZERO);
        assert_eq!(
            store.state().modules["m-tw"].props["amount"],
            json!(0.0),
            "first read stays on frame 0"
        );
        store.update_input_links(&AudioFrame::default(), Duration::ZERO);
        // 100 ms of a two-row (non-total) tween at 60 fps records three
        // frames; the second read is one third of the way along.
        let advanced = store.state().modules["m-tw"].props["amount"]
            .as_f64()
            .expect("number");
        assert!(
            (advanced - 1.0 / 3.0).abs() < 1e-5,
            "the second read advanced the tween (got {advanced})"
        );
    }

    #[test]
    fn set_bpm_rejects_non_positive_tempos_and_leaves_the_tweens_alone() {
        let mut store = Store::new();
        store.state.tweens = json!([modv_engine::scene_link_defaults::default_link_tween(
            "t1", None
        )]);
        store.reload_tweens();
        let descriptors = store.state.tweens.clone();
        let frames = store.tweens.get("t1").expect("tween").frames().len();

        for bpm in [json!(0), json!(-120), json!(-0.0)] {
            store
                .dispatch("beats/SET_BPM", json!({ "bpm": bpm }))
                .expect("invalid tempo is a no-op");
        }

        assert_eq!(store.bpm(), 0.0, "the engine keeps its 120 BPM fallback");
        assert_eq!(
            store.state.tweens, descriptors,
            "no descriptor may be rewritten (or collapsed to {{}}) by a 0 BPM"
        );
        assert_eq!(
            store.tweens.get("t1").expect("tween").frames().len(),
            frames,
            "the recorded frames stay as they were"
        );
        // A real tempo still applies.
        store
            .dispatch("beats/SET_BPM", json!({ "bpm": 60 }))
            .expect("a positive tempo applies");
        assert_eq!(store.bpm(), 60.0);
        assert_eq!(store.state().tweens[0]["duration"], json!(3840));
    }

    #[test]
    fn fps_rebuild_uses_the_120_bpm_fallback_when_no_tempo_is_known() {
        let mut store = Store::new();
        store.state.tweens = json!([modv_engine::scene_link_defaults::default_link_tween(
            "t1", None
        )]);

        // Reference: a 30 fps rebuild at the JS fallback tempo.
        store.beats.bpm = 120.0;
        store.fps = 30.0;
        store.reload_tweens();
        let expected = store.tweens.get("t1").expect("tween").frames().len();

        // No confident tempo yet (bpm 0): SET_FPS must fall back to 120 BPM
        // instead of rebuilding at 1 BPM (`max(1.0)` -> ~57 s, ~3456 frames).
        store.beats.bpm = 0.0;
        store.fps = 60.0;
        store.reload_tweens();
        store
            .dispatch("fps/SET_FPS", json!({ "fps": 30 }))
            .expect("set fps");

        assert_eq!(
            store.tweens.get("t1").expect("tween").frames().len(),
            expected,
            "a bpm-less rebuild must match the 120 BPM fallback"
        );
        assert!(
            expected < 100,
            "the bpm-less rebuild is the ~480 ms fallback, not a 57 s one ({expected} frames)"
        );
    }

    #[test]
    fn reload_tweens_skips_one_bad_descriptor_and_keeps_the_rest() {
        let mut store = Store::new();
        let good = modv_engine::scene_link_defaults::default_link_tween("t1", None);
        store.state.tweens = json!([good.clone(), { "id": "bad", "data": "not rows" }]);
        store.reload_tweens();

        assert!(
            store.tweens.contains("t1"),
            "the decodable descriptor survives one bad sibling"
        );
        assert!(!store.tweens.contains("bad"));
        assert_eq!(
            store.state.tweens.as_array().map(Vec::len),
            Some(2),
            "the document itself is not rewritten by a load"
        );

        // The bad entry keeps a previous runtime (and its playhead) rather
        // than being dropped when it is known.
        store.state.tweens = json!([good]);
        store.reload_tweens();
        store.tweens.get_mut("t1").expect("tween").set_frame(3);
        store.state.tweens = json!([good, { "id": "t1", "data": 7 }]);
        store.reload_tweens();
        assert_eq!(
            store.tweens.get("t1").expect("tween").frame(),
            3,
            "an undecodable replacement keeps the previous runtime"
        );
    }

    #[test]
    fn tween_state_writes_skip_an_unserialisable_entry_without_losing_the_others() {
        let mut store = Store::new();
        let first = modv_engine::scene_link_defaults::default_link_tween("t1", None);
        let second = modv_engine::scene_link_defaults::default_link_tween("t2", None);
        store.state.tweens = json!([first, second]);
        store.reload_tweens();

        // A non-finite duration cannot be serialised (serde_json refuses it);
        // that entry must be skipped, not collapse `state.tweens` to `{}`.
        store
            .tweens
            .get_mut("t2")
            .expect("tween")
            .descriptor_mut()
            .duration = f64::INFINITY;
        store.write_tween_state();

        let entries = store.state.tweens.as_array().expect("still an array");
        assert_eq!(entries.len(), 1, "only the offender is skipped");
        assert_eq!(entries[0]["id"], json!("t1"));
    }

    #[test]
    fn bpm_and_fps_changes_rebuild_tween_frames() {
        let mut store = Store::new();
        store.state.tweens = json!([modv_engine::scene_link_defaults::default_link_tween(
            "t1", None
        )]);
        store.reload_tweens();
        let frames_at_default = store.tweens.get("t1").expect("tween").frames().len();

        store
            .dispatch("beats/SET_BPM", json!({ "bpm": 60 }))
            .expect("set bpm");
        let slow = store.tweens.get("t1").expect("tween").frames().len();
        assert!(
            slow > frames_at_default,
            "halving the BPM lengthens the tween ({frames_at_default} -> {slow})"
        );
        // `updateBpm` writes `calculatedBpm * data.length` for a per-row
        // duration: (3600 / 60) * 32 * 2.
        assert_eq!(store.state().tweens[0]["duration"], json!(3840));

        store
            .dispatch("fps/SET_FPS", json!({ "fps": 30 }))
            .expect("set fps");
        let half_rate = store.tweens.get("t1").expect("tween").frames().len();
        assert!(
            half_rate < slow,
            "halving the frame rate halves the recorded frames"
        );
        assert_eq!(store.fps, 30.0);
    }

    #[test]
    fn create_tween_stores_and_prerenders_the_descriptor() {
        let mut store = Store::new();
        store.drain_deltas();
        let tween = store
            .dispatch(
                "tweens/createTween",
                json!({
                    "id": "created",
                    "data": [[0], [1]],
                    "duration": 400,
                    "direction": "alternate",
                    "easing": "easeInOutSine",
                    "loop": true,
                    "useBpm": false,
                }),
            )
            .expect("createTween");
        assert_eq!(tween["id"], json!("created"));
        assert_eq!(store.state().tweens[0]["id"], json!("created"));
        assert!(store.tweens.get("created").is_some());
        let deltas = store.drain_deltas();
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].r#type, "tweens/ADD_TWEEN");

        store
            .dispatch("tweens/REMOVE_TWEEN", json!({ "id": "created" }))
            .expect("remove tween");
        assert!(!store.tweens.contains("created"));
        assert_eq!(store.state().tweens, json!([]));
    }

    #[test]
    fn sync_state_carries_the_registered_module_map() {
        let store = Store::new();
        let sync = store.sync_state();
        let registered = sync["registered"]
            .as_object()
            .expect("state/sync includes registered modules");
        assert!(
            registered.len() >= 37 + 15 + 18,
            "expected every built-in module, got {}",
            registered.len()
        );
        assert_eq!(registered["Bands"]["meta"]["name"], json!("Bands"));
        assert_eq!(registered["Bands"]["meta"]["type"], json!("shader"));
        assert_eq!(registered["Ball"]["meta"]["type"], json!("2d"));
        assert_eq!(
            registered["Point10 Chromatic Line"]["meta"]["type"],
            json!("2d")
        );
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
        // A wrong-length cells array is ignored rather than corrupting the
        // grid; the renderer computed the bands, so no delta is echoed back.
        assert_eq!(store.audio().cells, [0.0; 16]);
        assert!(store.drain_deltas().is_empty());
    }

    #[test]
    fn set_bands_stores_the_full_analyser_payload() {
        let mut store = Store::new();
        let lanes: Vec<f64> = (0..12).map(|index| index as f64 / 100.0).collect();
        let cells: Vec<f64> = (0..16).map(|index| 0.5 + index as f64 / 100.0).collect();
        let raw_lanes: Vec<f64> = (0..12).map(|index| 1.0 + index as f64 / 100.0).collect();
        let raw_cells: Vec<f64> = (0..16).map(|index| 2.0 + index as f64 / 100.0).collect();
        let wave: Vec<f64> = (0..1024).map(|index| index as f64 / 1024.0).collect();
        let freq: Vec<f64> = (0..1024).map(|index| index as f64).collect();

        store
            .dispatch(
                "meyda/SET_BANDS",
                json!({
                    "bass": 0.5,
                    "mid": 0.25,
                    "treble": 0.75,
                    "beat": 0.4,
                    "lanes": lanes,
                    "cells": cells,
                    "rawLanes": raw_lanes,
                    "rawCells": raw_cells,
                    "wave": wave,
                    "freqMag": freq,
                    "byteFrequencyData": [1, 2, 300, -5],
                    "sampleRate": 48000,
                    "rms": 0.3,
                    "zcr": 0.05,
                    "energy": 0.9,
                    "spectralCentroid": 1200.0,
                    "spectralFlatness": 0.2,
                    "spectralSlope": -0.1,
                    "spectralRolloff": 3000.0,
                    "spectralSpread": 800.0,
                    "spectralSkewness": 0.4,
                    "spectralKurtosis": 1.5,
                    "perceptualSpread": 0.6,
                    "perceptualSharpness": 0.7,
                    "unknownKey": "ignored"
                }),
            )
            .expect("meyda/SET_BANDS dispatches");

        let audio = store.audio();
        assert!((audio.lanes[0] - 0.0).abs() < 1e-6);
        assert!((audio.lanes[11] - 0.11).abs() < 1e-6);
        assert!((audio.cells[0] - 0.5).abs() < 1e-6);
        assert!((audio.cells[15] - 0.65).abs() < 1e-6);
        assert!((audio.raw_lanes[11] - 1.11).abs() < 1e-6);
        assert!((audio.raw_cells[15] - 2.15).abs() < 1e-6);
        assert!((audio.beat - 0.4).abs() < 1e-6);
        assert!((audio.wave[0] - 0.0).abs() < 1e-6);
        assert!((audio.wave[1023] - 1023.0 / 1024.0).abs() < 1e-6);
        assert!((audio.freq_mag[1023] - 1023.0).abs() < 1e-6);
        // Byte values are clamped, not wrapped; a short array pads with zero.
        assert_eq!(&audio.byte_frequency_data[..4], &[1, 2, 255, 0]);
        assert_eq!(audio.byte_frequency_data[1023], 0);
        assert_eq!(audio.sample_rate, 48_000.0);
        // `Waveform` reads `features.buffer`; it mirrors the wave.
        assert_eq!(audio.features.buffer, audio.wave.to_vec());
        assert!((audio.features.rms - 0.3).abs() < 1e-6);
        assert!((audio.features.zcr - 0.05).abs() < 1e-6);
        assert!((audio.features.energy - 0.9).abs() < 1e-6);
        assert!((audio.features.spectral_centroid - 1200.0).abs() < 1e-6);
        assert!((audio.features.spectral_flatness - 0.2).abs() < 1e-6);
        assert!((audio.features.spectral_slope + 0.1).abs() < 1e-6);
        assert!((audio.features.spectral_rolloff - 3000.0).abs() < 1e-6);
        assert!((audio.features.spectral_spread - 800.0).abs() < 1e-6);
        assert!((audio.features.spectral_skewness - 0.4).abs() < 1e-6);
        assert!((audio.features.spectral_kurtosis - 1.5).abs() < 1e-6);
        assert!((audio.features.perceptual_spread - 0.6).abs() < 1e-6);
        assert!((audio.features.perceptual_sharpness - 0.7).abs() < 1e-6);
    }

    #[test]
    fn set_bands_ignores_bad_lengths_and_non_finite_values() {
        let mut store = Store::new();
        // Seed a valid grid.
        let lanes: Vec<f64> = (0..12).map(|_| 0.5).collect();
        let cells: Vec<f64> = (0..16).map(|_| 0.25).collect();
        store
            .dispatch(
                "meyda/SET_BANDS",
                json!({"lanes": lanes, "cells": cells, "sampleRate": 44100}),
            )
            .expect("dispatch");

        // Wrong lengths must not replace the seeded grid.
        store
            .dispatch(
                "meyda/SET_BANDS",
                json!({
                    "lanes": [1, 2, 3],
                    "cells": [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17],
                    "sampleRate": 0,
                    "rms": "not a number"
                }),
            )
            .expect("dispatch");

        assert_eq!(store.audio().lanes, [0.5; 12]);
        assert_eq!(store.audio().cells, [0.25; 16]);
        // A non-positive sample rate keeps the previous (default) rate rather
        // than storing zero; an unparsable scalar becomes 0.
        assert_eq!(store.audio().sample_rate, 44_100.0);
        assert_eq!(store.audio().features.rms, 0.0);
    }

    #[test]
    fn set_bands_derives_wave_from_the_byte_time_domain_fallback() {
        let mut store = Store::new();
        store
            .dispatch(
                "meyda/SET_BANDS",
                json!({"byteTimeDomainData": [0, 128, 255]}),
            )
            .expect("dispatch");

        assert!((store.audio().wave[0] + 1.0).abs() < 1e-6);
        assert!((store.audio().wave[1] - 0.003_921_569).abs() < 1e-5);
        assert!((store.audio().wave[2] - 1.0).abs() < 1e-6);
        // Unspecified tail bytes are zero, which maps to -1.
        assert!((store.audio().wave[3] + 1.0).abs() < 1e-6);
    }

    #[test]
    fn beats_actions_store_bpm_and_kick_and_echo_deltas() {
        let mut store = Store::new();
        assert_eq!(store.bpm(), 0.0);
        assert!(!store.kick());

        store
            .dispatch(
                "beats/setBpm",
                json!({"bpm": 128, "source": "beatdetektor"}),
            )
            .expect("setBpm dispatches");
        assert_eq!(store.bpm(), 128.0);

        store
            .dispatch("beats/SET_KICK", json!({"kick": true}))
            .expect("SET_KICK dispatches");
        assert!(store.kick());

        let deltas: Vec<String> = store
            .drain_deltas()
            .into_iter()
            .map(|delta| delta.r#type)
            .collect();
        assert_eq!(deltas, vec!["beats/SET_BPM", "beats/SET_KICK"]);

        // An equal value is not re-broadcast.
        store
            .dispatch("beats/SET_KICK", json!({"kick": true}))
            .expect("SET_KICK dispatches");
        assert!(store.drain_deltas().is_empty());

        // A foreign BPM source cannot set the tempo.
        store
            .dispatch("beats/setBpm", json!({"bpm": 90, "source": "tap"}))
            .expect("setBpm dispatches");
        assert_eq!(store.bpm(), 128.0);
    }

    /// A real analyser frame (silent) for the engine-owned audio tests: the
    /// snapshot shape belongs to `modv-media`, so building it through the
    /// analyser (instead of a hand-rolled literal) keeps these tests honest
    /// when the payload grows.
    fn engine_snapshot() -> modv_media::audio::analyser::AudioSnapshot {
        modv_media::audio::analyser::AudioAnalyser::new(48_000.0).frame(0.0)
    }

    /// The delta types in issue order.
    fn delta_types(store: &mut Store) -> Vec<String> {
        store
            .drain_deltas()
            .into_iter()
            .map(|delta| delta.r#type)
            .collect()
    }

    #[test]
    fn engine_audio_frame_becomes_the_stored_audio_frame() {
        let mut store = Store::new();
        assert!(!store.engine_audio_active());

        let mut snapshot = engine_snapshot();
        snapshot.bands = [0.6, 0.4, 0.3];
        snapshot.cells = [0.25; 16];
        snapshot.lanes = [0.5; 12];
        snapshot.beat = 0.2;
        snapshot.sample_rate = 48_000.0;
        snapshot.features.rms = 0.3;
        store.set_engine_audio(&snapshot, false, Duration::ZERO);

        assert!(store.engine_audio_active());
        assert_eq!(store.audio().bass, 0.6);
        assert_eq!(store.audio().mid, 0.4);
        assert_eq!(store.audio().treble, 0.3);
        assert_eq!(store.audio().cells, [0.25; 16]);
        assert_eq!(store.audio().lanes, [0.5; 12]);
        assert_eq!(store.audio().beat, 0.2);
        assert_eq!(store.audio().sample_rate, 48_000.0);
        assert_eq!(store.audio().features.rms, 0.3);
    }

    #[test]
    fn renderer_bands_are_accepted_until_engine_audio_starts() {
        let mut store = Store::new();
        store
            .dispatch("meyda/SET_BANDS", json!({"bass": 0.5, "mid": 0.25}))
            .expect("renderer bands");

        assert_eq!(store.audio().bass, 0.5);
        assert_eq!(store.audio().mid, 0.25);
    }

    #[test]
    fn engine_audio_ignores_renderer_bands_and_tempo() {
        let mut store = Store::new();
        let mut snapshot = engine_snapshot();
        snapshot.bands = [0.6, 0.4, 0.3];
        snapshot.bpm = Some(128.0);
        store.set_engine_audio(&snapshot, true, Duration::ZERO);
        store.drain_deltas();

        // The renderer's pump keeps running in engine mode; its frames (usually
        // silence, because the engine owns the device) must not win.
        store
            .dispatch(
                "meyda/SET_BANDS",
                json!({"bass": 0.9, "mid": 0.9, "treble": 0.9, "cells": [1.0]}),
            )
            .expect("ignored bands");
        store
            .dispatch("beats/setBpm", json!({"bpm": 90, "source": "beatdetektor"}))
            .expect("ignored bpm");
        store
            .dispatch("beats/SET_KICK", json!({"kick": false}))
            .expect("ignored kick");

        assert_eq!(store.audio().bass, 0.6, "engine bands stay authoritative");
        assert_eq!(store.audio().mid, 0.4);
        assert_eq!(store.bpm(), 128.0, "engine tempo stays authoritative");
        assert!(store.kick(), "engine kick stays authoritative");
        assert!(
            store.drain_deltas().is_empty(),
            "ignored writes emit nothing"
        );
    }

    #[test]
    fn engine_audio_publishes_bpm_and_kick_only_on_change() {
        let mut store = Store::new();
        let mut snapshot = engine_snapshot();
        snapshot.bpm = Some(120.0);

        store.set_engine_audio(&snapshot, true, Duration::ZERO);
        assert_eq!(
            delta_types(&mut store),
            vec!["beats/SET_BPM", "beats/SET_KICK", "meyda/SET_BANDS"]
        );
        assert_eq!(store.bpm(), 120.0);
        assert!(store.kick());

        // The same tempo/kick a frame later: no beats deltas, only the
        // throttled meter is due at 100 ms.
        store.set_engine_audio(&snapshot, true, Duration::from_millis(100));
        assert_eq!(delta_types(&mut store), vec!["meyda/SET_BANDS"]);

        // A tempo change (and a kick release) is published once.
        let mut changed = engine_snapshot();
        changed.bpm = Some(128.0);
        store.set_engine_audio(&changed, false, Duration::from_millis(200));
        assert_eq!(
            delta_types(&mut store),
            vec!["beats/SET_BPM", "beats/SET_KICK", "meyda/SET_BANDS"]
        );
        assert_eq!(store.bpm(), 128.0);
        assert!(!store.kick());

        // An unconfident estimate (`None`) keeps the last tempo.
        store.set_engine_audio(&engine_snapshot(), false, Duration::from_millis(300));
        assert_eq!(store.bpm(), 128.0);
        assert_eq!(delta_types(&mut store), vec!["meyda/SET_BANDS"]);
    }

    #[test]
    fn engine_audio_meter_deltas_are_throttled_to_ten_hertz() {
        let mut store = Store::new();
        let mut snapshot = engine_snapshot();
        snapshot.bands = [0.5, 0.0, 0.0];

        // A 60 Hz pump over 250 ms: publishes at 0, 100 and 200 ms only.
        let mut published = 0;
        for frame in 0..16 {
            let now = Duration::from_millis(frame * 16);
            let mut frame_snapshot = snapshot.clone();
            frame_snapshot.bands[0] = frame as f32 / 16.0;
            store.set_engine_audio(&frame_snapshot, false, now);
            published += delta_types(&mut store)
                .iter()
                .filter(|delta| delta.as_str() == "meyda/SET_BANDS")
                .count();
        }

        assert_eq!(published, 3, "one publish per 100 ms, not per frame");
        // The newest value is held until the next slot and flushed on quit.
        store.flush_link_deltas();
        let deltas = store.drain_deltas();
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].r#type, "meyda/SET_BANDS");
        assert_eq!(deltas[0].payload["bass"], json!(15.0 / 16.0));
        assert!(deltas[0].payload["cells"].is_array());
    }

    #[test]
    fn engine_audio_tempo_rebuilds_tween_durations() {
        let mut store = Store::new();
        store.state.tweens = json!([modv_engine::scene_link_defaults::default_link_tween(
            "t1", None
        )]);
        store.reload_tweens();

        let mut snapshot = engine_snapshot();
        snapshot.bpm = Some(120.0);
        store.set_engine_audio(&snapshot, false, Duration::ZERO);

        // `updateBpm` writes `calculatedBpm * data.length` for a per-row
        // duration: (3600 / 120) * 32 * 2.
        assert_eq!(store.state().tweens[0]["duration"], json!(1920));
    }

    #[test]
    fn disabling_engine_audio_hands_ownership_back_to_the_renderer() {
        let mut store = Store::new();
        store.set_engine_audio(&engine_snapshot(), false, Duration::ZERO);
        assert!(store.engine_audio_active());

        store.disable_engine_audio();
        assert!(!store.engine_audio_active());

        store
            .dispatch("meyda/SET_BANDS", json!({"bass": 0.75}))
            .expect("renderer bands after fallback");
        assert_eq!(store.audio().bass, 0.75);
    }

    /// A preset with one module, one action input and one getter link.
    fn link_preset(feature: &str) -> Value {
        json!({
            "groups": [],
            "modules": {
                "m1": {
                    "$id": "m1",
                    "$moduleName": "Bands",
                    "meta": { "name": "Bands", "type": "shader", "enabled": true },
                    "props": { "amount": 0.0 },
                    "data": {},
                    "$props": { "amount": { "id": "input-a", "type": "float" } }
                }
            },
            "inputs": {
                "focusedInput": { "id": null, "title": null },
                "inputs": {
                    "input-a": {
                        "id": "input-a",
                        "type": "action",
                        "data": { "moduleId": "m1", "prop": "amount" },
                        "getLocation": "modules.active[\"m1\"].props[\"amount\"]",
                        "location": "modules/updateProp"
                    }
                },
                "inputLinks": {
                    "input-a": {
                        "id": "input-a",
                        "type": "getter",
                        "location": "meyda/getFeature",
                        "source": "meyda",
                        "args": [feature]
                    }
                }
            },
            "expressions": { "assignments": {} },
            "tweens": [],
            "midi": {}
        })
    }

    fn audio_with_bass(bass: f32) -> AudioFrame {
        AudioFrame {
            bass,
            ..AudioFrame::default()
        }
    }

    #[test]
    fn input_links_write_module_props_from_audio() {
        let mut store = Store::new();
        store.restore(&link_preset("bass")).expect("preset loads");
        store.drain_deltas();

        store.update_input_links(&audio_with_bass(0.75), Duration::from_millis(0));

        // Values are coerced through the prop schema; the registry's Bands
        // `amount` prop is a plain float.
        let amount = store.state().modules["m1"].props["amount"]
            .as_f64()
            .expect("amount is numeric");
        assert!((amount - 0.75).abs() < 1e-6);
        assert_eq!(store.drain_deltas().len(), 1);
    }

    #[test]
    fn input_link_deltas_are_throttled_to_one_per_interval() {
        let mut store = Store::new();
        store.restore(&link_preset("bass")).expect("preset loads");
        store.drain_deltas();

        // First frame always emits.
        store.update_input_links(&audio_with_bass(0.1), Duration::from_millis(0));
        assert_eq!(store.drain_deltas().len(), 1);

        // Within 100 ms the prop still updates (the compositor reads it) but
        // no delta reaches the mirror.
        store.update_input_links(&audio_with_bass(0.4), Duration::from_millis(50));
        assert_eq!(store.drain_deltas().len(), 0);
        assert!(
            (store.state().modules["m1"].props["amount"]
                .as_f64()
                .unwrap()
                - 0.4)
                .abs()
                < 1e-6
        );

        // At 100 ms (and a >1e-3 change) exactly one delta goes out.
        store.update_input_links(&audio_with_bass(0.6), Duration::from_millis(100));
        let deltas = store.drain_deltas();
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].r#type, "modules/UPDATE_ACTIVE_MODULE_PROP");
        assert!((deltas[0].payload["data"]["value"].as_f64().unwrap() - 0.6).abs() < 1e-6);

        // A pending within-interval value is flushed on request.
        store.update_input_links(&audio_with_bass(0.9), Duration::from_millis(150));
        assert_eq!(store.drain_deltas().len(), 0);
        store.flush_link_deltas();
        let deltas = store.drain_deltas();
        assert_eq!(deltas.len(), 1);
        assert!((deltas[0].payload["data"]["value"].as_f64().unwrap() - 0.9).abs() < 1e-6);
    }

    #[test]
    fn input_links_skip_unchanged_values() {
        let mut store = Store::new();
        store.restore(&link_preset("bass")).expect("preset loads");
        store.drain_deltas();

        store.update_input_links(&audio_with_bass(0.5), Duration::from_millis(0));
        assert_eq!(store.drain_deltas().len(), 1);

        store.update_input_links(&audio_with_bass(0.500_000_1), Duration::from_millis(200));
        assert_eq!(store.drain_deltas().len(), 0);
    }

    #[test]
    fn input_links_write_sub_prop_paths() {
        let mut preset = link_preset("bass");
        preset["modules"]["m1"]["props"]["position"] = json!([0.0, 0.0]);
        preset["inputs"]["inputs"]["input-a"]["data"] =
            json!({ "moduleId": "m1", "prop": "position", "path": "[1]" });
        preset["modules"]["m1"]["$props"]["position"] = json!({ "id": "input-a", "type": "float" });

        let mut store = Store::new();
        store.restore(&preset).expect("preset loads");
        store.update_input_links(&audio_with_bass(0.25), Duration::from_millis(0));

        // The registry's Bands module has no `position` prop, so the value is
        // stored without coercion, exactly like `updateProp`'s fallback.
        assert_eq!(
            store.state().modules["m1"].props["position"],
            json!([0.0, 0.25])
        );
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
        // Every pre-restore mutation is dropped (they reference deleted
        // objects); the only queued delta is the post-restore row re-publish.
        let deltas = store.drain_deltas();
        assert_eq!(deltas.len(), 1, "{deltas:?}");
        assert_eq!(deltas[0].r#type, "scenes/SET_ACTIVE_ROWS");
        assert_eq!(deltas[0].payload, json!({}), "no Scene-stamped Group");
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

    /// A minimal module instance for tests that seed the state directly.
    fn csv_module(id: &str, meta: Value) -> ModuleInstance {
        ModuleInstance {
            id: id.to_string(),
            module_name: "Bands".to_string(),
            meta,
            props: json!({}),
            data: json!({}),
            dollar_props: json!({}),
            status: Vec::new(),
            extra: serde_json::Map::new(),
        }
    }

    #[test]
    fn add_input_returns_the_record_and_emits_the_mirror_delta() {
        let mut store = Store::new();
        let input = store
            .dispatch(
                "inputs/addInput",
                json!({
                    "type": "action",
                    "getLocation": "modules.active[\"m\"].props.alpha",
                    "location": "modules/updateProp",
                    "data": { "moduleId": "m", "prop": "alpha" },
                }),
            )
            .expect("addInput dispatches");
        let id = input["id"].as_str().expect("id assigned").to_string();
        assert_eq!(input["type"], json!("action"));
        assert_eq!(
            input["getLocation"],
            json!("modules.active[\"m\"].props.alpha")
        );
        assert!(store.state().inputs.contains_key(&id));

        let deltas = store.drain_deltas();
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].r#type, "inputs/ADD_INPUT");
        assert_eq!(deltas[0].payload["input"]["id"], json!(id));

        // The mutation-shaped `{ input }` payload is accepted too.
        store
            .dispatch(
                "inputs/addInput",
                json!({"input": {"id": "known", "type": "commit"}}),
            )
            .expect("addInput accepts a mutation payload");
        assert!(store.state().inputs.contains_key("known"));
    }

    #[test]
    fn create_input_link_defaults_bounds_and_requires_source_and_target() {
        let mut store = Store::new();
        store
            .dispatch(
                "inputs/addInput",
                json!({"id": "input-1", "type": "action", "location": "modules/updateProp"}),
            )
            .expect("addInput");
        store.drain_deltas();

        assert_eq!(
            store
                .dispatch(
                    "inputs/createInputLink",
                    json!({"inputId": "missing", "source": "meyda", "type": "getter"}),
                )
                .expect("createInputLink"),
            json!(false)
        );
        assert_eq!(
            store
                .dispatch(
                    "inputs/createInputLink",
                    json!({"inputId": "input-1", "type": "getter"}),
                )
                .expect("createInputLink"),
            json!(false)
        );

        let created = store
            .dispatch(
                "inputs/createInputLink",
                json!({
                    "inputId": "input-1",
                    "type": "getter",
                    "location": "meyda/getFeature",
                    "source": "meyda",
                    "args": ["bass"],
                }),
            )
            .expect("createInputLink");
        assert_eq!(created, json!(true));

        let link = &store.state().input_links["input-1"];
        assert!(link.is_getter());
        assert_eq!(link.args, Some(vec![json!("bass")]));
        // JS destructuring defaults: absent bounds become 0/1.
        assert_eq!(link.min, Some(0.0));
        assert_eq!(link.max, Some(1.0));

        let deltas = store.drain_deltas();
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].r#type, "inputs/ADD_INPUT_LINK");
        assert_eq!(deltas[0].payload["inputLink"]["id"], json!("input-1"));
    }

    #[test]
    fn update_and_remove_input_link_edit_the_record_and_drop_tweens() {
        let mut store = Store::new();
        store
            .dispatch(
                "inputs/addInput",
                json!({"id": "input-2", "type": "action", "location": "modules/updateProp"}),
            )
            .expect("addInput");
        store
            .dispatch(
                "inputs/createInputLink",
                json!({
                    "inputId": "input-2",
                    "type": "state",
                    "location": "beats.kick",
                    "source": "tween",
                }),
            )
            .expect("createInputLink");
        store.state.tweens = json!([{"id": "input-2", "data": [[0], [1]]}]);
        store.drain_deltas();

        store
            .dispatch(
                "inputs/updateInputLink",
                json!({"inputId": "input-2", "key": "min", "value": 2}),
            )
            .expect("updateInputLink");
        assert_eq!(store.state().input_links["input-2"].min, Some(2.0));
        assert_eq!(store.state().input_links["input-2"].max, Some(1.0));

        store
            .dispatch("inputs/removeInputLink", json!({"inputId": "input-2"}))
            .expect("removeInputLink");
        assert!(!store.state().input_links.contains_key("input-2"));
        assert_eq!(store.state().tweens, json!([]));

        let deltas = store.drain_deltas();
        let names: Vec<&str> = deltas.iter().map(|delta| delta.r#type.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "inputs/UPDATE_INPUT_LINK",
                "inputs/REMOVE_INPUT_LINK",
                "tweens/REMOVE_TWEEN"
            ]
        );
    }

    #[test]
    fn focused_input_round_trips_and_clears() {
        let mut store = Store::new();
        store
            .dispatch(
                "inputs/setFocusedInput",
                json!({"id": "input-3", "title": "Gain"}),
            )
            .expect("setFocusedInput");
        assert_eq!(
            store.state().focused_input,
            json!({"id": "input-3", "title": "Gain"})
        );
        store
            .dispatch("inputs/clearFocusedInput", Value::Null)
            .expect("clearFocusedInput");
        assert_eq!(
            store.state().focused_input,
            json!({"id": null, "title": null})
        );
        let deltas = store.drain_deltas();
        assert_eq!(deltas.len(), 2);
        assert_eq!(deltas[1].r#type, "inputs/SET_FOCUSED_INPUT");
    }

    #[test]
    fn meta_links_write_module_meta_and_emit_the_update_meta_delta() {
        use std::time::Duration;
        let mut store = Store::new();
        store.state.modules.insert(
            "m-meta".to_string(),
            csv_module("m-meta", json!({ "alpha": 1.0, "enabled": true })),
        );
        store
            .dispatch(
                "inputs/addInput",
                json!({
                    "id": "input-meta",
                    "type": "action",
                    "location": "modules/updateMeta",
                    "data": {"id": "m-meta", "metaKey": "alpha"},
                }),
            )
            .expect("addInput");
        store
            .dispatch(
                "inputs/createInputLink",
                json!({
                    "inputId": "input-meta",
                    "type": "getter",
                    "location": "meyda/getFeature",
                    "source": "meyda",
                    "args": ["bass"],
                    "min": 0.2,
                    "max": 0.8,
                }),
            )
            .expect("createInputLink");
        store.drain_deltas();

        let mut audio = AudioFrame::default();
        audio.bass = 0.5;
        store.update_input_links(&audio, Duration::ZERO);

        let module = store.state().modules.get("m-meta").expect("module");
        assert!((module.meta["alpha"].as_f64().unwrap() - 0.5).abs() < 1e-6);
        let deltas = store.drain_deltas();
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].r#type, "modules/UPDATE_ACTIVE_MODULE_META");
        assert_eq!(deltas[0].payload["id"], json!("m-meta"));
        assert_eq!(deltas[0].payload["metaKey"], json!("alpha"));
        assert!((deltas[0].payload["data"].as_f64().unwrap() - 0.5).abs() < 1e-6);
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

    // ————————————————— Expressions (WS-12) —————————————————

    /// A one-module preset with no links and no expressions.
    fn expression_preset() -> Value {
        let mut preset = link_preset("bass");
        preset["inputs"]["inputLinks"] = json!({});
        preset["expressions"] = json!({ "assignments": {} });
        preset
    }

    /// [`expression_preset`] plus one assignment on `input-a`.
    fn preset_with_expression(expression: &str, evaluate_every_frame: bool) -> Value {
        let mut preset = expression_preset();
        preset["expressions"] = json!({
            "assignments": {
                "ex-1": {
                    "id": "ex-1",
                    "inputId": "input-a",
                    "expression": expression,
                    "evaluateEveryFrame": evaluate_every_frame,
                }
            }
        });
        preset
    }

    fn amount(store: &Store) -> f64 {
        store.state().modules["m1"].props["amount"]
            .as_f64()
            .expect("amount is numeric")
    }

    #[test]
    fn expression_transforms_a_manual_update_prop_write() {
        let mut store = Store::new();
        store
            .restore(&preset_with_expression("value * 3", false))
            .expect("preset loads");
        store.drain_deltas();

        store
            .dispatch(
                "modules/updateProp",
                json!({
                    "moduleId": "m1",
                    "prop": "amount",
                    "data": { "value": 2.0 },
                }),
            )
            .expect("updateProp dispatches");

        assert!((amount(&store) - 6.0).abs() < 1e-6);
        let deltas = store.drain_deltas();
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].r#type, "modules/UPDATE_ACTIVE_MODULE_PROP");
        assert!((deltas[0].payload["data"]["value"].as_f64().unwrap() - 6.0).abs() < 1e-6);
    }

    #[test]
    fn expression_transforms_an_audio_link_write() {
        let mut preset = preset_with_expression("value * 2", false);
        preset["inputs"]["inputLinks"] = link_preset("bass")["inputs"]["inputLinks"].clone();
        let mut store = Store::new();
        store.restore(&preset).expect("preset loads");
        store.drain_deltas();

        store.update_input_links(&audio_with_bass(0.25), Duration::from_millis(0));

        assert!((amount(&store) - 0.5).abs() < 1e-6);
        let deltas = store.drain_deltas();
        assert_eq!(deltas.len(), 1);
        assert!((deltas[0].payload["data"]["value"].as_f64().unwrap() - 0.5).abs() < 1e-6);
    }

    #[test]
    fn expression_transforms_a_meta_link_write() {
        let mut preset = expression_preset();
        preset["modules"]["m1"]["meta"]["alphaInputId"] = json!("input-meta");
        preset["inputs"]["inputs"]["input-meta"] = json!({
            "id": "input-meta",
            "type": "action",
            "data": { "id": "m1", "metaKey": "alpha" },
            "getLocation": "modules.active[\"m1\"].meta.alpha",
            "location": "modules/updateMeta",
        });
        preset["inputs"]["inputLinks"]["input-meta"] = json!({
            "id": "input-meta",
            "type": "getter",
            "location": "meyda/getFeature",
            "source": "meyda",
            "args": ["bass"],
        });
        preset["expressions"] = json!({
            "assignments": {
                "ex-meta": {
                    "id": "ex-meta",
                    "inputId": "input-meta",
                    "expression": "value * 2",
                    "evaluateEveryFrame": false,
                }
            }
        });

        let mut store = Store::new();
        store.restore(&preset).expect("preset loads");
        store.drain_deltas();
        store.update_input_links(&audio_with_bass(0.25), Duration::from_millis(0));

        let module = store.state().modules.get("m1").expect("module");
        assert!((module.meta["alpha"].as_f64().unwrap() - 0.5).abs() < 1e-6);
        let deltas = store.drain_deltas();
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].r#type, "modules/UPDATE_ACTIVE_MODULE_META");
        assert!((deltas[0].payload["data"].as_f64().unwrap() - 0.5).abs() < 1e-6);
    }

    #[test]
    fn expression_transforms_a_group_link_write() {
        let mut store = Store::new();
        let group = store
            .dispatch("groups/createGroup", json!({ "name": "Effects" }))
            .expect("createGroup dispatches");
        let group_id = group["id"].as_str().expect("id").to_string();
        let input = store
            .dispatch(
                "inputs/addInput",
                json!({
                    "id": "input-alpha",
                    "type": "commit",
                    "location": "groups/UPDATE_GROUP_BY_KEY",
                    "data": { "groupId": group_id, "key": "alpha" },
                }),
            )
            .expect("addInput dispatches");
        assert_eq!(input["id"], json!("input-alpha"));
        store
            .dispatch(
                "groups/UPDATE_GROUP_BY_KEY",
                json!({ "groupId": group_id, "key": "alphaInputId", "value": "input-alpha" }),
            )
            .expect("bind alpha input");
        store
            .dispatch(
                "expressions/create",
                json!({
                    "id": "ex-group",
                    "inputId": "input-alpha",
                    "expression": "value * 2",
                }),
            )
            .expect("create expression");
        store
            .dispatch(
                "inputs/createInputLink",
                json!({
                    "inputId": "input-alpha",
                    "source": "meyda",
                    "type": "getter",
                    "location": "meyda/getFeature",
                    "args": ["bass"],
                }),
            )
            .expect("create link");
        store.drain_deltas();

        store.update_input_links(&audio_with_bass(0.25), Duration::from_millis(0));

        let live = store.state().find_group(&group_id).expect("group");
        assert!((live.alpha - 0.5).abs() < 1e-6);
        let deltas = store.drain_deltas();
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].r#type, "groups/UPDATE_GROUP_BY_KEY");
        assert_eq!(deltas[0].payload["key"], json!("alpha"));
        assert!((deltas[0].payload["data"].as_f64().unwrap() - 0.5).abs() < 1e-6);
    }

    #[test]
    fn evaluate_every_frame_runs_once_per_frame_without_recursion() {
        let mut store = Store::new();
        store
            .restore(&preset_with_expression("value + 1", true))
            .expect("preset loads");
        store.drain_deltas();

        store.update_input_links(&AudioFrame::default(), Duration::from_millis(0));
        assert!((amount(&store) - 1.0).abs() < 1e-6);

        store.update_input_links(&AudioFrame::default(), Duration::from_millis(16));
        // Exactly one evaluation per frame: 0 → 1 → 2, never 3.
        assert!((amount(&store) - 2.0).abs() < 1e-6);

        store.update_input_links(&AudioFrame::default(), Duration::from_millis(32));
        assert!((amount(&store) - 3.0).abs() < 1e-6);
    }

    #[test]
    fn expressions_actions_mutate_state_and_emit_mirror_deltas() {
        let mut store = Store::new();
        store.restore(&expression_preset()).expect("preset loads");
        store.drain_deltas();

        let created = store
            .dispatch(
                "expressions/create",
                json!({ "id": "ex-new", "inputId": "input-a", "expression": "value * 2" }),
            )
            .expect("create dispatches");
        assert_eq!(created, json!("ex-new"));
        assert_eq!(store.state().expressions["ex-new"].expression, "value * 2");
        let deltas = store.drain_deltas();
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].r#type, "expressions/ADD_EXPRESSION");
        assert_eq!(
            deltas[0].payload["assignment"]["expression"],
            json!("value * 2")
        );
        assert_eq!(deltas[0].payload["assignment"]["inputId"], json!("input-a"));

        store
            .dispatch(
                "modules/updateProp",
                json!({
                    "moduleId": "m1",
                    "prop": "amount",
                    "data": { "value": 4.0 },
                }),
            )
            .expect("updateProp dispatches");
        assert!((amount(&store) - 8.0).abs() < 1e-6);
        store.drain_deltas();

        let updated = store
            .dispatch(
                "expressions/update",
                json!({ "id": "ex-new", "expression": "value + 5" }),
            )
            .expect("update dispatches");
        assert_eq!(updated, json!("ex-new"));
        let deltas = store.drain_deltas();
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].r#type, "expressions/ADD_EXPRESSION");

        store
            .dispatch(
                "modules/updateProp",
                json!({
                    "moduleId": "m1",
                    "prop": "amount",
                    "data": { "value": 4.0 },
                }),
            )
            .expect("updateProp dispatches");
        assert!((amount(&store) - 9.0).abs() < 1e-6);
        store.drain_deltas();

        // `updateEvaluation` persists the flag and re-announces the assignment.
        store
            .dispatch(
                "expressions/updateEvaluation",
                json!({ "id": "ex-new", "evaluateEveryFrame": true }),
            )
            .expect("updateEvaluation dispatches");
        assert!(store.state().expressions["ex-new"].evaluate_every_frame);
        let deltas = store.drain_deltas();
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].r#type, "expressions/ADD_EXPRESSION");
        assert_eq!(
            deltas[0].payload["assignment"]["evaluateEveryFrame"],
            json!(true)
        );

        store
            .dispatch("expressions/remove", json!({ "id": "ex-new" }))
            .expect("remove dispatches");
        assert!(!store.state().expressions.contains_key("ex-new"));
        let deltas = store.drain_deltas();
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].r#type, "expressions/REMOVE_EXPRESSION");
    }

    #[test]
    fn invalid_expression_leaves_values_untouched() {
        let mut store = Store::new();
        store
            .restore(&preset_with_expression("value +", false))
            .expect("preset loads");

        store
            .dispatch(
                "modules/updateProp",
                json!({
                    "moduleId": "m1",
                    "prop": "amount",
                    "data": { "value": 2.0 },
                }),
            )
            .expect("updateProp dispatches");
        assert!((amount(&store) - 2.0).abs() < 1e-6);

        // `create` with an invalid source is rejected and nothing is stored.
        let created = store
            .dispatch(
                "expressions/create",
                json!({ "id": "bad", "inputId": "input-a", "expression": "value +" }),
            )
            .expect("create dispatches");
        assert_eq!(created, Value::Null);
        assert!(!store.state().expressions.contains_key("bad"));
    }

    #[test]
    fn expression_returning_value_is_treated_as_no_assignment() {
        let mut store = Store::new();
        store.restore(&expression_preset()).expect("preset loads");
        store.drain_deltas();

        let created = store
            .dispatch(
                "expressions/create",
                json!({ "inputId": "input-a", "expression": " value " }),
            )
            .expect("create dispatches");
        assert_eq!(created, Value::Null);
        assert!(store.state().expressions.is_empty());
        assert!(store.drain_deltas().is_empty());
    }

    #[test]
    fn runtime_throwing_expression_keeps_the_previous_value() {
        let mut store = Store::new();
        store
            .restore(&preset_with_expression("inputValue.missing.deeper", false))
            .expect("preset loads");
        store.drain_deltas();

        store
            .dispatch(
                "modules/updateProp",
                json!({
                    "moduleId": "m1",
                    "prop": "amount",
                    "data": { "value": 2.0 },
                }),
            )
            .expect("updateProp dispatches");
        assert!((amount(&store) - 2.0).abs() < 1e-6);
    }
}
