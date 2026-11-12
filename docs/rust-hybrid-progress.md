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

- [x] modv-render: wgpu device/queue, Fifo surface/present, headless target,
      frame stats (p50/p90/p99, CPU %, RSS)
- [x] modv-app: CLI (`--width/--height/--fullscreen/--frames/--headless/
      --bench/--bench-seconds/--screenshot/--audio-fixture/--print-adapter`),
      winit native window
- [x] modv-renderers: ChromaticLine port (ring buffer, texel windows, uniforms,
      WGSL translation) + unit tests
- [x] Headless screenshot of the scene (tmp/spike-chromatic-line.png — visual
      check vs reference)
- [x] Windowed run on Wayland: presents at 120 fps Fifo, p99 ≈ vsync period,
      RSS ~103 MB
- [x] Frame-time/CPU/RSS recorded; Electron + XWayland baseline measured the
      same way (see docs/rust-hybrid-spike.md)
- [x] `docs/rust-hybrid-spike.md` — measurements + go/no-go recommendation

## M1 — Engine skeleton

- [x] modv-ipc: framing (u32 LE len + u8 kind), request/response/event JSON,
      Unix socket server
- [x] modv-ipc: VMP1 preview ring (/dev/shm, 3 slots, seqlock read-back)
- [x] modv-app: `--socket` / `--state` wiring — socket poll in the frame loop,
      heartbeat, snapshot save/restore, `engine/shutdown` handling,
      `preview/ready` + `preview/frame` events (flat-colour placeholder until
      WS-2/M2)
- [x] Electron: `engine-supervisor.js`, `engine-protocol.js`,
      `engine-log-forwarder.js`, `engine-ipc.js` in `src/main/`; shutdown
      cleanup task; restart budget (5/60 s); engine-mode e2e spec
- [x] Preload: `window.engine` API; renderer IPC client replacing the worker
      behind `MODV_ENGINE=1`
- [x] e2e green on `dist/` with the engine binary embedded (preview frames,
      kill survives, restart) — `tests/e2e/spec/engine/engineSkeleton.spec.js`
      passes (GPU-context-free renderer asserted); native Wayland window
      presentation verified

## M2 — Shader / ISF

- [x] modv-engine: state model (groups, modules, inputs, links, props coercion,
      blend modes, render order, SWAP), delta stream; preset round-trip
      (deep-equal on the six-key format, scene/playlist detection); 96 tests
- [x] modv-render: FrameGraph (Group/Composite/RowFade/SceneTransition/Present
      node shapes), LRU texture pool, blend WGSL for all 28 modes; headless
      lavapipe pixel tests (21)
- [x] modv-renderers/shader: uniform contract, wrapper shim, 37 modules → WGSL,
      naga-validated (24 tests)
- [x] modv-renderers/isf: parser, macro expander, ES1→3 upgrade, prop
      synthesis, multi-pass runtime, naga translator, overrides, corpus report
      — 344/351 fragments translate without an override (98.0%)
- [x] engine integration: EngineState → render graph → present → preview ring;
      real store mutations + delta stream; verified end-to-end in the packaged
      app via CDP (engine-created group + Point10 Invert → 100% white preview)
- [ ] 2D modules render through the engine compositor (facade exists, wiring)
- [ ] three/Cube renderer
- [x] mirror parity: Rust delta stream replayed into the JS mirror store
      yields a deep-equal preset (tests/unit/engine-mirror.test.js +
      modv-engine --emit-fixtures; 19 scripted mutations)

## M3 — 2D modules

- [x] wgpu 2D facade (path builder, fill/stroke, clip, transforms,
      drawImage, text via cosmic-text) + tests for the 22 ops
- [x] 15 hand-written 2D modules ported, smoke renders on lavapipe
- [x] 2D modules wired into the engine compositor (per-module canvases,
      clearing semantics, module registry in modv-app) — verified in the
      packaged app via CDP (Ball field renders through the preview ring)
- [ ] three Cube port (native WGSL)

## M4 — Point10

- [ ] shared/ + plane9 ports (P9Glsl, P9Audio), post-fx chain
      (RenderPass/UnrealBloom/ShaderPass/OutputPass/Afterimage/SSAO)
- [x] Point10 scene trait, audio bridge, shared particle helpers, postfx chain
      (UnrealBloom/OutputPass/Composer), compositor integration
- [x] Chromatic Line, Chromatic Wave, Particle Field, Spiral Particles,
      Waveform ported and rendering in the engine
- [ ] Remaining 20 scenes (next batches: tunnels, cubes, water, WebGPU/TSL
      scenes, SSAO/Afterimage); golden images vs the legacy renderer; warm
      cache/instance eviction

## M5 — Scenes / performance

- [x] scenes core logic (per-cell dwell, row names/family sharing, transition
      pick, capture, preview-data, columns/cells, link defaults) — 269 tests
- [x] default-scenes.js → JSON data (59 Scenes) + seeding/refresh/removal logic
- [x] Scene apply/save/capture, playlist import/export, scenes.json
      persistence, row-name remap — engine-integrated, mirror deltas emitted
- [x] performance runtime (start/stop/tick, per-row dwell advance, crossfade
      ghosts, row-fade blending assigned to both ids, whole-scene Fade for
      manual changes during performance, event-driven deltas)
- [ ] scene-preview frames to the UI (hover previews ≥ 25 fps)
- [ ] ISF shader scene transitions (Fade is the runtime transition for now)

## M6 — Media

- [x] AudioBands port (AGC, cells/lanes, beat envelope) — fixture-parity test
- [x] tempo-detector, phrase-detector, audio-features smoothing
- [x] BeatDetektor (onset/draft scoring, kick/BPM)
- [x] Meyda feature extractors (rms, zcr, energy, spectrum stats, Bark bands)
- [x] MIDI clock → BPM (95-sample MA, 500 ms emit, outlier reset)
- [ ] OSC server (rosc), NDI sender/receiver (libndi FFI)
- [ ] GStreamer cameras/screen/video + PipeWire system audio
- [ ] renderer → engine audio feed (engine analysis of the renderer's pump)

## M7 — Parity and release

- [ ] Full e2e suite green against the engine; `MODV_LEGACY_RENDERER` removed
- [ ] Performance harness results recorded vs baseline
- [ ] Docs, known-gaps section (user JS modules/plugins), packaging verified on
      a clean machine
