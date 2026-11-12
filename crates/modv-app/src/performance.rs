//! Performance-mode runtime for the engine store (WS-8, M5c).
//!
//! This module owns the per-frame state machine the legacy worker ran inside
//! `scenes/tick` (`store/modules/scenes.js:3756-4104`) plus the start/stop
//! actions (`scenes/startPerformance`, `scenes/stopPerformance`). It is
//! deliberately kept **outside** the Vuex-shaped [`ScenesState`]: the JS
//! module keeps `rowTransitions`, `rowRuntime` and the whole-scene `transition`
//! in module-level state precisely so the per-frame tick does not flood the
//! `commitQueue`. The engine keeps the same separation — [`ScenesState`]
//! holds only the persisted config/Scene list, and [`PerformanceRuntime`]
//! lives here, emitting mirror deltas only when something visible changes
//! (transition start/end, dwell-driven Scene change, throttled progress at
//! ~10 Hz while a fade runs).
//!
//! ## Behaviour ported from `scenes.js`
//!
//! * **Per-cell dwell** — each row picks its own `pickCellDwell` target when
//!   the cell becomes active and advances independently; dwell does not
//!   advance while that row is transitioning, nor while the whole-scene
//!   crossfade runs (`scenes.js:4786-4790` returns before the row loop).
//! * **Always Fade under performance** — `pickTransitionName`
//!   (`scene-transition-pick.js`) returns `"Fade"` whenever performance mode
//!   is enabled; the allowed-transition list only matters for manual changes
//!   while performance is stopped.
//! * **Row crossfade** — the outgoing Group becomes a `transitionGhost` with
//!   the old Modules, the incoming Group is built with fresh ids (like
//!   `applySceneRowNow`, which reuses the old Group id in JS) and starts at
//!   alpha 0, both fade over `transitionSeconds`, and the ghost is removed
//!   when the fade completes.
//! * **Whole-scene transitions** — a manual `scenes/applyScene` during
//!   performance mode picks a crossfade style. `Fade` keeps the outgoing
//!   Groups alive for `transitionSeconds` and crossfades their alphas against
//!   the incoming set (exactly like `applySceneNow`'s Fade branch). One of the
//!   six native shader effects ([`crate::scene_transition`]) instead keeps the
//!   outgoing Groups at full alpha, marks them as hidden for the compositor
//!   and records a pending capture so the engine can freeze the outgoing
//!   frame; the shader then composites both Scenes itself and completion
//!   removes the outgoing Groups. `None` switches instantly.
//! * **`stopPerformance` finalises** — in-flight row/whole-scene crossfades
//!   are completed (final alphas, ghosts removed) before the flag is turned
//!   off, so nothing stale survives.
//!
//! ## Determinism
//!
//! Random picks (dwell target, `order: "random"`, manual transition pick) go
//! through [`Rng`], a small xorshift64* PRNG. Tests seed it explicitly so a
//! simulated minute is reproducible; [`Store::with_state_path`] seeds it from
//! the wall clock. This keeps the module free of an extra dependency while
//! matching the JS `Math.random()` call sites (and only calling it where JS
//! does).
//!
//! ## Not implemented here
//!
//! * Phrase detection (`phrase-detector.js`) — WS-9/M6 audio owns it. With no
//!   phrase flips the dwell deadline (`shouldAdvanceRow`'s `forced`) alone
//!   advances rows, which is the JS behaviour when the bass band is 0.
//! * Point10 pre-warming — the engine builds scenes synchronously; the
//!   `nextSceneId` field is still tracked so the selection is made once.
//! * Scene hover previews — M5d.

use std::collections::{BTreeMap, HashSet};

use serde_json::{Map, Value, json};

use modv_engine::engine_state::{GALLERY_GROUP_NAME, Group};
use modv_engine::scene_rows::{
    PickNextRowScene, ResolveRowAdvanceScene, get_row_count, is_row_cell_empty, is_scene_enabled,
    normalize_row_names, pick_next_row_scene, resolve_row_advance_scene,
};
use modv_engine::scene_timing::{
    normalize_transition_duration, pick_cell_dwell_numeric, resolve_scene_timing,
};
use modv_engine::scene_transition_pick::{
    FADE_TRANSITION, NATIVE_TRANSITION_NAMES, NONE_TRANSITION, pick_transition_name,
};
use modv_engine::scenes_playlist::normalize_transition_list;

use crate::scene_ops::RowApplyOutcome;
use crate::store_ops::{Store, StoreError};

/// How long a cell plays before its next Scene is selected
/// (`ROW_PREWARM_AFTER_SECONDS`, `scenes.js:1084`). The engine has no GPU
/// pre-warm yet, but the selection is still made once at this point so the
/// advance uses the Scene chosen mid-dwell (`resolveRowAdvanceScene`).
pub const ROW_PREWARM_AFTER_SECONDS: f64 = 0.5;

/// Minimum interval between mirror progress/alpha deltas while a crossfade
/// runs (`scenes.js:4795-4815`, `lastRowProgressCommit`).
pub const PROGRESS_EMIT_INTERVAL_SECONDS: f64 = 0.1;

/// Dwell/phrase time is clamped per frame (`scenes.js:3770-3775`); the
/// transition itself advances in real time.
pub const MAX_TICK_DT_SECONDS: f64 = 0.1;

/// Small deterministic PRNG (xorshift64*) for the pick sites that use
/// `Math.random()` in JS.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rng {
    state: u64,
}

impl Rng {
    /// Seed the generator. Zero is remapped to a fixed non-zero state because
    /// xorshift cannot leave zero.
    pub fn seeded(seed: u64) -> Self {
        Self {
            state: if seed == 0 {
                0x9E37_79B9_7F4A_7C15
            } else {
                seed
            },
        }
    }

    /// Next `f64` in `[0, 1)` (`Math.random` shape).
    pub fn next_f64(&mut self) -> f64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        let value = x.wrapping_mul(0x2545_F491_4F6C_DD1D);
        // 53 significant bits keeps the value in [0, 1).
        (value >> 11) as f64 / (1u64 << 53) as f64
    }
}

impl Default for Rng {
    fn default() -> Self {
        Self::seeded(0x1234_5678_9ABC_DEF0)
    }
}

/// One row's dwell state (`rowRuntime`, `scenes.js:1240-1295`).
#[derive(Clone, Debug, PartialEq)]
pub struct RowRuntime {
    /// The Scene the row is playing.
    pub current_scene_id: Option<String>,
    /// The live Group that Scene occupies on this row.
    pub group_id: Option<String>,
    /// `pickCellDwell` target picks once when the cell becomes active.
    pub dwell_target: f64,
    /// Seconds the cell has been active (does not advance transitioning).
    pub dwell_elapsed: f64,
    /// Scene chosen mid-dwell for the next advance (`runtime.nextSceneId`).
    pub next_scene_id: Option<String>,
    /// Whether the mid-dwell pick already happened.
    pub pick_pending: bool,
}

impl RowRuntime {
    /// A freshly activated cell with the given dwell target.
    pub fn new(scene_id: Option<String>, group_id: Option<String>, dwell_target: f64) -> Self {
        Self {
            current_scene_id: scene_id,
            group_id,
            dwell_target,
            dwell_elapsed: 0.0,
            next_scene_id: None,
            pick_pending: false,
        }
    }
}

/// An in-flight row crossfade (`rowTransitions[row]`, `scenes.js:2502-2512`).
#[derive(Clone, Debug, PartialEq)]
pub struct RowTransition {
    /// Row index.
    pub row: usize,
    /// The incoming (and once-fading, surviving) Group id.
    pub group_id: String,
    /// The outgoing Group kept alive for the fade (`transitionGhost`).
    pub ghost_group_id: String,
    /// Alpha the ghost started at.
    pub from_alpha: f64,
    /// Alpha the incoming Group fades to.
    pub target_alpha: f64,
    /// Seconds elapsed.
    pub elapsed: f64,
    /// Total fade duration (`transitionSeconds`).
    pub duration: f64,
}

impl RowTransition {
    /// `min(1, elapsed / duration)`, guarding a non-positive duration.
    pub fn progress(&self) -> f64 {
        let duration = if self.duration.is_finite() && self.duration > 0.0 {
            self.duration
        } else {
            normalize_transition_duration(None)
        };
        (self.elapsed / duration).min(1.0)
    }

    /// The incoming half's current alpha (`targetAlpha * eased`).
    pub fn incoming_alpha(&self) -> f64 {
        self.target_alpha * smoothstep(self.progress())
    }

