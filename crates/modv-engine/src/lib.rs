//! Engine state model, scenes, presets, inputs, timing. See docs/rust-hybrid.md.
//!
//! M2a — the pure-logic core of the state model ported from the worker store
//! modules under `src/renderer/src/application/worker/store/modules/`:
//!
//! - [`engine_state`] — groups/modules/inputs/expressions containers,
//!   render-order helpers and inherit resolution
//!   (`store/modules/groups.js`, `worker/group-inherit.js`, `worker/loop.js`).
//! - [`props`] — the `updateProp` coercion pipeline (`modules.js:481-562`).
//! - [`inputs`] — input links and `resolveInputLinkValue`
//!   (`store/modules/inputs.js`, `worker/input-link-value.js`, `utils/map.js`).
//! - [`blend`] — the 28 composite/blend tokens (`util/composite-operations.js`).
//! - [`swap`] — the preset-load SWAP engine (`store/modules/common/swap.js`).
//! - [`delta`] — the `{type, payload}` mutation delta stream
//!   (`worker/index.worker.js:136-144`, WS-1 “Delta stream”).
//! - [`preset`] — the six-key preset format, Scene/playlist detection and
//!   round-trip (`worker/preset-serializer.js`).
//!
//! M5/WS-8 — Scenes and performance mode, ported from the `scene-*.js`
//! helpers, `store/default-scenes.js` and the playlist actions in
//! `store/modules/scenes.js`:
//!
//! - [`scene_timing`] — performance timing normalisation and per-cell dwell
//!   (`worker/scene-timing.js`).
//! - [`scene_rows`] — row ↔ Group name matching, grid height/labels and row
//!   advance selection (`worker/scene-rows.js`).
//! - [`scene_transition_pick`] — crossfade selection
//!   (`worker/scene-transition-pick.js`).
//! - [`scene_capture`] — capture live Groups back into a Scene
//!   (`worker/scene-capture.js`).
//! - [`scene_preview_data`] — pure hover-preview snapshots
//!   (`worker/scene-preview-data.js`).
//! - [`scene_link_defaults`] — default audio links/tweens for Scenes
//!   (`worker/scene-link-defaults.js` + the range builders in `scenes.js`).
//! - [`scene_columns`] / [`scene_cells`] — grid column naming/reordering and
//!   cell clearing/merging (`worker/scene-columns.js`, `worker/scene-cell.js`).
//! - [`scenes_playlist`] — playlist import/export and the performance config
//!   (`store/modules/scenes.js` playlist actions).
//! - [`default_scenes`] — the versioned default Scene data port plus the
//!   seeding/migration rules (`store/default-scenes.js`).

pub mod blend;
pub mod default_scenes;
pub mod delta;
pub mod engine_state;
pub mod inputs;
pub mod preset;
pub mod props;
pub mod scene_capture;
pub mod scene_cells;
pub mod scene_columns;
pub mod scene_link_defaults;
pub mod scene_preview_data;
pub mod scene_rows;
pub mod scene_timing;
pub mod scene_transition_pick;
pub mod scenes_playlist;
pub mod swap;
