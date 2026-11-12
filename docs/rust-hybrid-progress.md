# Rust hybrid engine — progress tracker

Tracks implementation of [rust-hybrid.md](./rust-hybrid.md) on the `rust-hybrid`
branch. Updated as milestones land; each box is a commit or a small group of
commits. Test-first per AGENTS.md; Rust tests are authoritative for ported
logic, the Vitest suite stays until parity is signed off.

Tooling: the engine builds only inside Docker (`make engine` / `engine-test` /
`engine-lint` / `engine-run`); `crates/` is a workspace. The engine binary is
`crates/target/release/modv-engine`. `make disk-clean` prunes BuildKit cache;
`make build`/`appimage` refuse to start below 20 GB free.

## Verification snapshot

- Rust workspace: **911 tests pass** (modv-app 106, modv-engine 365, modv-ipc
  32, modv-media 131, modv-render 21, modv-renderers 256).
- Vitest: **924 tests pass** (includes the engine mirror-parity replay).
- `make lint`: clean (one pre-existing warning in an untouched component).
- e2e: **143/143 pass** against the packaged AppImage, including the
  engine-mode spec (preview frames publish, engine kill survived and
  restarted, renderer creates no WebGL/WebGPU context).
- Packaged-engine smoke (CDP against the AppImage, `MODV_ENGINE=1`): the UI
  mirror receives all **59 seeded Scenes** via `state/sync`; applying the
  default Scene "Lone Chrome" builds the Foreground group + Point10 Chromatic
  Line module and the composited preview renders (8 % lit, full-range pixels);
  performance mode starts 3 rows, runs crossfades and stops without ghosts.
- Abrupt Electron death (SIGKILL/container teardown) leaves **no orphaned
  engine** (`PR_SET_PDEATHSIG`).

## Phase 0 — Tooling

- [x] Rust stage + engine-build stage in `Dockerfile`; `.dockerignore` /
      `.gitignore` exclude `crates/target` and core dumps at any depth
- [x] Makefile targets `docker-rust`, `engine`, `engine-test`, `engine-lint`,
      `engine-run`, `disk-guard`, `disk-clean`; named cargo registry volume
- [x] `crates/` workspace skeleton (modv-app, modv-engine, modv-ipc,
      modv-media, modv-render, modv-renderers)
- [x] electron-builder `extraResources: crates/target/release/modv-engine →
      modv-engine`
- [x] lavapipe (Mesa Vulkan) + libxkbcommon-x11 in the toolchain/rust stages

## M0 — Spike — complete

See [rust-hybrid-spike.md](./rust-hybrid-spike.md): native Wayland Vulkan at
vsync (120 fps, p99 ≈ vsync period, 3.4 % CPU, 103 MB RSS), headless 6137 fps
at 720p, ChromaticLine ported with unit tests, go/no-go = **GO**.

## M1 — Engine skeleton — complete

Control protocol, VMP1 preview ring, engine supervisor (spawn/connect retry,
heartbeat watchdog, restart budget, snapshot restore), shutdown wiring,
`window.engine` preload API, renderer engine client, `MODV_ENGINE=1` gating,
engine e2e spec. Fixed along the way: X11 `libxkbcommon-x11` dependency,
`fs.rmSync` behaviour in this Electron build, bench-seconds suicide in
supervisor mode, lost `engine/state` events, engine-mode renderer TypeErrors.

## M2 — Shader / ISF — complete

- [x] modv-engine state model (groups/modules/inputs/props/blend/SWAP), delta
      stream, preset round-trip + Scene/playlist detection (96 tests)
- [x] modv-render FrameGraph, LRU texture pool, canvas-semantics blend WGSL
      for all 28 modes (21 tests)
- [x] Shader renderer: uniform contract, wrapper shim, all 37 modules in WGSL,
      naga-validated (24 tests)
- [x] ISF: parser/upgrader, ES→desktop-450 preprocessor with the full macro
      set, naga GLSL→WGSL translator, overrides hook, multi-pass runtime with
      PERSISTENT/FLOAT buffers; corpus report — **344/351 fragments translate
      without an override (98.0 %)**, 35/36 vertex sidecars (7 overrides
      pending, documented in `docs/isf-corpus-report.md`)
- [x] Engine integration: EngineState store, GPU compositor, real previews,
      mutations + delta stream (44+ tests)
- [x] Mirror parity: `--emit-fixtures` + `tests/unit/engine-mirror.test.js`
      replay 19 mutations into the real Vuex mirror with deep-equal state

## M3 — 2D modules — complete

- [x] wgpu canvas-2D facade (paths/arcs, 4× MSAA fill/stroke, transforms,
      clip, drawImage incl. self-sampling, cosmic-text/swash text) — 54 tests
- [x] 15 hand-written 2D modules ported with JS meta/props; per-module canvases
      in the compositor; Ball renders through the packaged preview (CDP)
- [x] Palette tween colours advance on the frame clock

## M4 — Point10 — partial (5 of 25 scenes)

- [x] Scene trait/SceneContext, AudioBridge + P10Audio, shared particle
      alpha/sprite + random, postfx chain (UnrealBloom/OutputPass/ShaderPass/
      Composer), compositor integration, interim band mapping
- [x] ChromaticLine (audio-driven), ChromaticWave, ParticleField,
      SpiralParticles, Waveform — 96 Point10 tests incl. lavapipe renders
- [ ] Remaining 20 scenes (tunnels, cubes, water/ripples, GlassLens, UltraHDR,
      WebGPU/TSL scenes, GeometricTunnel SSAO, RollingFire/SmokeRoom,
      LatentBackrooms) — next batches in the plan's dependency order
- [ ] Golden-image parity vs the legacy renderer, warm cache/instance pool

## M5 — Scenes / performance — mostly complete

- [x] scenes core logic (timing/rows/transition-pick/capture/preview-data/
      columns/cells/link-defaults) — 269 tests
- [x] `default-scenes.js` → `data/default-scenes.json` (59 Scenes,
      DEFAULTS_VERSION 12) + seeding/refresh/removal logic
- [x] Scene apply/save/capture, playlists, scenes.json persistence, row-name
      remap, scene-list deltas
- [x] Performance runtime: start/stop/tick, per-row dwell, crossfade ghosts,
      row-fade blending assigned to both group ids, Fade for manual changes,
      event-driven deltas
- [ ] Scene-preview frames for the UI hover previews (≥ 25 fps)
- [ ] ISF shader scene transitions (Fade is the runtime transition for now)
- [ ] Rename/removal migration for default Scenes on version bumps

## M6 — Media — partial

- [x] AudioBands, tempo/phrase detectors, feature smoothing (37 tests)
- [x] Meyda feature extractors, BeatDetektor, MIDI clock → BPM (43 tests)
- [x] OSC server, raw MIDI ingestion, NDI send/receive via runtime FFI
      (51 tests; NDI needs the SDK at runtime)
- [ ] Camera/screen/video capture (GStreamer) + system audio (PipeWire/cpal)
- [ ] Renderer → engine audio feed (engine analysis of the renderer's pump)
- [ ] Engine-side input-link evaluation of audio/MIDI/OSC sources

## M7 — Parity and release — pending

- [ ] Full e2e green **against the engine default** (currently opt-in); keep
      `MODV_LEGACY_RENDERER` until parity
- [ ] Performance harness results vs baseline; release packaging verification
      on a clean machine
- [ ] Remaining ISF overrides; goldens for all renderers/scenes
- [ ] User JS modules/plugins known-gap documentation and the Rust plugin ABI