    /// The outgoing half's current alpha (`fromAlpha * (1 - eased)`).
    pub fn outgoing_alpha(&self) -> f64 {
        self.from_alpha * (1.0 - smoothstep(self.progress()))
    }
}

/// An in-flight whole-scene transition (`transition`, `scenes.js:2765-2777`).
#[derive(Clone, Debug, PartialEq)]
pub struct WholeSceneTransition {
    /// Outgoing Groups with the alpha they started at.
    pub from: Vec<(String, f64)>,
    /// Incoming Groups with their target alpha.
    pub to: Vec<(String, f64)>,
    /// Transition name. [`FADE_TRANSITION`] runs the per-Group alpha path; a
    /// native name (`NONE_TRANSITION` never reaches here) runs the matching
    /// [`crate::scene_transition::TransitionEffect`] over the captured outgoing
    /// frame.
    pub kind: String,
    /// Seconds elapsed.
    pub elapsed: f64,
    /// Total duration (`transitionSeconds`).
    pub duration: f64,
}

impl WholeSceneTransition {
    /// `min(1, elapsed / duration)`.
    pub fn progress(&self) -> f64 {
        let duration = if self.duration.is_finite() && self.duration > 0.0 {
            self.duration
        } else {
            normalize_transition_duration(None)
        };
        (self.elapsed / duration).min(1.0)
    }

    /// The shader effect this transition runs, if any.
    ///
    /// An unknown name (a custom ISF transition the engine cannot load) is not
    /// a shader here: it degrades to the Fade alpha path, which is the JS
    /// `fallbackToFade` behaviour.
    pub fn shader_effect(&self) -> Option<crate::scene_transition::TransitionEffect> {
        crate::scene_transition::TransitionEffect::from_name(&self.kind)
    }
}

/// The whole performance-mode runtime.
#[derive(Clone, Debug)]
pub struct PerformanceRuntime {
    /// Whether performance mode is running (`state.performance.enabled`).
    pub enabled: bool,
    /// Row runtimes keyed by row index (`rowRuntime`).
    pub rows: BTreeMap<usize, RowRuntime>,
    /// In-flight row crossfades keyed by row index (`rowTransitions`).
    pub row_transitions: BTreeMap<usize, RowTransition>,
    /// In-flight whole-scene crossfade (`transition`).
    pub transition: Option<WholeSceneTransition>,
    /// Deterministic RNG for dwell/order/transition picks.
    pub rng: Rng,
    /// Accumulated seconds since the last throttled progress delta.
    pub(crate) progress_emit_elapsed: f64,
}

impl Default for PerformanceRuntime {
    fn default() -> Self {
        Self {
            enabled: false,
            rows: BTreeMap::new(),
            row_transitions: BTreeMap::new(),
            transition: None,
            rng: Rng::default(),
            progress_emit_elapsed: 0.0,
        }
    }
}

impl PerformanceRuntime {
    /// `true` when a tick has work to do: performance mode is running or any
    /// crossfade is active. The window loop uses this to skip `scenes/tick`
    /// on idle frames.
    pub fn is_active(&self) -> bool {
        self.enabled || !self.row_transitions.is_empty() || self.transition.is_some()
    }
}

/// `renderRowFade`'s easing (`row-transition.js:39-62`): `smoothstep`.
pub(crate) fn smoothstep(progress: f64) -> f64 {
    let clamped = if progress.is_finite() {
        progress.clamp(0.0, 1.0)
    } else {
        0.0
    };
    clamped * clamped * (3.0 - 2.0 * clamped)
}

impl Store {
    /// The performance runtime (tests / diagnostics).
    pub fn performance_runtime(&self) -> &PerformanceRuntime {
        &self.performance
    }

    /// The runtime settings for one row, if the row is running.
    #[allow(dead_code)]
    pub fn row_runtime(&self, row: usize) -> Option<&RowRuntime> {
        self.performance.rows.get(&row)
    }

    /// The in-flight row crossfade, if any.
    #[allow(dead_code)]
    pub fn row_transition(&self, row: usize) -> Option<&RowTransition> {
        self.performance.row_transitions.get(&row)
    }

    /// The in-flight whole-scene crossfade, if any.
    #[allow(dead_code)]
    pub fn whole_scene_transition(&self) -> Option<&WholeSceneTransition> {
        self.performance.transition.as_ref()
    }

    /// `true` when `scenes/tick` has work to do (`loop.js:94-111` always
    /// dispatched it; the engine can skip it on idle frames).
    pub fn scenes_tick_due(&self) -> bool {
        self.performance.is_active()
    }

    /// Live performance rows: every non-Gallery, non-ghost Group in draw
    /// order (`getLiveRowGroups`, `scenes.js:1088-1094`).
    ///
    /// While a whole-scene crossfade runs, the outgoing (`transition.from`)
    /// Groups are still in the layer stack — sorted *before* the incoming ones
    /// so the incoming Scene draws on top — but they are no longer live rows.
    /// Row lookups must resolve to the incoming Groups, so they are excluded
    /// here (R-29).
    fn live_row_groups(&self) -> Vec<Group> {
        let outgoing: HashSet<&str> = self
            .performance
            .transition
            .as_ref()
            .map(|transition| {
                transition
                    .from
                    .iter()
                    .map(|(group_id, _)| group_id.as_str())
                    .collect()
            })
            .unwrap_or_default();
        self.state
            .groups
            .iter()
            .filter(|group| {
                group.name != GALLERY_GROUP_NAME
                    && group.transition_ghost != Some(true)
                    && !outgoing.contains(group.id.as_str())
            })
            .cloned()
            .collect()
    }

    /// `findLiveRowGroup` (`scenes.js:1116-1128`): the Group stamped with the
    /// row, falling back to the row's position in the live stack.
    pub(crate) fn find_live_row_group(&self, row: usize) -> Option<Group> {
        let groups = self.live_row_groups();
        groups
            .iter()
            .find(|group| group.scene_row == Some(row))
            .or_else(|| groups.get(row))
            .cloned()
    }

    /// The live row count (`getters["scenes/rowCount"]`).
    pub(crate) fn performance_row_count(&self) -> usize {
        self.row_count_for(&self.row_names())
    }

    /// [`Self::performance_row_count`] against already-normalised row names, so
    /// the per-frame tick does not re-normalise (and re-clone) them for every
    /// call.
    fn row_count_for(&self, row_names: &[String]) -> usize {
        let live: Vec<Value> = self
            .live_row_groups()
            .iter()
            .map(|group| serde_json::to_value(group).unwrap_or(Value::Null))
            .collect();
        get_row_count(&self.scenes.scenes, &live, row_names)
    }

    /// Row names from the performance config.
    fn row_names(&self) -> Vec<String> {
        normalize_row_names(self.scenes.performance.get("rowNames"))
    }

    /// Pick the next Scene for one row (`pickNextRowScene`).
    ///
    /// `row_names` is passed in so a caller that already normalised them (the
    /// per-frame tick, which asks for every row) does not rebuild the list.
    fn pick_next_scene_for_row(
        &mut self,
        row: usize,
        current_scene_id: Option<&str>,
        row_names: &[String],
    ) -> Option<String> {
        let order = self
            .scenes
            .performance
            .get("order")
            .and_then(Value::as_str)
            .unwrap_or("in-order")
            .to_string();
        let scenes = &self.scenes.scenes;
        let picked = pick_next_row_scene(
            &PickNextRowScene {
                scenes,
                row_index: row,
                current_scene_id,
                order: &order,
                row_names,
            },
            || self.performance.rng.next_f64(),
        );
        picked
            .and_then(|scene| scene.get("id").and_then(Value::as_str))
            .map(str::to_string)
    }

