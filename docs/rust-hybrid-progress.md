# Rust hybrid engine — progress tracker

Tracks implementation of [rust-hybrid.md](./rust-hybrid.md) on the `rust-hybrid`
branch. Updated as milestones land; each box is a commit or a small group of
commits. Test-first per AGENTS.md; Rust tests are authoritative for ported
logic, the Vitest suite stays until parity is signed off.

Tooling: the engine builds only inside Docker (`make engine` / `engine-test` /
`engine-lint` / `engine-run`); `crates/` is a workspace. The engine binary is
`crates/target/release/modv-engine`.

## Phase 0 — Tooling

- [x] Rust stage + engine-build stage in `Dockerfile`; `.dockerignore` /
      `.gitignore` exclude `crates/target`
- [x] Makefile targets `docker-rust`, `engine`, `engine-test`, `engine-lint`,
      `engine-run`; named cargo registry volume
- [x] `crates/` workspace skeleton (modv-app, modv-engine, modv-ipc,
      modv-media, modv-render, modv-renderers)
- [x] electron-builder `extraResources: crates/target/release/modv-engine →
      modv-engine`
- [x] lavapipe (Mesa Vulkan) in the rust stage for headless GPU tests

## M0 — Spike

- [ ] modv-render: wgpu device/queue, Fifo surface/present, headless target,
      frame stats (p50/p90/p99, CPU %, RSS)
- [ ] modv-app: CLI (`--width/--height/--fullscreen/--frames/--headless/
      --bench/--bench-seconds/--screenshot/--audio-fixture/--print-adapter`),
      winit native window
- [ ] modv-renderers: ChromaticLine port (ring buffer, texel windows, uniforms,
      WGSL translation) + unit tests
- [ ] Headless screenshot of the scene
- [ ] Windowed run on Wayland (and X11 if Wayland unavailable)
- [ ] Frame-time/CPU/RSS recorded for the scene; Electron + XWayland baseline
      measured the same way
- [ ] `docs/rust-hybrid-spike.md` — measurements + go/no-go recommendation

## M1 — Engine skeleton

- [ ] modv-ipc: framing (u32 LE len + u8 kind), request/response/event JSON,
      Unix socket server
- [ ] modv-app: `--socket` / `--state`, supervisor loop, heartbeat events,
      snapshot save/restore, clean `engine/shutdown`
- [ ] Preview channel: memfd ring (3 slots), VMP1 header, sequence numbers
- [ ] Electron: `engine-supervisor.js`, `engine-protocol.js`,
      `engine-log-forwarder.js` in `src/main/`; shutdown cleanup task; restart
      budget (5/60 s); snapshot replay on restart
- [ ] Preload: `window.engine` API; renderer IPC client replacing the worker
      behind `MODV_ENGINE=1`
- [ ] e2e: engine presents clear colour; UI alive after forced engine kill;
      restart restores snapshot

## M2 — Shader / ISF

- [ ] modv-engine: state model (groups, modules, inputs, links, props coercion,
      blend modes, render order, SWAP), delta stream + deep-equal mirror test
- [ ] modv-render: FrameGraph (Group/Composite/RowFade/SceneTransition/Present),
      LRU texture pool, blend.wgsl for non-separable modes
- [ ] modv-renderers/shader: uniform contract, wrapper shim, 37 modules → WGSL
- [ ] modv-renderers/isf: parser, macro expander, ES1→3 upgrade, prop
      synthesis, multi-pass runtime, naga translator, overrides, corpus report
- [ ] Preset round-trip deep-equal test (fixtures)

## M3 — 2D modules

- [ ] wgpu 2D facade (path builder, fill/stroke, clip, transforms,
      drawImage, text via cosmic-text) + unit tests for the 22 ops
- [ ] 15 hand-written 2D modules ported, golden images
- [ ] three Cube port (native WGSL)

## M4 — Point10

- [ ] shared/ + plane9 ports (P9Glsl, P9Audio), post-fx chain
      (RenderPass/UnrealBloom/ShaderPass/OutputPass/Afterimage/SSAO)
- [ ] Point10 scene trait, instance pool, eviction, warm cache
- [ ] 25 scenes ported (dependency order); golden images; prop round-trip

## M5 — Scenes / performance

- [ ] scenes state machine (per-cell dwell, row names, ghosts, crossfades,
      stopPerformance, transition pick), row-transition blend
- [ ] default-scenes.js → JSON data + seeding logic
- [ ] Scene capture, hover preview, playlist import/export, transition defs
- [ ] scene-preview renders ≥ 25 fps

## M6 — Media

- [ ] AudioBands port (AGC, cells/lanes, beat envelope) — fixture-parity test
- [ ] tempo-detector, phrase-detector, audio-features smoothing, BeatDetektor
- [ ] Meyda feature extractors (rms, zcr, energy, spectrum stats, Bark bands)
- [ ] MIDI clock → BPM (95-sample MA, 500 ms emit, outlier reset)
- [ ] OSC server (rosc), NDI sender/receiver (libndi FFI)
- [ ] GStreamer cameras/screen/video + PipeWire system audio

## M7 — Parity and release

- [ ] Full e2e suite green against the engine; `MODV_LEGACY_RENDERER` removed
- [ ] Performance harness results recorded vs baseline
- [ ] Docs, known-gaps section (user JS modules/plugins), packaging verified on
      a clean machine
