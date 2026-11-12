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

- Rust workspace: **1538 tests pass** (modv-app 227, modv-engine 413,
  modv-ipc 32, modv-media 160, modv-render 21, modv-renderers 685).
- Vitest: **947 tests pass** (includes the engine mirror-parity replay, the
  preview-client helpers and the engine audio payload shape).
- Real-music analyser check (opt-in fixture): 12 s of the noiseblock radio
  stream decoded to 48 kHz mono, 1199 analysis frames — bands peak at
  bass 0.85 / mid 0.48 / treble 0.30, beat envelope 1.0, tempo 160 BPM, lane
  spread 0.42 (`MODV_AUDIO_FIXTURE=… cargo test -p modv-media --test
  audio_stream_fixture`).
- Packaged-engine smoke (CDP, `MODV_ENGINE=1`): the Gallery renders all **424**
  registered modules (37 shader + 15 2D + 21 Point10 + 351 ISF) with engine
  thumbnails; Scene hover previews paint the hovered cell; BPM tweens animate
  (Chromantic Line `smoothing` 0.252 → 0.209 over 0.9 s); the status bar shows
  live FPS/size and the detected BPM.
- **End-to-end music reaction with engine-owned capture**: playing the
  noiseblock radio stream to the host sink and launching with
  `MODV_AUDIO_SOURCE=pipewire` logs "audio source pipewire owns capture" and
  ignores the renderer pump; over 2.5 s the engine bands move
  (bass 0.73→0.30→0.73, mid 0.58→0.84) at **160 BPM** and the linked
  Chromatic Line gain tracks it (1.217–1.269).
- Headless benchmark (packaged engine, 1280×720, 600 frames):
  **5885 fps**, p99 0.332 ms.
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
- [x] ISF modules registered in the engine: corpus bundled as `resources/isf`,
      discovered via `--isf-dir`, props/meta synthesised, compositor rendering
      with per-frame uniforms + group-canvas image inputs (351 registered)
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

## M4 — Point10 — complete (25 of 25 scenes)

- [x] Scene trait/SceneContext, AudioBridge + P10Audio, shared particle
      alpha/sprite + random, postfx chain (UnrealBloom/OutputPass/ShaderPass/
      Composer), compositor integration, interim band mapping
- [x] Batch 1: ChromaticLine (audio-driven), ChromaticWave, ParticleField,
      SpiralParticles, Waveform
- [x] Batch 2: SquareTunnel, LightTunnel, Wormhole (100k billboards, 5 px
      sprites), ItIsComingForYou — composer bloom/ACES, Boxy
      `preserveBloomAlpha`, transparent-background parity with a lavapipe
      alpha test
- [x] Batch 3: WaterRipple (WaveSim CPU field with a 64-point Node reference
      match), GlassLens (raymarched glass over an internal Chromatic Wave
      backdrop), Mycelium, AroundPlus — shared asset loader with procedural
      fallbacks, phrase-colour helpers
- [x] Batch 4: UltraHDR (torus-knot morph + analytic mirror BRDF), MirrorCube
      (ping-pong feedback), ColorCube (512 instances), Boxy (panorama env +
      `preserveBloomAlpha`), SoundCube (16-bit trail); shared cube mesh and
      p9 shading helpers
- [x] Batch 5a: HollowCube (15k instances + fbm/Shader2 chain),
      AttractorParticles (TSL compute → fragment MRT sim), DvdRoom (TSL smoke →
      seeded puffs, procedural logo)
- [ ] Batch 5b in flight: GeometricTunnel (SSAO), RollingFire, SmokeRoom,
      LatentBackrooms (Afterimage)
- [ ] Golden images (optional; behaviour parity is the priority), warm
      cache/instance eviction

## M5 — Scenes / performance — complete

- [x] scenes core logic (timing/rows/transition-pick/capture/preview-data/
      columns/cells/link-defaults) — 269 tests
- [x] `default-scenes.js` → `data/default-scenes.json` (59 Scenes,
      DEFAULTS_VERSION 12) + seeding/refresh/removal logic
- [x] Scene apply/save/capture, playlists, scenes.json persistence, row-name
      remap, scene-list deltas
- [x] Performance runtime: start/stop/tick, per-row dwell, crossfade ghosts,
      row-fade blending assigned to both group ids, Fade for manual changes,
      event-driven deltas
- [x] Scene hover previews via `preview/scene`: an isolated compositor renders
      one Scene row (fresh ids, saved props) at the requested size, cached per
      `(sceneId,row,width,height)` and invalidated by `scene_list_revision`
      (~1.1 ms warm at 240×135); Scenes.vue polls at ~30 fps and blits with
      `putImageData` (legacy worker path unchanged)