    /// `pickNextScene` for the legacy whole-Scene autopilot (`scenes.js:3256`).
    pub(crate) fn pick_next_whole_scene(&mut self) -> Option<String> {
        let order = self
            .scenes
            .performance
            .get("order")
            .and_then(Value::as_str)
            .unwrap_or("in-order")
            .to_string();
        let current = self
            .scenes
            .performance
            .get("currentSceneId")
            .and_then(Value::as_str)
            .map(str::to_string);
        let enabled: Vec<&Value> = self
            .scenes
            .scenes
            .iter()
            .filter(|scene| is_scene_enabled(scene))
            .collect();
        if enabled.is_empty() {
            return None;
        }
        let chosen = if order == "random" && enabled.len() > 1 {
            let pool: Vec<&Value> = enabled
                .iter()
                .copied()
                .filter(|scene| scene.get("id").and_then(Value::as_str) != current.as_deref())
                .collect();
            let choices = if pool.is_empty() { &enabled } else { &pool };
            let index = (self.performance.rng.next_f64() * choices.len() as f64).floor();
            let index = index.clamp(0.0, (choices.len() - 1) as f64) as usize;
            choices[index]
        } else {
            let index = current
                .as_deref()
                .and_then(|id| {
                    enabled
                        .iter()
                        .position(|scene| scene.get("id").and_then(Value::as_str) == Some(id))
                })
                .map(|index| (index + 1) % enabled.len())
                .unwrap_or(0);
            enabled[index]
        };
        chosen.get("id").and_then(Value::as_str).map(str::to_string)
    }

    /// `pickTransitionName` with the engine's available set.
    ///
    /// The engine executes `Fade` (the per-Group alpha path) and the six native
    /// shader effects ([`crate::scene_transition`]); `None` is preserved so a
    /// manual change can still switch instantly. An allowed name that is not
    /// available (a custom ISF transition the engine cannot load) is filtered
    /// out by [`pick_transition_name`] and the change fades instead, which is
    /// the JS `fallbackToFade` path — the Scene change itself never fails.
    pub(crate) fn pick_scene_transition_name(&mut self) -> String {
        let enabled = self.performance.enabled;
        let allowed = normalize_transition_list(self.scenes.performance.get("allowedTransitions"));
        let mut available = vec![FADE_TRANSITION.to_string(), NONE_TRANSITION.to_string()];
        available.extend(NATIVE_TRANSITION_NAMES.iter().map(|name| name.to_string()));
        pick_transition_name(enabled, &allowed, &available, || {
            self.performance.rng.next_f64()
        })
    }

    /// Create `RowRuntime` for a cell using the Scene's resolved timing.
    pub(crate) fn reset_row_runtime(
        &mut self,
        row: usize,
        scene_id: Option<String>,
        group_id: Option<String>,
    ) {
        let scene = scene_id.as_deref().and_then(|id| {
            self.scenes
                .scenes
                .iter()
                .find(|scene| scene.get("id").and_then(Value::as_str) == Some(id))
        });
        let scene_timing = resolve_scene_timing(scene, Some(&self.scenes.performance));
        let roll = self.performance.rng.next_f64();
        let target = pick_cell_dwell_numeric(
            Some(scene_timing.min_seconds),
            Some(scene_timing.max_seconds),
            Some(roll),
        );
        self.performance
            .rows
            .insert(row, RowRuntime::new(scene_id, group_id, target));
    }

    /// `scenes/startPerformance` (`scenes.js:3165-3250`).
    ///
    /// Enables performance mode, keeps every row that is already playing a
    /// valid cell, applies an initial Scene to the remaining rows, and starts
    /// all dwells. When no row has any candidate cell the legacy whole-Scene
    /// autopilot takes over (`nextScene`).
    pub fn start_performance(&mut self) -> Result<Value, StoreError> {
        self.complete_all_row_transitions();
        self.complete_whole_scene_transition();
        self.performance.rows.clear();
        self.performance.enabled = true;
        self.set_performance_value("enabled", json!(true));

        let row_names = self.row_names();
        let row_count = self.row_count_for(&row_names);
        if row_count == 0 {
            return Ok(json!({ "enabled": true, "rows": 0 }));
        }

        let has_row_candidates = (0..row_count).any(|row| {
            self.scenes
                .scenes
                .iter()
                .any(|scene| is_scene_enabled(scene) && !is_row_cell_empty(scene, row, &row_names))
        });

        if !has_row_candidates {
            let current_valid = self
                .scenes
                .performance
                .get("currentSceneId")
                .and_then(Value::as_str)
                .and_then(|id| {
                    self.scenes
                        .scenes
                        .iter()
                        .find(|scene| scene.get("id").and_then(Value::as_str) == Some(id))
                })
                .is_some_and(is_scene_enabled);
            if !current_valid {
                if let Some(scene_id) = self.pick_next_whole_scene() {
                    self.apply_scene_action(&single_param("sceneId", &scene_id))?;
                }
            }
            return Ok(json!({ "enabled": true, "rows": 0 }));
        }

        // Keep rows that already hold a valid cell; apply an initial Scene to
        // the rest (`scenes.js:3195-3240`).
        for row in 0..row_count {
            let live = self.find_live_row_group(row);
            let active_scene_id = live
                .as_ref()
                .and_then(|group| group.scene_id.as_deref())
                .map(str::to_string);
            let valid = match (&live, &active_scene_id) {
                (Some(_), Some(scene_id)) => self
                    .scenes
                    .scenes
                    .iter()
                    .find(|scene| scene.get("id").and_then(Value::as_str) == Some(scene_id))
                    .is_some_and(|scene| {
                        is_scene_enabled(scene) && !is_row_cell_empty(scene, row, &row_names)
                    }),
                _ => false,
            };

            if !valid {
                if let Some(scene_id) =
                    self.pick_next_scene_for_row(row, active_scene_id.as_deref(), &row_names)
                {
                    self.apply_scene_row_transition(&scene_id, row)?;
                }
            }
        }

        for row in 0..row_count {
            let live = self.find_live_row_group(row);
            let scene_id = live
                .as_ref()
                .and_then(|group| group.scene_id.clone())
                .or_else(|| {
                    self.performance
                        .rows
                        .get(&row)
                        .and_then(|runtime| runtime.current_scene_id.clone())
                });
            let group_id = live.as_ref().map(|group| group.id.clone());
            self.reset_row_runtime(row, scene_id, group_id);
        }

        self.persist_best_effort();
        Ok(json!({ "enabled": true, "rows": row_count }))
    }

    /// `scenes/stopPerformance` (`scenes.js:3248-3254`).
    ///
    /// Completes every in-flight crossfade (final alphas, ghosts removed)
    /// before disabling, so no stale transition state or orphaned Group
    /// survives the stop.
    pub fn stop_performance(&mut self) -> Result<Value, StoreError> {
        self.complete_all_row_transitions();
        self.complete_whole_scene_transition();
        self.performance.rows.clear();
        self.performance.progress_emit_elapsed = 0.0;
        self.performance.enabled = false;
        self.set_performance_value("enabled", json!(false));
        self.deltas
            .push("scenes/SET_ACTIVE_ROWS", self.live_scene_rows());
        self.persist_best_effort();
        Ok(json!({ "enabled": false }))
    }

