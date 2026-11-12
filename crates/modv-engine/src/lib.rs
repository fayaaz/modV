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

pub mod blend;
pub mod delta;
pub mod engine_state;
pub mod inputs;
pub mod preset;
pub mod props;
pub mod swap;