- [x] Gallery/output previews in the UI: the main panel blits the composited
      preview ring; gallery tiles poll `preview/module`, which the supervisor
      queues (`Reply::Deferred`) and the window loop answers up to 4/frame
      from an isolated LRU-cached module instance (shader/2D/Point10/ISF)
- [ ] Scene hover previews through `preview/scene` (in flight)
- [x] Shader whole-Scene transitions: the six native effects (crossfade/
      dissolve/pixelate/glitch/wipe/circle) as WGSL passes, the outgoing frame
      captured at transition start, Fade fallback (performance mode always
      fades), manual changes honour `allowedTransitions`
- [x] Rename/removal migration for default Scenes: version-keyed rules,
      seeded-only renames (id/payload preserved) and removals, collision and
      idempotency handling, applied before seeding; the production table is
      empty until the first rename ships with a bump

## M6 — Media — partial

- [x] AudioBands, tempo/phrase detectors, feature smoothing (37 tests)
- [x] Meyda feature extractors, BeatDetektor, MIDI clock → BPM (43 tests)
- [x] OSC server, raw MIDI ingestion, NDI send/receive via runtime FFI
      (51 tests; NDI needs the SDK at runtime)
- [x] Audio analysis parity in Rust: `AudioAnalyser` (2048 Blackman FFT,
      bands/lanes/cells/AGC, tempo/phrase/BeatDetektor, Meyda, smoothing)
      with a 24-frame JS reference fixture within 1 % and a real-music check
- [x] Audio capture primitives: cpal input devices and a native PipeWire
      monitor stream (replaces `pw-record`), bounded queues, `AudioFeed`
- [x] Renderer → engine audio feed (default source): the pump forwards the
      full payload and beats; the engine drives shader/2D/ISF/Point10 audio
- [x] Engine-side input-link evaluation: getter/state/mutation links,
      smoothing, min/max, meta inputs, `inputs/*` actions, 10 Hz deltas
- [x] Engine-owned capture wiring: `--audio-source renderer|pipewire|
      input[:device]` (env `MODV_AUDIO_SOURCE`), `--audio-device`,
      `--list-audio-devices`. The engine pumps `AudioFeed` before links/render,
      owns bands/BPM/kick, ignores the renderer's writes while active
      (warned once) and republishes meter bands at 10 Hz; capture failures
      fall back to the renderer pump. `renderer` stays the default
- [x] Tween runtime (WS-12a): `TweenDescriptor`/`TweenRuntime`/`TweenSet` port
      `buildFrames` exactly (BPM durations, Math.round frame counts,
      loop/alternate, steps, 41 penner easings verified against anime.js);
      Input Links resolve tween paths read-then-advance, `beats/SET_BPM`
      rebuilds durations and `fps/SET_FPS` rebuilds frames
- [ ] Expression assignments (`new Function` in the JS) and JS plugins: a
      documented M7 gap until a sandboxed evaluator/ABI lands
- [ ] Camera/screen/video capture (GStreamer)

## Known gaps (M7 sign-off)

- **Engine stays opt-in (`MODV_ENGINE=1`).** WS-10 media capture (cameras,
  screen/portal capture, video files via GStreamer) has not been ported; Web
  Audio capture still lives in the renderer. Camera/Screen/Video modules
  therefore render their legacy inputs only in the default renderer.
- Expression assignments (the JS `new Function` per input) are not evaluated
  by the engine; no default Scene uses them. JS plugins (`preprocessFrame`
  hooks) are likewise renderer-only.
- Custom/third-party ISF transitions are not available: the six native
  transitions are ported, the picker filters unknown names to Fade.
- 7 of 351 ISF corpus shaders still need translation overrides
  (`docs/isf-corpus-report.md`); they log once and render nothing.
- The packaged engine links `libpipewire-0.3` for system-audio capture; the
  AppImage relies on the host providing it (standard on current distros).
- Shader transitions freeze the outgoing frame and render at full output size
  (the JS re-renders the outgoing groups and caps the transition target at
  1920×1080): behaviour parity, not pixel parity.

## M7 — Parity and release — in progress

- [ ] Full e2e green **against the engine default** (currently opt-in); keep
      `MODV_LEGACY_RENDERER` until parity
- [ ] Performance harness results vs baseline; release packaging verification
      on a clean machine
- [ ] Remaining ISF overrides; goldens for all renderers/scenes
- [ ] User JS modules/plugins known-gap documentation and the Rust plugin ABI