    /// `scenes/tick` (`scenes.js:3756-4104`).
    ///
    /// `delta_ms` is the real frame delta in milliseconds (`loop.js:94-111`).
    /// Returns the number of rows that changed Scene this frame.
    pub fn tick_scenes(&mut self, delta_ms: f64) -> Result<Value, StoreError> {
        let numeric = if delta_ms.is_finite() {
            delta_ms.max(0.0) / 1000.0
        } else {
            0.0
        };
        let dt = numeric.min(MAX_TICK_DT_SECONDS);

        self.performance.progress_emit_elapsed += numeric;
        let emit_progress =
            self.performance.progress_emit_elapsed >= PROGRESS_EMIT_INTERVAL_SECONDS;
        if emit_progress {
            self.performance.progress_emit_elapsed = 0.0;
        }

        // 1. Advance row crossfades in real time (`scenes.js:3778-3868`).
        let rows: Vec<usize> = self.performance.row_transitions.keys().copied().collect();
        for row in rows {
            let Some(transition) = self.performance.row_transitions.get(&row).cloned() else {
                continue;
            };
            let elapsed = transition.elapsed + numeric;
            let progress = if transition.duration > 0.0 {
                (elapsed / transition.duration).min(1.0)
            } else {
                1.0
            };
            let mut current = transition.clone();
            current.elapsed = elapsed;
            self.set_group_alpha(&current.group_id, current.incoming_alpha(), emit_progress);
            self.set_group_alpha(
                &current.ghost_group_id,
                current.outgoing_alpha(),
                emit_progress,
            );

            if progress >= 1.0 {
                self.complete_row_transition(row);
            } else {
                self.performance.row_transitions.insert(row, current);
                if emit_progress {
                    self.deltas.push(
                        "scenes/SET_ROW_PROGRESS",
                        json!({ "rowIndex": row, "progress": progress }),
                    );
                }
            }
        }

        // 2. The whole-scene crossfade owns the frame; dwell waits
        //    (`scenes.js:4786-4790`).
        if self.performance.transition.is_some() {
            self.advance_whole_scene_transition(numeric, emit_progress);
            return Ok(json!({ "advanced": 0 }));
        }

        if !self.performance.enabled {
            return Ok(json!({ "advanced": 0 }));
        }

        // 3. Per-row dwell (`scenes.js:3900-4105`).
        let row_names = self.row_names();
        let row_count = self.row_count_for(&row_names);
        let order = self
            .scenes
            .performance
            .get("order")
            .and_then(Value::as_str)
            .unwrap_or("in-order")
            .to_string();
        let mut advanced = 0usize;

        for row in 0..row_count {
            if self.performance.row_transitions.contains_key(&row) {
                continue;
            }

            let active = self
                .performance
                .rows
                .get(&row)
                .cloned()
                .unwrap_or_else(|| RowRuntime::new(None, None, f64::INFINITY));
            let current_scene = active.current_scene_id.as_deref().and_then(|id| {
                self.scenes
                    .scenes
                    .iter()
                    .find(|scene| scene.get("id").and_then(Value::as_str) == Some(id))
            });
            let live = self.find_live_row_group(row);
            let valid = match (&live, current_scene) {
                (Some(group), Some(scene)) => {
                    is_scene_enabled(scene)
                        && !is_row_cell_empty(scene, row, &row_names)
                        && active
                            .group_id
                            .as_deref()
                            .map(|id| id == group.id)
                            .unwrap_or(true)
                }
                _ => false,
            };

            if !valid {
                if let Some(scene_id) = self.pick_next_scene_for_row(
                    row,
                    active.current_scene_id.as_deref(),
                    &row_names,
                ) {
                    self.apply_scene_row_transition(&scene_id, row)?;
                    advanced += 1;
                }
                continue;
            }

            // A single candidate cell never changes (`scenes.js:3970-3978`).
            let candidate_count = self
                .scenes
                .scenes
                .iter()
                .filter(|scene| {
                    is_scene_enabled(scene) && !is_row_cell_empty(scene, row, &row_names)
                })
                .count();
            if candidate_count < 2 {
                continue;
            }

            let scene = current_scene.expect("validated above");
            let scene_id = scene.get("id").and_then(Value::as_str).map(str::to_string);

            // Pick the next Scene once, mid-dwell, so the advance is stable
            // even if another Scene is added meanwhile (`scenes.js:4035-4064`).
            let mut runtime = self
                .performance
                .rows
                .remove(&row)
                .unwrap_or_else(|| RowRuntime::new(scene_id.clone(), None, f64::INFINITY));
            if runtime.current_scene_id.is_none() {
                runtime.current_scene_id = scene_id.clone();
            }
            if runtime.group_id.is_none() {
                runtime.group_id = live.as_ref().map(|group| group.id.clone());
            }
            if !runtime.pick_pending && runtime.dwell_elapsed >= ROW_PREWARM_AFTER_SECONDS {
                runtime.pick_pending = true;
                runtime.next_scene_id =
                    self.pick_next_scene_for_row(row, scene_id.as_deref(), &row_names);
            }

            runtime.dwell_elapsed += dt;

            if runtime.dwell_elapsed + f64::EPSILON < runtime.dwell_target {
                self.performance.rows.insert(row, runtime);
                continue;
            }

            let pending_scene_id = runtime.next_scene_id.clone();
            self.performance.rows.insert(row, runtime);

            let next_scene = {
                let scenes = &self.scenes.scenes;
                resolve_row_advance_scene(
                    &ResolveRowAdvanceScene {
                        scenes,
                        row_index: row,
                        current_scene_id: scene_id.as_deref(),
                        pending_scene_id: pending_scene_id.as_deref(),
                        order: &order,
                        row_names: &row_names,
                    },
                    || self.performance.rng.next_f64(),
                )
                .and_then(|scene| scene.get("id").and_then(Value::as_str))
                .map(str::to_string)
            };

            if let Some(next_scene) = next_scene {
                self.apply_scene_row_transition(&next_scene, row)?;
                advanced += 1;
            }
        }

        Ok(json!({ "advanced": advanced }))
    }

    /// Apply one Scene to one row and start its crossfade. Resets the row's
    /// dwell target (`applySceneRowNow`, `scenes.js:2250-2525`).
    pub(crate) fn apply_scene_row_transition(
        &mut self,
        scene_id: &str,
        row: usize,
    ) -> Result<Option<RowApplyOutcome>, StoreError> {
        let Some(outcome) = self.apply_scene_row_now(scene_id, row, true) else {
            return Ok(None);
        };
        self.reset_row_runtime(
            row,
            Some(scene_id.to_string()),
            Some(outcome.group_id.clone()),
        );
        if let Some(ghost_id) = &outcome.ghost_group_id {
            self.performance.row_transitions.insert(
                row,
                RowTransition {
                    row,
                    group_id: outcome.group_id.clone(),
                    ghost_group_id: ghost_id.clone(),
                    from_alpha: outcome.from_alpha,
                    target_alpha: outcome.target_alpha,
                    elapsed: 0.0,
                    duration: outcome.duration,
                },
            );
            self.deltas.push(
                "scenes/SET_ROW_PROGRESS",
                json!({ "rowIndex": row, "progress": 0.0 }),
            );
        }
        if self.performance.enabled {
            self.persist_best_effort();
        }
        Ok(Some(outcome))
    }

    /// Finish one row crossfade: bake the target alpha, remove the ghost Group
    /// and its Modules, drop the transition state (`completeRowTransition`,
    /// `scenes.js:1150-1178`).
    pub(crate) fn complete_row_transition(&mut self, row: usize) {
        let Some(transition) = self.performance.row_transitions.remove(&row) else {
            return;
        };
        self.set_group_alpha(&transition.group_id, transition.target_alpha, true);
        self.remove_live_group(&transition.ghost_group_id);
        self.deltas
            .push("scenes/CLEAR_ROW_PROGRESS", json!({ "rowIndex": row }));
        // The dwell target was chosen when the incoming Scene was applied and
        // did not advance during the fade, so restarting is a no-op except for
        // clearing the mid-dwell pick.
        if let Some(runtime) = self.performance.rows.get_mut(&row) {
            runtime.dwell_elapsed = 0.0;
            runtime.next_scene_id = None;
            runtime.pick_pending = false;
        }
    }

    /// Finish every in-flight row crossfade (`completeAllRowTransitions`).
    pub(crate) fn complete_all_row_transitions(&mut self) {
        let rows: Vec<usize> = self.performance.row_transitions.keys().copied().collect();
        for row in &rows {
            self.complete_row_transition(*row);
        }
        if !rows.is_empty() {
            self.deltas.push("scenes/CLEAR_ALL_ROW_PROGRESS", json!({}));
        }
    }

    /// `completeTransition` for the whole-scene transition (`scenes.js:1870-1890`):
    /// bake the incoming alphas, remove the outgoing Groups, clear the flag.
    pub(crate) fn complete_whole_scene_transition(&mut self) {
        self.pending_transition_capture = None;
        let Some(transition) = self.performance.transition.take() else {
            return;
        };
        for (group_id, alpha) in &transition.to {
            self.set_group_alpha(group_id, *alpha, true);
        }
        for (group_id, _) in &transition.from {
            self.remove_live_group(group_id);
        }
        self.deltas.push("scenes/SET_TRANSITIONING", json!(false));
    }

    /// Record the whole-scene transition started by `applySceneNow`.
    ///
    /// A native shader kind leaves a pending capture request for the window
    /// loop (see [`Store::take_pending_transition_capture`]); `Fade` and
    /// unknown kinds do not, and keep the alpha path.
    pub(crate) fn begin_whole_scene_transition(
        &mut self,
        from: Vec<(String, f64)>,
        to: Vec<(String, f64)>,
        kind: String,
    ) {
        let duration =
            normalize_transition_duration(self.scenes.performance.get("transitionSeconds"));
        self.pending_transition_capture =
            crate::scene_transition::TransitionEffect::from_name(&kind);
        self.performance.transition = Some(WholeSceneTransition {
            from,
            to,
            kind,
            elapsed: 0.0,
            duration,
        });
        self.performance.progress_emit_elapsed = 0.0;
        self.deltas.push("scenes/SET_TRANSITIONING", json!(true));
    }

    /// Take the capture request a just-started shader transition left behind.
    ///
    /// The window loop calls this before rendering the first frame of the
    /// incoming Scene: main still holds the outgoing Scene at that point, so
    /// the compositor can copy it once. The request is consumed either way.
    pub fn take_pending_transition_capture(
        &mut self,
    ) -> Option<crate::scene_transition::TransitionEffect> {
        self.pending_transition_capture.take()
    }

    /// Downgrade the active whole-scene transition to the Fade alpha path.
    ///
    /// Used when the outgoing frame could not be captured (no main texture yet,
    /// a resize invalidated it, or the effect failed to compile). The change is
    /// never dropped: the incoming Groups are reset to alpha 0 and the outgoing
    /// ones to the alpha they recorded when the change started — exactly the
    /// Fade setup — so `scenes/tick` crossfades them as usual and the Scene
    /// change still completes. Returns `true` when a shader transition was
    /// downgraded.
    pub fn fall_back_scene_transition_to_fade(&mut self) -> bool {
        self.pending_transition_capture = None;
        let Some(transition) = self.performance.transition.as_ref() else {
            return false;
        };
        if transition.shader_effect().is_none() {
            return false;
        }
        let from = transition.from.clone();
        let to = transition.to.clone();
        if let Some(transition) = self.performance.transition.as_mut() {
            transition.kind = FADE_TRANSITION.to_string();
        }
        for (group_id, _) in &to {
            self.set_group_alpha(group_id, 0.0, true);
        }
        for (group_id, alpha) in &from {
            self.set_group_alpha(group_id, *alpha, true);
        }
        true
    }

    /// Advance the whole-scene transition (`scenes.js:3860-3910`).
    ///
    /// A shader transition advances only its timeline: both Scenes render at
    /// their final opacity and the effect does the blending, exactly like the
    /// JS `if (!transition.shader)` guard around the alpha loop. The Fade path
    /// keeps animating the Groups' alphas.
    fn advance_whole_scene_transition(&mut self, delta: f64, emit_progress: bool) {
        let Some(mut transition) = self.performance.transition.clone() else {
            return;
        };
        transition.elapsed += delta;
        let progress = transition.progress();

        if transition.shader_effect().is_none() {
            let eased = smoothstep(progress);
            for (group_id, alpha) in &transition.to {
                self.set_group_alpha(group_id, alpha * eased, emit_progress);
            }
            for (group_id, alpha) in &transition.from {
                self.set_group_alpha(group_id, alpha * (1.0 - eased), emit_progress);
            }
        }

        if progress >= 1.0 {
            self.performance.transition = Some(transition);
            self.complete_whole_scene_transition();
        } else {
            self.performance.transition = Some(transition);
        }
    }

    /// Set a Group's alpha and (optionally) mirror it.
    fn set_group_alpha(&mut self, group_id: &str, alpha: f64, emit: bool) {
        let alpha = if alpha.is_finite() {
            alpha.clamp(0.0, 1.0)
        } else {
            1.0
        };
        if let Some(group) = self.state.find_group_mut(group_id) {
            group.alpha = alpha;
        }
        if emit {
            self.deltas.push(
                "groups/UPDATE_GROUP",
                json!({ "groupId": group_id, "data": { "alpha": alpha } }),
            );
        }
    }
}

/// Build a one-key parameter map for calling an action helper directly.
fn single_param(key: &str, value: &str) -> Map<String, Value> {
    let mut params = Map::new();
    params.insert(key.to_string(), json!(value));
    params
}

#[cfg(test)]
mod tests {
    //! Deterministic runtime tests (no GPU). The compositor's row-fade blend
    //! is covered in `compositor.rs` (lavapipe).
    use super::*;
    use crate::store_ops::Store;
    use modv_engine::delta::Mutation;
    use serde_json::json;

    fn scene(id: &str, name: &str) -> Value {
        json!({
            "id": id,
            "name": name,
            "groups": [
                {
                    "id": format!("{id}-bg"),
                    "name": "Background",
                    "hidden": false,
                    "modules": [format!("{id}-bands")],
                    "enabled": 1,
                    "alpha": 1,
                    "inherit": false,
                    "inheritFrom": -1,
                    "pipeline": false,
                    "clearing": true,
                    "compositeOperation": "normal",
                },
                {
                    "id": format!("{id}-fx"),
                    "name": "Effects",
                    "hidden": false,
                    "modules": [format!("{id}-ball")],
                    "enabled": 1,
                    "alpha": 1,
                    "inherit": true,
                    "inheritFrom": format!("{id}-bg"),
                    "pipeline": false,
                    "clearing": true,
                    "compositeOperation": "normal",
                },
            ],
            "modules": {
                format!("{id}-bands"): {
                    "$id": format!("{id}-bands"),
                    "$moduleName": "Bands",
                    "meta": { "name": "Bands", "enabled": true },
                    "props": {},
                    "data": {},
                    "$props": {},
                },
                format!("{id}-ball"): {
                    "$id": format!("{id}-ball"),
                    "$moduleName": "Ball",
                    "meta": { "name": "Ball", "enabled": true },
                    "props": { "amount": 3 },
                    "data": {},
                    "$props": {},
                },
            },
        })
    }

    fn store_with_scenes() -> Store {
        let mut store = Store::new();
        for (id, name) in [
            ("scene-a", "Alpha"),
            ("scene-b", "Beta"),
            ("scene-c", "Gamma"),
        ] {
            store
                .dispatch(
                    "scenes/loadSceneFromJson",
                    json!({ "scene": scene(id, name), "persist": false }),
                )
                .expect("scene loads");
        }
        store.performance.rng = Rng::seeded(7);
        store
            .dispatch(
                "scenes/updatePerformance",
                json!({
                    "config": {
                        "minSeconds": 1,
                        "maxSeconds": 2,
                        "transitionSeconds": 1,
                        "order": "in-order",
                        "allowedTransitions": ["None"],
                    }
                }),
            )
            .expect("performance config");
        store.drain_deltas();
        store
    }

    fn delta_types(store: &mut Store) -> Vec<String> {
        store
            .drain_deltas()
            .into_iter()
            .map(|mutation| mutation.r#type)
            .collect()
    }

    fn tick(store: &mut Store, seconds: f64) {
        store
            .dispatch("scenes/tick", json!({ "delta": seconds * 1000.0 }))
            .expect("tick dispatches");
    }

    /// Simulate `seconds` at 60 fps (the tick clamps dwell time per frame, so
    /// a single large delta would not advance the dwell).
    fn advance(store: &mut Store, seconds: f64) {
        let steps = (seconds * 60.0).ceil() as usize;
        for _ in 0..steps {
            tick(store, 1.0 / 60.0);
        }
    }

    #[test]
    fn start_performance_applies_every_row_and_starts_dwells() {
        let mut store = store_with_scenes();
        store
            .dispatch("scenes/startPerformance", json!({}))
            .expect("startPerformance dispatches");

        assert!(store.performance_runtime().enabled);
        assert_eq!(store.performance_runtime().rows.len(), 2);
        let live: Vec<&Group> = store
            .state()
            .groups
            .iter()
            .filter(|group| group.name == "Background" || group.name == "Effects")
            .collect();
        assert_eq!(live.len(), 2, "both rows have a live Group");
        for row in 0..2 {
            let runtime = store.row_runtime(row).expect("row runtime");
            assert!(runtime.current_scene_id.is_some());
            assert!(
                runtime.dwell_target >= 1.0 && runtime.dwell_target <= 2.0,
                "per-cell target in [min, max]: {}",
                runtime.dwell_target
            );
        }
    }

    #[test]
    fn rows_with_different_dwell_targets_advance_independently() {
        let mut store = store_with_scenes();
        store
            .dispatch("scenes/startPerformance", json!({}))
            .expect("start");
        // Make the two rows' deadlines unambiguous.
        store.performance.rows.get_mut(&0).unwrap().dwell_target = 1.5;
        store.performance.rows.get_mut(&1).unwrap().dwell_target = 5.0;
        store.drain_deltas();

        // Advance just past the first row's deadline.
        for _ in 0..100 {
            tick(&mut store, 1.0 / 60.0);
        }

        let first = store.row_transition(0).expect("row 0 started a crossfade");
        assert_eq!(first.from_alpha, 1.0);
        assert!(store.row_transition(1).is_none(), "row 1 is still dwelling");
        let second = store.row_runtime(1).expect("row 1 runtime");
        assert!(
            (second.dwell_elapsed - 100.0 / 60.0).abs() < 0.01,
            "row 1 kept dwelling: {}",
            second.dwell_elapsed
        );
    }

    #[test]
    fn dwell_does_not_advance_during_a_row_transition_and_restarts_after() {
        let mut store = store_with_scenes();
        store
            .dispatch("scenes/startPerformance", json!({}))
            .expect("start");
        store.performance.rows.get_mut(&0).unwrap().dwell_target = 1.0;
        store.drain_deltas();

        advance(&mut store, 1.05);
        let transition = store
            .row_transition(0)
            .cloned()
            .expect("transition started");
        assert_eq!(transition.duration, 1.0);
        assert_eq!(store.row_runtime(0).unwrap().dwell_elapsed, 0.0);

        // Still mid-transition: dwell must not move.
        tick(&mut store, 0.4);
        assert_eq!(store.row_runtime(0).unwrap().dwell_elapsed, 0.0);
        assert!(store.row_transition(0).is_some());

        // Finish the fade; the row's dwell starts again from zero (the same
        // tick then accrues at most its clamped dt).
        tick(&mut store, 0.65);
        assert!(store.row_transition(0).is_none());
        let restarted = store.row_runtime(0).unwrap().dwell_elapsed;
        assert!(
            restarted <= 0.1 + 1e-9,
            "dwell restarts after the fade: {restarted}"
        );

        tick(&mut store, 0.05);
        let runtime = store.row_runtime(0).unwrap();
        assert!((runtime.dwell_elapsed - (restarted + 0.05)).abs() < 1e-9);
    }

    #[test]
    fn row_crossfade_follows_the_smoothstep_and_removes_the_ghost() {
        let mut store = store_with_scenes();
        store
            .dispatch("scenes/startPerformance", json!({}))
            .expect("start");
        store.performance.rows.get_mut(&0).unwrap().dwell_target = 1.0;
        advance(&mut store, 1.05);

        let transition = store.row_transition(0).cloned().expect("transition");
        let ghost_id = transition.ghost_group_id.clone();
        let incoming_id = transition.group_id.clone();
        let ghost_modules = store
            .state()
            .find_group(&ghost_id)
            .expect("ghost group exists")
            .modules
            .clone();
        assert!(!ghost_modules.is_empty(), "ghost owns the old modules");
        assert!(
            store
                .state()
                .find_group(&ghost_id)
                .unwrap()
                .transition_ghost
                == Some(true)
        );
        for module_id in &ghost_modules {
            assert!(
                store.state().modules.contains_key(module_id),
                "ghost module {module_id} stays active during the fade"
            );
        }

        // Park the fade at a known progress before sampling the alphas.
        store
            .performance
            .row_transitions
            .get_mut(&0)
            .unwrap()
            .elapsed = 0.0;
        tick(&mut store, 0.5);
        let incoming_alpha = store.state().find_group(&incoming_id).unwrap().alpha;
        let ghost_alpha = store.state().find_group(&ghost_id).unwrap().alpha;
        // smoothstep(0.5) = 0.5.
        assert!((incoming_alpha - 0.5).abs() < 1e-6, "{incoming_alpha}");
        assert!((ghost_alpha - 0.5).abs() < 1e-6, "{ghost_alpha}");

        tick(&mut store, 0.55);
        assert!(store.row_transition(0).is_none(), "transition finalised");
        assert!(
            store.state().find_group(&ghost_id).is_none(),
            "the ghost is removed"
        );
        for module_id in &ghost_modules {
            assert!(
                !store.state().modules.contains_key(module_id),
                "ghost module {module_id} released"
            );
        }
        assert_eq!(store.state().find_group(&incoming_id).unwrap().alpha, 1.0);
    }

    #[test]
    fn stopping_mid_transition_finalises_without_ghosts_or_transitions() {
        let mut store = store_with_scenes();
        store
            .dispatch("scenes/startPerformance", json!({}))
            .expect("start");
        store.performance.rows.get_mut(&0).unwrap().dwell_target = 1.0;
        advance(&mut store, 1.05);
        let transition = store.row_transition(0).cloned().expect("transition");
        tick(&mut store, 0.3);

        store
            .dispatch("scenes/stopPerformance", json!({}))
            .expect("stop");
        assert!(!store.performance_runtime().enabled);
        assert!(store.performance_runtime().row_transitions.is_empty());
        assert!(store.performance_runtime().rows.is_empty());
        assert!(
            store
                .state()
                .find_group(&transition.ghost_group_id)
                .is_none(),
            "no orphaned ghost Group"
        );
        assert!(
            !store
                .state()
                .groups
                .iter()
                .any(|group| group.transition_ghost == Some(true))
        );
        assert_eq!(
            store
                .state()
                .find_group(&transition.group_id)
                .unwrap()
                .alpha,
            1.0
        );
        let types = delta_types(&mut store);
        assert!(
            types
                .iter()
                .any(|type_| type_ == "scenes/UPDATE_PERFORMANCE")
        );
    }

    #[test]
    fn stopping_mid_whole_scene_shader_transition_drops_the_capture_and_ghosts() {
        let mut store = store_with_scenes();
        store
            .dispatch(
                "scenes/updatePerformance",
                json!({
                    "config": {
                        "allowedTransitions": ["Three Crossfade"],
                        "transitionSeconds": 1,
                    }
                }),
            )
            .expect("config");
        store
            .dispatch("scenes/applyScene", json!({ "sceneId": "scene-b" }))
            .expect("apply b");
        store
            .dispatch("scenes/applyScene", json!({ "sceneId": "scene-c" }))
            .expect("apply c");

        let transition = store
            .whole_scene_transition()
            .cloned()
            .expect("a shader transition is running");
        assert_eq!(transition.kind, "Three Crossfade");
        assert!(
            store.take_pending_transition_capture().is_some(),
            "the shader transition asked for the outgoing frame"
        );
        tick(&mut store, 0.4);

        store
            .dispatch("scenes/stopPerformance", json!({}))
            .expect("stop");

        assert!(!store.performance_runtime().enabled);
        assert!(
            store.whole_scene_transition().is_none(),
            "the in-flight whole-scene transition is finished, not left behind"
        );
        assert!(
            store.take_pending_transition_capture().is_none(),
            "no capture request survives the stop"
        );
        for (group_id, _) in &transition.from {
            assert!(
                store.state().find_group(group_id).is_none(),
                "the outgoing Scene is removed"
            );
        }
        for (group_id, alpha) in &transition.to {
            assert_eq!(
                store.state().find_group(group_id).unwrap().alpha,
                *alpha,
                "the incoming Scene lands at its target alpha"
            );
        }
        assert!(
            !store
                .state()
                .groups
                .iter()
                .any(|group| group.transition_ghost == Some(true)),
            "no ghost Group survives the stop"
        );
    }

    /// A preset document with one plain Group and no Modules: restoring it
    /// removes every Scene Group the performance runtime was pointing at.
    fn restored_preset() -> Value {
        json!({
            "groups": [{
                "id": "restored-group",
                "name": "Restored",
                "hidden": false,
                "modules": [],
                "enabled": 1,
                "alpha": 1,
                "inherit": false,
                "inheritFrom": -1,
                "pipeline": false,
                "clearing": false,
                "compositeOperation": "normal",
            }],
            "modules": {},
        })
    }

    /// Every Group/Module id in the live state.
    fn live_ids(store: &Store) -> Vec<String> {
        store
            .state()
            .groups
            .iter()
            .flat_map(|group| {
                std::iter::once(group.id.clone()).chain(group.modules.iter().cloned())
            })
            .collect()
    }

    /// No post-restore delta may mention an object the restore deleted.
    fn assert_no_stale_ids(deltas: &[Mutation], stale: &[String]) {
        for mutation in deltas {
            let payload = mutation.payload.to_string();
            for id in stale {
                assert!(
                    !payload.contains(id.as_str()),
                    "stale id {id} in {}: {payload}",
                    mutation.r#type
                );
            }
        }
    }

    #[test]
    fn restore_clears_row_transitions_and_republishes_active_rows() {
        let mut store = store_with_scenes();
        // A Scene is applied (so `applied_scene_id` is stamped), performance
        // mode starts, and a row crossfade is left in flight: three pieces of
        // runtime bookkeeping that reference live Group ids.
        store
            .dispatch("scenes/applyScene", json!({ "sceneId": "scene-a" }))
            .expect("apply a");
        store
            .dispatch("scenes/startPerformance", json!({}))
            .expect("start");
        store.performance.rows.get_mut(&0).unwrap().dwell_target = 0.0;
        advance(&mut store, 0.05);
        assert!(
            store.row_transition(0).is_some(),
            "a row crossfade is in flight"
        );
        assert!(!store.performance_runtime().rows.is_empty());
        assert!(store.applied_scene_id.is_some());

        let stale = live_ids(&store);
        assert!(!stale.is_empty());
        store.drain_deltas();

        store.restore(&restored_preset()).expect("restore loads");

        // Nothing that referenced the deleted Groups survives (R-29).
        assert!(store.performance_runtime().rows.is_empty());
        assert!(store.performance_runtime().row_transitions.is_empty());
        assert!(store.whole_scene_transition().is_none());
        assert!(store.applied_scene_id.is_none());
        assert!(store.pending_transition_capture.is_none());

        let deltas = store.drain_deltas();
        let types: Vec<&str> = deltas.iter().map(|m| m.r#type.as_str()).collect();
        assert!(
            types.contains(&"scenes/CLEAR_ALL_ROW_PROGRESS"),
            "the mirror's row-progress meters are cleared: {types:?}"
        );
        let active_rows = deltas
            .iter()
            .find(|m| m.r#type == "scenes/SET_ACTIVE_ROWS")
            .expect("the active rows are re-emitted from the restored state");
        assert_eq!(
            active_rows.payload,
            json!({}),
            "the restored preset has no Scene-stamped Groups, so no row is active"
        );
        assert_no_stale_ids(&deltas, &stale);
    }

    #[test]
    fn restore_clears_a_whole_scene_transition_and_its_capture_request() {
        let mut store = store_with_scenes();
        store
            .dispatch(
                "scenes/updatePerformance",
                json!({
                    "config": {
                        "allowedTransitions": ["Three Crossfade"],
                        "transitionSeconds": 1,
                    }
                }),
            )
            .expect("config");
        store
            .dispatch("scenes/applyScene", json!({ "sceneId": "scene-a" }))
            .expect("apply a");
        store
            .dispatch("scenes/applyScene", json!({ "sceneId": "scene-b" }))
            .expect("apply b");

        let transition = store
            .whole_scene_transition()
            .cloned()
            .expect("a shader transition is running");
        assert!(
            store.pending_transition_capture.is_some(),
            "the shader transition asked for the outgoing frame"
        );
        assert_eq!(store.applied_scene_id.as_deref(), Some("scene-b"));

        let stale = live_ids(&store);
        store.drain_deltas();

        store.restore(&restored_preset()).expect("restore loads");

        assert!(
            store.whole_scene_transition().is_none(),
            "the in-flight whole-scene transition is dropped, not left on dead ids"
        );
        assert!(store.performance_runtime().row_transitions.is_empty());
        assert!(store.performance_runtime().rows.is_empty());
        assert!(
            store.pending_transition_capture.is_none(),
            "no capture request survives the restore"
        );
        assert!(store.applied_scene_id.is_none());

        let deltas = store.drain_deltas();
        assert!(
            deltas
                .iter()
                .any(|m| m.r#type == "scenes/SET_TRANSITIONING" && m.payload == json!(false)),
            "the mirror's transitioning flag is cleared"
        );
        assert!(
            deltas.iter().any(|m| m.r#type == "scenes/SET_ACTIVE_ROWS"),
            "the active rows are re-emitted"
        );
        assert_no_stale_ids(&deltas, &stale);
        for (group_id, _) in &transition.from {
            assert!(
                store.state().find_group(group_id).is_none(),
                "the outgoing Group is gone with the restored state"
            );
        }
    }

    #[test]
    fn whole_scene_fade_draws_the_incoming_scene_above_the_outgoing_one() {
        let mut store = store_with_scenes();
        store
            .dispatch(
                "scenes/updatePerformance",
                json!({ "config": { "allowedTransitions": ["Fade"], "transitionSeconds": 1 } }),
            )
            .expect("config");
        store
            .dispatch("scenes/applyScene", json!({ "sceneId": "scene-a" }))
            .expect("apply a");
        store
            .dispatch("scenes/applyScene", json!({ "sceneId": "scene-b" }))
            .expect("apply b");

        let transition = store
            .whole_scene_transition()
            .cloned()
            .expect("a Fade is running");
        assert_eq!(transition.kind, FADE_TRANSITION);
        assert_eq!(transition.from.len(), transition.to.len());

        let order = store.state().group_index_render_order();
        let position = |group_id: &str| {
            order
                .iter()
                .position(|&index| store.state().groups[index].id == group_id)
                .unwrap_or_else(|| panic!("{group_id} is in the layer stack"))
        };

        // Later entries draw on top, so the incoming Group of each row must come
        // after that row's outgoing Group (R-29: the outgoing Scene used to be
        // on top because the created Groups were inserted first).
        for ((outgoing, _), (incoming, _)) in transition.from.iter().zip(&transition.to) {
            assert!(
                position(incoming) > position(outgoing),
                "the incoming Group {incoming} draws above the outgoing one {outgoing}"
            );
            assert_eq!(
                store.state().find_group(outgoing).and_then(|g| g.scene_row),
                store.state().find_group(incoming).and_then(|g| g.scene_row),
                "the pair belongs to the same row"
            );
        }

        // Row lookups during the fade resolve to the incoming Groups: the
        // outgoing ones are only still in the layer stack for the crossfade.
        for (row, (incoming, _)) in transition.to.iter().enumerate() {
            let live = store.find_live_row_group(row).expect("a live row Group");
            assert_eq!(live.id, *incoming);
        }

        // `emit_group_order` always keeps the render order row-sorted, so rows
        // ascend and each row's incoming Group follows that row's outgoing one
        // (the `orderRowGroups` shape in `scenes.js:1300-1316`; the created
        // Group's `inheritFrom` is remapped to the new ids, so the rows stay
        // connected across the change).
        let rows: Vec<usize> = order
            .iter()
            .filter_map(|&index| store.state().groups[index].scene_row)
            .collect();
        assert!(
            rows.windows(2).all(|pair| pair[0] <= pair[1]),
            "render order is row-sorted: {rows:?}"
        );
    }

    #[test]
    fn row_crossfade_draws_the_incoming_group_above_its_ghost() {
        let mut store = store_with_scenes();
        store
            .dispatch("scenes/startPerformance", json!({}))
            .expect("start");
        store.performance.rows.get_mut(&0).unwrap().dwell_target = 0.0;
        advance(&mut store, 0.05);

        let transition = store.row_transition(0).cloned().expect("row crossfade");
        let order = store.state().group_index_render_order();
        let position = |group_id: &str| {
            order
                .iter()
                .position(|&index| store.state().groups[index].id == group_id)
                .unwrap_or_else(|| panic!("{group_id} is in the layer stack"))
        };

        assert!(
            position(&transition.group_id) > position(&transition.ghost_group_id),
            "the incoming Group draws above the outgoing ghost (`orderRowGroups`, scenes.js:1300)"
        );
    }

    #[test]
    fn performance_row_changes_always_fade_regardless_of_allowed_transitions() {
        let mut store = store_with_scenes();
        // The config already allows only "None".
        store
            .dispatch("scenes/startPerformance", json!({}))
            .expect("start");
        store.performance.rows.get_mut(&0).unwrap().dwell_target = 1.0;
        advance(&mut store, 1.05);
        assert!(
            store.row_transition(0).is_some(),
            "performance changes ignore the allowed list and crossfade"
        );
    }

    #[test]
    fn manual_change_during_performance_uses_a_fade_transition() {
        let mut store = store_with_scenes();
        store
            .dispatch("scenes/startPerformance", json!({}))
            .expect("start");
        let before: Vec<String> = store
            .state()
            .groups
            .iter()
            .filter(|group| group.name != GALLERY_GROUP_NAME)
            .map(|group| group.id.clone())
            .collect();
        assert!(store.whole_scene_transition().is_none());

        store
            .dispatch("scenes/applyScene", json!({ "sceneId": "scene-c" }))
            .expect("manual apply");
        let transition = store
            .whole_scene_transition()
            .cloned()
            .expect("manual change crossfades");
        assert_eq!(transition.kind, FADE_TRANSITION);
        assert_eq!(transition.from.len(), before.len());
        assert!(transition.to.len() >= 2);
        for (group_id, _) in &transition.to {
            assert_eq!(store.state().find_group(group_id).unwrap().alpha, 0.0);
        }
        for (group_id, alpha) in &transition.from {
            assert_eq!(store.state().find_group(group_id).unwrap().alpha, *alpha);
        }

        tick(&mut store, transition.duration + 0.01);
        assert!(
            store.whole_scene_transition().is_none(),
            "the fade completes after transitionSeconds"
        );
        for (group_id, _) in &transition.from {
            assert!(store.state().find_group(group_id).is_none());
        }
        for (group_id, alpha) in &transition.to {
            assert_eq!(store.state().find_group(group_id).unwrap().alpha, *alpha);
        }
    }

    #[test]
    fn manual_change_with_a_shader_transition_records_the_effect_and_capture() {
        let mut store = store_with_scenes();
        store
            .dispatch(
                "scenes/updatePerformance",
                json!({
                    "config": {
                        "allowedTransitions": ["Three Crossfade"],
                        "transitionSeconds": 1,
                    }
                }),
            )
            .expect("config");
        store
            .dispatch("scenes/applyScene", json!({ "sceneId": "scene-b" }))
            .expect("apply b");
        store.drain_deltas();
        store
            .dispatch("scenes/applyScene", json!({ "sceneId": "scene-c" }))
            .expect("apply c");

        let transition = store
            .whole_scene_transition()
            .cloned()
            .expect("a transition is recorded");
        assert_eq!(transition.kind, "Three Crossfade");
        assert_eq!(
            transition.shader_effect(),
            Some(crate::scene_transition::TransitionEffect::Crossfade)
        );
        assert_eq!(
            store.take_pending_transition_capture(),
            Some(crate::scene_transition::TransitionEffect::Crossfade),
            "the window loop is asked to capture the outgoing frame"
        );
        assert!(
            store.take_pending_transition_capture().is_none(),
            "the capture request is one-shot"
        );

        // Both Scenes render at final opacity: the shader does the blending.
        for (group_id, _) in &transition.to {
            assert_eq!(store.state().find_group(group_id).unwrap().alpha, 1.0);
        }
        for (group_id, alpha) in &transition.from {
            assert_eq!(store.state().find_group(group_id).unwrap().alpha, *alpha);
        }

        // Half-way: the timeline advanced, the alphas did not.
        tick(&mut store, 0.5);
        let mid = store
            .whole_scene_transition()
            .cloned()
            .expect("still running");
        assert!((mid.progress() - 0.5).abs() < 0.02, "{}", mid.progress());
        for (group_id, _) in &transition.to {
            assert_eq!(store.state().find_group(group_id).unwrap().alpha, 1.0);
        }

        tick(&mut store, 0.6);
        assert!(store.whole_scene_transition().is_none(), "completes at 1.0");
        for (group_id, _) in &transition.from {
            assert!(
                store.state().find_group(group_id).is_none(),
                "the outgoing Scene is removed at completion"
            );
        }
        for (group_id, alpha) in &transition.to {
            assert_eq!(store.state().find_group(group_id).unwrap().alpha, *alpha);
        }
    }

    #[test]
    fn unavailable_or_empty_allowed_lists_still_fade() {
        for allowed in [json!([]), json!(["E2E Missing Transition"])] {
            let mut store = store_with_scenes();
            store
                .dispatch(
                    "scenes/updatePerformance",
                    json!({ "config": { "allowedTransitions": allowed, "transitionSeconds": 1 } }),
                )
                .expect("config");
            store
                .dispatch("scenes/applyScene", json!({ "sceneId": "scene-b" }))
                .expect("apply b");
            store
                .dispatch("scenes/applyScene", json!({ "sceneId": "scene-c" }))
                .expect("apply c");
            let transition = store
                .whole_scene_transition()
                .cloned()
                .expect("a transition is recorded");
            assert_eq!(transition.kind, FADE_TRANSITION, "allowed: {allowed}");
            assert_eq!(transition.shader_effect(), None);
            assert_eq!(
                store.take_pending_transition_capture(),
                None,
                "Fade needs no capture"
            );
        }
    }

    #[test]
    fn shader_transition_falls_back_to_fade_without_losing_the_change() {
        let mut store = store_with_scenes();
        store
            .dispatch(
                "scenes/updatePerformance",
                json!({
                    "config": {
                        "allowedTransitions": ["Three Wipe"],
                        "transitionSeconds": 1,
                    }
                }),
            )
            .expect("config");
        store
            .dispatch("scenes/applyScene", json!({ "sceneId": "scene-b" }))
            .expect("apply b");
        store
            .dispatch("scenes/applyScene", json!({ "sceneId": "scene-c" }))
            .expect("apply c");
        let shader = store
            .whole_scene_transition()
            .cloned()
            .expect("shader transition");
        assert_eq!(shader.kind, "Three Wipe");

        assert!(store.fall_back_scene_transition_to_fade());
        let fade = store
            .whole_scene_transition()
            .cloned()
            .expect("transition survives the fallback");
        assert_eq!(fade.kind, FADE_TRANSITION);
        assert_eq!(fade.duration, shader.duration);
        for (group_id, _) in &fade.to {
            assert_eq!(store.state().find_group(group_id).unwrap().alpha, 0.0);
        }
        for (group_id, alpha) in &fade.from {
            assert_eq!(store.state().find_group(group_id).unwrap().alpha, *alpha);
        }
        assert!(
            !store.fall_back_scene_transition_to_fade(),
            "a Fade transition is not downgraded again"
        );

        // Half-way the Fade path reaches smoothstep(0.5) = 0.5.
        tick(&mut store, 0.5);
        for (group_id, _) in &fade.to {
            let alpha = store.state().find_group(group_id).unwrap().alpha;
            assert!((alpha - 0.5).abs() < 0.01, "{alpha}");
        }

        tick(&mut store, 0.6);
        assert!(store.whole_scene_transition().is_none());
        for (group_id, _) in &fade.from {
            assert!(store.state().find_group(group_id).is_none());
        }
    }

    #[test]
    fn manual_change_with_performance_stopped_respects_none() {
        let mut store = store_with_scenes();
        store
            .dispatch(
                "scenes/updatePerformance",
                json!({ "config": { "allowedTransitions": ["None"] } }),
            )
            .expect("config");
        store
            .dispatch("scenes/applyScene", json!({ "sceneId": "scene-b" }))
            .expect("apply b");
        store.drain_deltas();
        store
            .dispatch("scenes/applyScene", json!({ "sceneId": "scene-c" }))
            .expect("apply c");
        assert!(
            store.whole_scene_transition().is_none(),
            "None switches instantly"
        );
        assert!(
            !store
                .state()
                .groups
                .iter()
                .any(|group| group.scene_id.as_deref() == Some("scene-b")),
            "the outgoing Scene's Groups are gone immediately"
        );
    }

    #[test]
    fn tick_deltas_are_event_driven_not_per_frame() {
        let mut store = store_with_scenes();
        store
            .dispatch(
                "scenes/updatePerformance",
                json!({ "config": { "minSeconds": 8, "maxSeconds": 12 } }),
            )
            .expect("slow the dwell down");
        store
            .dispatch("scenes/startPerformance", json!({}))
            .expect("start");
        store.performance.rng = Rng::seeded(11);
        store.drain_deltas();

        let mut quiet_ticks = 0usize;
        let mut progress_deltas = 0usize;
        let mut changes = 0usize;
        let mut max_per_tick = 0usize;
        for _ in 0..(60 * 60) {
            tick(&mut store, 1.0 / 60.0);
            let types = delta_types(&mut store);
            if types.is_empty() {
                quiet_ticks += 1;
            }
            max_per_tick = max_per_tick.max(types.len());
            progress_deltas += types
                .iter()
                .filter(|type_| type_.as_str() == "scenes/SET_ROW_PROGRESS")
                .count();
            changes += types
                .iter()
                .filter(|type_| type_.as_str() == "scenes/SET_ACTIVE_ROW")
                .count();
        }

        assert!(
            quiet_ticks > 3000,
            "the tick must stay quiet between events (quiet={quiet_ticks})"
        );
        assert!(
            progress_deltas <= 60 * 30,
            "row progress is throttled to ~10 Hz (got {progress_deltas} in a minute)"
        );
        assert!(
            max_per_tick < 120,
            "no single tick floods the delta stream: {max_per_tick}"
        );
        assert!(changes > 0, "a simulated minute changes some rows");
    }
}
