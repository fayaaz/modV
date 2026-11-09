# Rust hybrid engine plan

Status: proposal / spike branch
Branch: `rust-hybrid`
Last revised: 2026-11-09

This document is the porting specification. It takes the high-level proposal and
expands it into per-subsystem workstreams: what is being ported, which file in
this repository is the source of truth, what the Rust design is, what it
depends on, how it is tested, and what "done" means.

Every file path and line number below refers to this repository at commit
`8a512b6e` (branch `rust-hybrid`). Line numbers are anchors for the reader,
not contracts — they will drift as the code changes.

---

## Goal

Move **all GPU work** — rendering, compositing, transitions, previews, video
upload, shader execution and presentation — into a native Rust process built on
`wgpu`. Electron keeps the control UI (Vue/Vuex), dialogs, menus and IPC.

This is a hybrid, not a UI rewrite: the existing Vue application remains the
shell, but it stops owning WebGL/WebGPU contexts and stops rendering frames.
The Rust engine owns the output window and every GPU surface.

## Why

The current stack is Electron + Chromium + WebGPU/WebGL:

- Chromium refuses Vulkan on Ozone Wayland (`wayland_surface_factory.cc`), so
  native Wayland cannot run Point10's WebGPU scenes; XWayland is required.
- Every frame crosses `OffscreenCanvas` → `transferToImageBitmap` → 2D canvas
  → Chromium compositor.
- The renderer, worker and GPU process all carry Chromium/V8 overhead even when
  the UI is idle.
- Electron's GPU process is a black box for frame pacing, surface format and
  presentation timing.

A native `winit` + `wgpu` output process gets a real Vulkan Wayland surface,
direct presentation, explicit frame pacing and a much smaller CPU/RAM profile.
The GPU-bound shader work itself will not get dramatically faster; the wins are
native Wayland, latency, CPU, compositing and stability.

## Non-goals

- Rewriting the Vue UI in Rust.
- Replacing Vuex as the UI's local state store.
- Changing preset/Scene/playlist JSON formats.
- Supporting native Wayland + WebGPU through Chromium (that is upstream's
  problem).
- Keeping WebGL/WebGPU renderers in Electron after the migration completes.

During migration only, a `MODV_LEGACY_RENDERER=1` flag may keep the current
Electron renderer available for comparison and rollback. It is not part of the
target architecture.

---

## Target architecture

```text
┌──────────────────────────────┐         ┌──────────────────────────────────┐
│ Electron main / preload      │         │ Rust engine process              │
│  - windows, menus, dialogs   │  IPC    │  - winit output window (Wayland/ │
│  - file/media-manager        │◀───────▶│    X11), wgpu Vulkan surface     │
│  - logs, shutdown            │  Unix   │  - render graph + compositor     │
├──────────────────────────────┤  socket │  - module/renderer registry      │
│ Electron renderer (Vue UI)   │ + shm   │  - ISF / shader / 2D / Point10   │
│  - controls, gallery, scenes │         │  - scene engine + transitions    │
│  - preview surfaces (blit   │         │  - audio analysis, tempo/phrase  │
│    of Rust-owned frames)     │         │  - camera/screen/audio capture   │
│  - NO WebGL / NO WebGPU      │         │  - NDI in/out, MIDI clock, OSC    │
└──────────────────────────────┘         └──────────────────────────────────┘
```

### Process model

- One long-lived `modv-engine` process, separate from Electron. A GPU crash or
  driver reset must not take the UI down; the UI can restart the engine and
  restore the last state snapshot.
- The engine owns the output window(s) and all GPU devices, queues, swapchains,
  pipelines and textures.
- Electron main starts/stops the engine, supervises it, and reports crashes to
  the UI.
- Optionally the engine can run as a library behind a Node-API addon later, but
  the default is a separate process for crash isolation and clean lifecycle.

Supervision contract (see [WS-14](#ws-14-electron-integration-lifecycle-and-packaging)):

| Event                                     | Electron main action                                       | Engine action                                 |
| ----------------------------------------- | ---------------------------------------------------------- | --------------------------------------------- |
| App start                                 | spawn `modv-engine --socket <path> --state <path>`         | bind socket, load snapshot, open window       |
| Engine exits 0                            | mark stopped, notify UI                                    | —                                             |
| Engine exits non-zero / SIGSEGV / SIGABRT | restart after 500 ms (max 5 in 60 s), replay last snapshot | —                                             |
| Engine hangs (no heartbeat for 5 s)       | SIGTERM, then SIGKILL after 1 s                            | —                                             |
| `modv-destroy` / window close             | send `engine/shutdown`, wait ≤ 1 s, then kill              | flush snapshot, release GPU resources, exit 0 |

### Rust crate layout

```text
crates/
  modv-engine/       # app state, groups/modules graph, scenes, timing,
                     # expressions, input links, preset (de)serialisation
  modv-render/       # wgpu device/queue, render graph, compositor,
                     # texture pool, surface/present, frame pacing
  modv-renderers/    # renderer implementations:
                     #   isf/  shader/  2d/  point10/  three/
  modv-media/        # cameras (v4l2/gstreamer), PipeWire screen + system
                     # audio, video decode, NDI, audio features
  modv-ipc/          # protocol, framing, shared-memory frame transport
  modv-app/          # engine binary: winit window(s), supervisor loop
```

Suggested crates: `wgpu`, `winit`, `naga` (with the `glsl-in` feature for the
ISF translator), `raw-window-handle`, `bytemuck`, `serde`/`serde_json`,
`tokio` (IPC/media async), `cpal`, `rustfft` or `realfft`, `rosc`,
`pipewire`/`ashpd` (portal), `gstreamer` (capture + decode), `ndi` bindings /
direct NDI SDK FFI, `cosmic-text` + `fontdb` for text, `tracing` for logs.

The 2D facade is written in-house rather than taken from a dependency — see
[WS-5](#ws-5-2d-renderer-and-canvas-facade) for the reasoning and the
alternatives that were considered.

`midir` is deliberately **not** on this list: MIDI device enumeration stays in
Electron, where Web MIDI provides the device model the UI needs — see
[WS-11](#ws-11-ndi-midi-osc).

### State planes

The worker store (`src/renderer/src/application/worker/store/`, 27 modules,
~12.8k LOC) is today the single source of truth for both rendering and UI
control. After the split it becomes **two planes**:

| Plane            | Owner      | Contents                                                                                                                                                         | Communication                                        |
| ---------------- | ---------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------- |
| **Engine plane** | Rust       | groups, modules, inputs, expressions\*, tweens\*, scenes, performance, outputs, size, beats, meyda, images, videos, renderers, plugins\*, ndi, osc, scenePreview | authoritative; UI sends intents, engine emits deltas |
| **UI plane**     | JavaScript | `ui-store/*` (389 LOC) + a read-only mirror of the engine plane                                                                                                  | engine → UI deltas over the control socket           |

\* `expressions`, `tweens` and `plugins` start in the UI plane and are ported
last — see [WS-12](#ws-12-expressions-tweens-and-plugins) for the rationale
and the migration trigger.

Store-module → plane mapping:

| Store module   |  LOC | Plane            | Note                                                                    |
| -------------- | ---: | ---------------- | ----------------------------------------------------------------------- |
| `groups`       |  584 | Engine           | the render graph                                                        |
| `modules`      |  887 | Engine           | module registry + active instances                                      |
| `scenes`       | 4211 | Engine           | largest single port; scene/performance state machine                    |
| `inputs`       |  277 | Engine           | inputs + input links                                                    |
| `outputs`      |  266 | Engine           | canvases → GPU textures                                                 |
| `size`         |   51 | Engine           | output resolution / DPR                                                 |
| `beats`        |   52 | Engine           | BPM + kick                                                              |
| `meyda`        |  117 | Engine           | feature bag + bands                                                     |
| `dataTypes`    |  149 | Engine           | prop coercion rules                                                     |
| `errors`       |   36 | Engine           | module error status                                                     |
| `images`       |   56 | Engine           | image texture registry                                                  |
| `videos`       |  125 | Engine           | video decode → textures                                                 |
| `renderers`    |   26 | Engine           | renderer registry                                                       |
| `scenePreview` |  171 | Engine           | hover-preview render                                                    |
| `ndi`          |  246 | Engine           | NDI sender/receivers                                                    |
| `osc`          |   80 | Engine           | OSC servers                                                             |
| `plugins`      |  156 | UI (then Engine) | JS hooks; see WS-12                                                     |
| `expressions`  |  176 | UI (then Engine) | JS `new Function`; see WS-12                                            |
| `tweens`       |  264 | UI (then Engine) | anime.js easing; see WS-12                                              |
| `fonts`        |   30 | Engine           | font registry for 2D text                                               |
| `fps`          |   31 | Engine           | frame pacing target                                                     |
| `metrics`      |   15 | Engine           | measured FPS                                                            |
| `mediaStream`  |  160 | UI               | device enumeration/constraints (Web API only)                           |
| `media`        |   67 | UI               | media-manager → module/ISF payloads                                     |
| `midi`         |   76 | UI + Engine      | Web MIDI device model in the UI; MIDI clock → BPM in the engine (WS-11) |
| `projects`     |   22 | UI               | project name                                                            |
| `windows`      |   44 | UI + Engine      | UI lists them, engine owns native windows                               |

### Electron responsibilities

- Vue UI, Vuex UI store, Linux window management, modal dialogs.
- Media-manager file watching and read/save handlers (unchanged).
- Settings persistence currently held by `mediaStream`/UI state.
- Gallery thumbnails and previews: request a frame from Rust and blit it; no
  GPU context in the renderer.
- Menu, shutdown coordination, log forwarding.

### IPC and frame transport

- Control: versioned JSON or `bincode` messages over a Unix domain socket.
  Request/response for commands, pub/sub for engine events and state deltas.
- Frame previews: shared memory ring buffer (`/dev/shm` or `memfd`) with
  width/height/stride/format/frame id metadata. The engine writes BGRA/RGBA;
  Electron uploads to a 2D canvas (or uses `ImageBitmap`). This is a display
  copy only, not GPU work in Electron.
- On Linux, prefer DMA-BUF sharing for zero-copy preview; fall back to shared
  memory for portability.
- Backpressure: never queue unbounded frames; drop stale previews.
- The engine is authoritative for render state. The UI sends intents
  (`applyScene`, `setProp`, `startPerformance`, …) and receives snapshots.

The wire format is specified in [Protocol specification](#protocol-specification).

### Rendering pipeline in Rust

1. `winit` creates a native Wayland (or X11 fallback) window per output.
2. `wgpu` creates a Vulkan surface/device/queue and swapchain.
3. A render graph evaluates Groups in row order, each pass reading the previous
   row's texture (`lastCanvas` equivalent) and writing its own target.
4. Modules are GPU passes:
   - ISF: parse ISF JSON, upload uniforms/textures, run fragment pass.
   - shader: GLSL/TSL-equivalent WGSL pipelines with prop uniforms.
   - 2D: a canvas-like draw API implemented as a small `wgpu` 2D path, with
     text via `cosmic-text`.
   - Point10: WGSL compute/render ports of the TSL scenes.
   - three: either WGSL ports or a Rust scene-graph renderer; last priority.
5. Transitions and performance mode: same row/group machinery, blend the
   outgoing ghost with the incoming Group in a scratch texture.
6. Present: write the final texture to the swapchain; pace with Wayland frame
   callbacks / `wp_presentation` where available.

No CPU readbacks on the hot path. Reuse textures via a pool. Cache pipelines
and bind groups. Keep a persistent `wgpu::Device` for the engine lifetime.

---

## Wayland plan

- Use `winit` with native Wayland by default, X11 fallback.
- Fullscreen/borderless, fractional scale, multi-monitor and output selection
  handled by the engine (one window per configured output).
- Handle `wl_surface.frame` callbacks for pacing; use presentation feedback
  when available for latency stats.
- Keep XWayland only as a fallback for drivers/compositors that fail native
  Wayland.
- The output is a genuine Wayland surface with Vulkan; Chromium's
  `wayland_surface_factory` restriction never applies.

Notes and caveats:

- `winit` does not expose `wl_surface.frame` callbacks. Frame pacing therefore
  uses `wgpu::PresentMode::Fifo` (vsync) by default, and presentation
  statistics come from Vulkan presentation timestamps
  (`VK_KHR_present_wait` / `VK_GOOGLE_display_timing`) where the compositor
  exposes them. If a raw frame-callback handle is needed later, add
  `wayland-client` alongside `winit` and drive the callback manually; do not
  fork `winit`.
- Fractional scale: `winit` reports the scale factor; the engine renders at
  `logical_size * scale` and presents to a `PhysicalSize` surface. Under X11
  the fractional-scale path is unavailable, so the engine falls back to
  integer scale.
- Multi-monitor: one `winit::Window` per configured output, each with its own
  `wgpu::Surface`; the engine keeps one `wgpu::Device` and one queue for all of
  them.
- Compositor/driver variance is the biggest Wayland risk; the spike (M0) must
  be run against at least labwc, GNOME (Mutter) and KDE (KWin).

---

## Porting workstreams

| ID                                                           | Area                         | Source of truth                                               | Size                    | Milestone             |
| ------------------------------------------------------------ | ---------------------------- | ------------------------------------------------------------- | ----------------------- | --------------------- |
| [WS-1](#ws-1-engine-state-model)                             | Engine state model           | `worker/store/modules/*`                                      | ~12.8k LOC              | M1–M2                 |
| [WS-2](#ws-2-render-loop-and-compositor)                     | Render loop + compositor     | `worker/loop.js`, `worker/index.worker.js`                    | ~1.3k LOC               | M1–M2                 |
| [WS-3](#ws-3-shader-renderer)                                | Shader renderer              | `renderers/shader/`                                           | 1.3k LOC + 37 modules   | M2                    |
| [WS-4](#ws-4-isf-renderer)                                   | ISF renderer                 | `renderers/isf/`                                              | 313 LOC + 387 shaders   | M2                    |
| [WS-5](#ws-5-2d-renderer-and-canvas-facade)                  | 2D renderer + canvas facade  | `renderers/2d/`                                               | 210 LOC + 15 modules    | M3                    |
| [WS-6](#ws-6-three-renderer)                                 | three renderer               | `renderers/three/`                                            | 210 LOC + 1 module      | M3                    |
| [WS-7](#ws-7-point10-scenes)                                 | Point10 scenes               | `application/point10/`                                        | ~16.8k LOC / 25 scenes  | M4                    |
| [WS-8](#ws-8-scenes-and-performance-mode)                    | Scenes + performance         | `worker/store/modules/scenes.js` + `worker/scene-*.js`        | ~5.5k LOC               | M5                    |
| [WS-9](#ws-9-audio-analysis-and-transport)                   | Audio analysis + transport   | `application/audio-bands.js` etc.                             | ~1.6k LOC               | M2 / M6               |
| [WS-10](#ws-10-media-capture-cameras-screen-video)           | Cameras, screen, video       | `application/index.js`, `setup-media.js`                      | ~2.4k LOC               | M6                    |
| [WS-11](#ws-11-ndi-midi-osc)                                 | NDI, MIDI, OSC               | `worker/ndi-worker.worker.js` etc.                            | ~0.9k LOC               | M6                    |
| [WS-12](#ws-12-expressions-tweens-and-plugins)               | Expressions, tweens, plugins | `store/modules/{expressions,tweens,plugins}.js`               | ~0.6k LOC               | M5 (UI) / M7 (Engine) |
| [WS-13](#ws-13-preset-scene-and-playlist-serialisation)      | Preset/Scene/playlist        | `worker/preset-serializer.js`, `store/modules/common/swap.js` | 214 LOC + 6.5k LOC data | M2 / M5               |
| [WS-14](#ws-14-electron-integration-lifecycle-and-packaging) | Electron integration         | `src/main/**`                                                 | ~2.4k LOC               | M1                    |
| [WS-15](#ws-15-testing-and-benchmarking)                     | Testing + benchmarks         | `tests/**`                                                    | new                     | every                 |

---

## WS-1 Engine state model

**Source.** `src/renderer/src/application/worker/store/` — 27 modules,
12.8k LOC including `scenes.js` (4211) and `default-scenes.js` (4243, data
only). The store is created at `worker/store/index.js:46-78` and is
authoritative inside the worker.

**Target.** A Rust state model that owns everything the renderer needs to draw
a frame, plus a delta stream that keeps the Vue mirror store valid.

### Core types

```rust
pub struct EngineState {
    pub groups:        IndexMap<GroupId, Group>,
    pub group_order:   Vec<GroupId>,
    pub modules:       ModuleRegistry,   // registered + active instances
    pub inputs:        IndexMap<InputId, Input>,
    pub input_links:   IndexMap<InputId, InputLink>,
    pub expressions:   IndexMap<ExpressionId, Expression>,
    pub tweens:        IndexMap<TweenId, Tween>,
    pub scenes:        IndexMap<SceneId, Scene>,
    pub performance:   PerformanceConfig,
    pub active_rows:   BTreeMap<usize, RowRuntime>,
    pub row_transitions: BTreeMap<usize, RowTransition>,
    pub transition:    Option<WholeSceneTransition>,
    pub outputs:       OutputRegistry,    // main + aux surfaces + windows
    pub size:          OutputSize,
    pub beats:         Beats,
    pub bands:         BandPayload,
    pub images:        IndexMap<ImageId, TextureHandle>,
    pub videos:        IndexMap<VideoId, TextureHandle>,
}
```

`Group` mirrors the typedef at `store/modules/groups.js:8-81`:

```rust
pub struct Group {
    pub id: GroupId,
    pub name: String,
    pub hidden: bool,
    pub modules: Vec<ModuleId>,          // draw order
    pub enabled: GroupEnabled,            // Disabled | Drawn | DrawnNotToOutput
    pub alpha: f32,
    pub inherit: bool,
    pub inherit_from: InheritFrom,        // None | Previous | Group(GroupId)
    pub pipeline: bool,
    pub clearing: bool,
    pub composite_operation: BlendMode,
    pub alpha_input_id: Option<InputId>,
    pub enabled_input_id: Option<InputId>,
    pub clearing_input_id: Option<InputId>,
    pub inherit_input_id: Option<InputId>,
    pub composite_operation_input_id: Option<InputId>,
    pub pipeline_input_id: Option<InputId>,
    pub scene_id: Option<SceneId>,        // scene machinery
    pub scene_row: Option<usize>,         // scene machinery
    pub transition_ghost: bool,           // scene machinery
}
```

`ModuleInstance` mirrors `store/modules/modules.js:660-683`:

```rust
pub struct ModuleInstance {
    pub id: ModuleId,
    pub module_name: String,              // registry key, e.g. "Bands"
    pub meta: ModuleMeta,                 // name/type/author/version/category/...
    pub props: BTreeMap<String, PropValue>,
    pub data: serde_json::Value,          // renderer-specific, JSON-safe
    pub enabled: bool,
    pub alpha: f32,
    pub composite_operation: BlendMode,
    pub status: Option<ModuleStatus>,
}
```

### Prop model

Prop types come from `store/modules/dataTypes.js`. The engine must implement
the same coercion pipeline that `modules/updateProp` performs at
`store/modules/modules.js:481-562`:

1. `dataTypes[type].create` — coerce the incoming value into the type's
   canonical shape.
2. Apply an attached expression (see WS-12).
3. Clamp to `[min, max]` when the prop is `strict`.
4. `abs` when the prop sets `abs`.
5. `round` for `int`.

| Prop type                                              | Canonical value                                               | Source                 |
| ------------------------------------------------------ | ------------------------------------------------------------- | ---------------------- |
| `text`, `int`, `bool`, `event`, `float`, `enum`, `osc` | scalar                                                        | `dataTypes.js`         |
| `vec2`, `vec3`, `vec4`                                 | `[f32; N]`                                                    | `dataTypes.js:17-28`   |
| `color`                                                | `{r,g,b,a}`                                                   | `dataTypes.js:32-35`   |
| `texture`                                              | texture handle + source kind (image / video / canvas / group) | `dataTypes.js:37-86`   |
| `tween`                                                | current tween value                                           | `dataTypes.js:122-127` |

### Input links

`store/modules/inputs.js` defines four link types: `action`, `commit`,
`state`, `getter`, `mutation`. The engine must implement:

- **getter / state** — read from the engine state by path, apply
  `resolveInputLinkValue` semantics (`worker/input-link-value.js:9-22`): a
  `null`/`undefined`/`NaN` source value must **not** overwrite the saved value.
  This is the bug class that produced `NaN` props; it is a hard requirement.
- **mutation** — subscribe to a mutation type, optionally with a payload
  matcher, and fire the target commit. This is the mutation bus at
  `worker/index.worker.js:76-121`. Because the engine owns the mutation bus
  natively, this becomes a direct subscription rather than a store watcher.
- **commit** — dispatch a Vuex-style mutation.

### Blend modes

`src/renderer/src/util/composite-operations.js` exposes 30 modes:

- 15 separable blend modes (`normal`, `multiply`, `overlay`, `darken`,
  `lighten`, `color-dodge`, `color-burn`, `hard-light`, `soft-light`,
  `difference`, `exclusion`, `hue`, `saturation`, `color`, `luminosity`).
- 15 Porter-Duff composite modes (`clear`, `copy`, `destination`,
  `source-over`, `destination-over`, `source-in`, `destination-in`,
  `source-out`, `destination-out`, `source-atop`, `destination-atop`, `xor`,
  `lighter`).

All Porter-Duff modes and 12 of the 15 separable modes map directly onto a
`wgpu::BlendState` (src/dst factor + operation). The three non-separable modes
that are _not_ expressible as fixed-function factors — `hue`, `saturation`,
`color`, `luminosity` (the W3C non-separable set) — need a shader-based blend
pass. Implement one `blend.wgsl` implementing the W3C non-separable blend
functions and route those four modes through it.

### Delta stream

The engine emits the same `{type, payload}` records the worker's `commitQueue`
produces today (`worker/index.worker.js:136-144`), so the renderer's mirror
replay (`application/index.js:199-208`) is unchanged. The engine must apply
the same per-frame filtering the worker does today:

- `modules/UPDATE_ACTIVE_MODULE` is only queued when the changed key is
  `props` or `meta` (`index.worker.js:69-74`).
- `beats/SET_KICK` is de-duplicated against the previous kick
  (`index.worker.js:53-63`).
- `beats/SET_BPM` and `fps/SET_FPS` additionally drive `tweens/updateBpm`
  (`index.worker.js:49-51`).

**Done when.** A Rust unit test can load a preset, mutate every store module,
and produce a delta stream that, replayed into a JavaScript mirror store,
yields a deep-equal state. This is the parity gate for WS-13.

---

## WS-2 Render loop and compositor

**Source.** `src/renderer/src/application/worker/loop.js` (738 LOC) and
`worker/index.worker.js` (604 LOC).

### The JavaScript frame, today

1. `scenes/tick` with a real elapsed delta (`loop.js:94-111`).
2. FFT texture upload into a 512×1 aux canvas (`loop.js:113-127`).
3. Input-link evaluation (`loop.js:129-192`).
4. Plugin pre-process hooks (`loop.js:194-204`).
5. Renderer `tick` hooks — currently a no-op set (`loop.js:206-210`).
6. Hover preview render, capped at 30 fps (`loop.js:215-229`).
7. Group loop (`loop.js:239-553`).
8. Clear main canvas; apply a whole-scene transition if one is running
   (`loop.js:555-621`).
9. Composite groups onto the main canvas in `groupIndexRenderOrder`
   (`loop.js:622-658`).
10. Blit main → each output window (`loop.js:660-669`).
11. Debug canvas (`loop.js:679-697`), plugin post-process (`loop.js:699-710`).
12. NDI export — **must be last** because `transferToImageBitmap` consumes the
    canvas (`loop.js:712-724`).

### The Rust frame

```rust
pub fn render_frame(&mut self, dt: Duration) -> FrameReport {
    self.tick_scenes(dt);                 // 1
    self.update_tweens(dt);               // 2 (engine-side tweens)
    self.evaluate_input_links();          // 3
    self.render_scene_preview_if_due();   // 6
    let graph = self.build_render_graph();// 7
    self.execute(&graph);                 // 7
    self.present();                       // 9/10
    self.export_ndi_if_enabled();         // 12
}
```

### Render graph

```rust
pub struct FrameGraph {
    pub nodes: Vec<PassNode>,
    pub textures: TexturePool,
}

pub enum PassNode {
    Module {
        group: GroupId,
        module: ModuleId,
        pipeline_slot: usize,
        target: TextureHandle,
    },
    Composite {
        source: TextureHandle,
        target: TextureHandle,     // the accumulating output
        alpha: f32,
        blend: BlendMode,
    },
    RowFade {
        from: TextureHandle,
        to: TextureHandle,
        target: TextureHandle,
        progress: f32,
        from_alpha: f32,
        to_alpha: f32,
    },
    SceneTransition {
        from: TextureHandle,
        to: TextureHandle,
        target: TextureHandle,
        kind: TransitionKind,
        progress: f32,
    },
    Present {
        source: TextureHandle,
    },
}
```

Semantics that must be reproduced exactly (each has caused a visible bug
before):

- **Group order.** `groupIndexRenderOrder` (`store/modules/groups.js:111-123`)
  moves the Gallery group to the end. The engine must not assume "last entry"
  is the Gallery group — `loop.js:630` deliberately skips the Gallery group
  **by name**, because gallery-less layouts render every row.
- **Alpha culling.** A group with `alpha < 0.001` is skipped, _except_ when it
  is part of a transition (`loop.js:257-266`). Transition groups must exist at
  both ends of a fade.
- **Inheritance.** `inheritFrom == -1` means the previous group in render
  order (`lastCanvas`); a UUID means that specific group. Resolution lives in
  `worker/group-inherit.js:10-28`. The **first** group of a frame inherits
  `null`, never the previous frame's root canvas — feeding it back makes
  transparent scenes accumulate trails (`loop.js:333-374`).
- **Gallery inheritance.** The Gallery group only inherits when it has an
  auxiliary target; otherwise the copy lands in a full-size canvas nobody
  reads and forces a GPU sync (`loop.js:333`).
- **Pipeline mode.** When `group.pipeline` is set, the first module draws into
  the group target and each subsequent module draws into a scratch texture
  that is composited back (`loop.js:391-402`, `renderers/2d/index.js:48-74`).
  This is an extra full-frame copy per module in the current implementation;
  the Rust graph can express it as a linear chain without the copy.
- **Row transitions.** A fading row keeps the old Group alive as a
  `transitionGhost`. `worker/row-transition.js:71-115` blends the outgoing
  ghost and the incoming Group into a per-row scratch canvas; that blend is
  assigned to **both** the incoming and ghost group ids so the next row
  inherits the crossfade, not just the incoming canvas
  (`loop.js:530-535`). A ghost must not become `lastCanvas`
  (`loop.js:545-552`).
- **Whole-scene transition.** `worker/scene-transition.js` composes the
  from/to group sets into two textures, runs an ISF transition shader in a
  bounded render target (`worker/transition-sizing.js:8-40`, cap
  `1920*1080` pixels) and caches up to 4 compiled ISF renderers
  (`createBoundedCache`).

### Texture pool

Textures are pooled by `(width, height, format, usage-class)` and evicted LRU.
The pool must support:

- group targets (RGBA8, render + sample),
- pipeline scratch (RGBA8),
- row-fade scratch (one per row, ≤16 rows),
- transition scratch (bounded size),
- input textures (video / webcam / screen / image),
- FFT texture (512×1 RGBA8 — the same layout the shader renderer expects at
  `renderers/shader/index.js:260-278`).

**Done when.** A Rust test renders a two-group scene with a chained inherit and
a row fade and compares the output against a reference PNG produced by the
legacy Electron renderer.

---

## WS-3 Shader renderer

**Source.** `src/renderer/src/application/renderers/shader/`
(`index.js` 372 LOC, `default-shader.js` 89 LOC, `utils.js` shared) plus
37 shader modules:

- 6 single-effect modules: `Bands.js`, `ChromaticAbberation.js`, `Fisheye.js`,
  `MattiasCRT-2.0.js`, `Plasma.js`, `Wobble.js` (5 of them backed by a `.frag`
  file, 181 LOC total).
- `Point10Effects.js` (675 LOC) exporting **31** inline-GLSL effects.

**Target.** Direct WGSL pipelines. The GLSL corpus is small (~800 LOC), so it
is hand-ported rather than machine-translated. A user-supplied `.frag` is not a
media type today (the media manager handles `module`, `isf`, `image`, `video`,
`palette`, `preset`, `scene` only), so arbitrary-GLSL support is optional —
but the translator in WS-4 will be able to serve it later.

### Uniform contract

`renderers/shader/index.js:90-126` defines the uniform block that every
shader receives. The Rust port must expose exactly the same names and types:

| Uniform                       | Type              | Source                                             |
| ----------------------------- | ----------------- | -------------------------------------------------- |
| `iGlobalTime`, `iFrame`       | `f32`             | `worker/frame-counter.js`                          |
| `iDate[4]`                    | `vec4<f32>`       | wall clock                                         |
| `iTime`, `iTimeDelta`         | `f32`             | frame clock                                        |
| `iResolution[3]`              | `vec3<f32>`       | width, height, dpr                                 |
| `iChannel0..3`                | `texture_2d<f32>` | the group canvas (all four alias the same texture) |
| `iChannelResolution[4][3]`    | `vec3<f32>`       | per-channel resolution                             |
| `u_modVCanvas`                | `texture_2d<f32>` | alias of `iChannel0`                               |
| `u_fft`                       | `texture_2d<f32>` | 512×1 FFT texture                                  |
| `u_fftResolution`             | `vec2<f32>`       | 512×1                                              |
| `u_delta`, `u_time`           | `f32`             | frame clock                                        |
| `u_kick`                      | `bool`            | beat flag                                          |
| `u_bass`, `u_mid`, `u_treble` | `f32`             | band payload                                       |

Module props are added on top, overriding the defaults
(`renderers/shader/index.js:125`, `:156-169`). A prop of type `texture`
contributes its texture handle; every other prop contributes its raw value.

### Wrapper semantics

`renderers/shader/index.js:140-144` wraps a fragment shader that lacks
`gl_FragColor` in a `#version 300 es` shim exposing
`mainImage(out vec4, in vec2)`, and swaps in a matching vertex shader. The
Rust port keeps the same two shader flavours (legacy `gl_FragColor` and the
`mainImage` shim) and ports `default-shader.js` accordingly.

### Context split

The renderer keeps **two** WebGL2 contexts today — `shader-buffer` (live) and
`shader-buffer-gallery` (preview) — so a preview cannot thrash the live
drawing buffer (`renderers/shader/index.js:62`, `:71`). In wgpu there is one
device; the equivalent isolation is _separate textures_, not separate
contexts. Live and gallery passes therefore get separate targets and separate
pipeline caches keyed by (shader, target format).

### Per-frame canvas upload

Today the whole group canvas is re-uploaded to a GL texture every frame
(`renderers/shader/index.js:253-257`). In the Rust engine the group canvas is
already a GPU texture, so the "upload" is a bind-group swap with no copy —
a direct latency and bandwidth win. The same applies to the `three` renderer's
`inputTexture` (`renderers/three/index.js:84`).

**Done when.** All 37 shader modules render in the Rust engine and produce
golden images matching the legacy renderer within a tolerance (see
[WS-15](#ws-15-testing-and-benchmarking)).

---

## WS-4 ISF renderer

**Source.** `src/renderer/src/application/renderers/isf/`:

- `index.js` (313 LOC) — the renderer.
- `float-buffer-support.js` (64 LOC) — monkey-patches
  `ISFTexture.prototype.setSize` so PERSISTENT FLOAT buffers allocate
  `RGBA32F` with `NEAREST` filtering. Required because the vendored ISF
  renderer used an unsized RGBA internal format, which WebGL2 rejects.
- `modules/isf/` — **351** `.fs` shaders and **36** `.vs` vertex sidecars.

Corpus characteristics (measured):

| Property                               |                                                                     Count |
| -------------------------------------- | ------------------------------------------------------------------------: |
| `.fs` shaders                          |                                                                       351 |
| `.vs` sidecars                         |                                                                        36 |
| Multi-pass (`PASSES` array)            |                                                                        63 |
| `PERSISTENT` buffers (needs `RGBA32F`) |                                                                        43 |
| Shaders with an `image` input          |                                                                       288 |
| Total input declarations               | 1096 float, 392 image, 122 color, 117 bool, 89 point2D, 73 long, 15 event |

**Target.** A Rust ISF runtime: parser, macro expander, uniform generator,
multi-pass renderer — plus a GLSL ES → WGSL translator.

### The ISF runtime

The JavaScript implementation is the npm dependency
`interactive-shader-format` (a `vcync/interactive-shader-format-js` fork).
It is not vendored in this tree, so the Rust port must reimplement three
pieces that the renderer uses:

1. **`ISFParser`** — parse the JSON header out of the leading `/*{ ... }*/`
   block, extract `INPUTS`, `PASSES`, `ISFVSN`, `CREDIT`, `DESCRIPTION`,
   `CATEGORIES`, and expand the ISF macro set.
2. **`ISFUpgrader`** — `renderers/isf/index.js:145-179` upgrades ISF v1
   (`gl_FragColor`, `texture2D`) to v2 before compiling. The Rust port needs
   the same upgrade pass.
3. **`ISFRenderer`** — owns the render buffers (one per `PASSES` entry), the
   image-input textures, and the uniform generation.

### ISF macro set

ISF shaders reference runtime-provided macros and functions that are not in
the `.fs` file:

| Macro                           | Meaning                                         |
| ------------------------------- | ----------------------------------------------- |
| `RENDERSIZE`                    | `vec2` render target size                       |
| `PASSINDEX`                     | `int` index of the current pass                 |
| `PI`                            | `float` π                                       |
| `isf_FragCoord`                 | `vec4` fragment coordinate                      |
| `isf_FragNormCoord`             | `vec2` normalised fragment coordinate           |
| `isf_vertShaderInit()`          | vertex-shader preamble                          |
| `IMG_NORM_PIXEL(target, coord)` | normalised pixel coordinate into a named target |
| `IMG_SIZE(target)`              | size of a named target                          |
| `IMG_PIXEL(target, coord)`      | pixel coordinate into a named target            |

The Rust runtime supplies these as WGSL constants/functions injected into the
translated shader, exactly as the JS runtime does.

### Prop synthesis

`renderers/isf/index.js:181-281` synthesises module props from the parsed
inputs. The Rust port must produce the identical prop schema so saved presets
stay valid:

| ISF input `TYPE` | Prop type | Notes                                          |
| ---------------- | --------- | ---------------------------------------------- |
| `float`          | `float`   | `label`, `default`, `min`, `max`, `step: 0.01` |
| `bool`           | `bool`    | `default: Boolean(input.DEFAULT)`              |
| `long`           | `enum`    | `enum` built from `VALUES` + `LABELS`          |
| `color`          | `vec4`    | `default: input.DEFAULT`                       |
| `point2D`        | `vec2`    | `min`, `max`, `default: [0,0]`                 |
| `image`          | `texture` | also sets `meta.previewWithOutput = true`      |
| `event`          | `event`   |                                                |

`CREDIT`/`DESCRIPTION`/`VSN` map to `meta.author`/`description`/`version`, and
the Gallery category is derived the same way (`getIsfGalleryCategory`).

### GLSL ES → WGSL translation

The translator is the highest-risk single component in the plan. Strategy:

1. **Preprocess** in Rust: expand the ISF macro set, upgrade ES 1.00 → 3.00
   (`gl_FragColor` → out var, `texture2D` → `texture`, `varying` → in/out,
   `gl_FragCoord` handling).
2. **Translate** with `naga`'s GLSL front-end (`glsl-in` feature) → SPIR-V →
   `naga::back::wgsl` → WGSL. `naga` supports embedded GLSL versions, which
   is what ISF uses.
3. **Fall back** to a hand-written WGSL override when translation fails.
   Overrides live in `crates/modv-renderers/isf/overrides/<shader-name>.wgsl`
   and are selected by shader name.
4. **Measure** — a corpus test compiles all 351 shaders and writes a report of
   which ones translate cleanly and which need overrides. The report is a
   tracked artefact so progress is visible and regressions are caught.

`float-buffer-support.js` disappears as a concern: the Rust runtime allocates
`Rgba32Float` textures natively for `PERSISTENT` FLOAT buffers, with
`FilterMode::Nearest`.

**Done when.** ≥ 95 % of the 351 shaders translate with no override, the
remaining ≤ 18 have overrides, and all 351 render a golden image. The
`float` and multi-pass paths are covered by dedicated tests (the multi-pass
corpus is the 63 `PASSES` shaders; the float corpus is the 43 `PERSISTENT`
shaders).

---

## WS-5 2D renderer and canvas facade

**Source.** `src/renderer/src/application/renderers/2d/index.js` (210 LOC) and
`renderers/2d/modules/`.

The 41 module files split into 15 hand-written 2D modules and 26 Point10
wrappers (8 LOC each, handled by WS-7).

| Hand-written module | LOC | Module           | LOC |
| ------------------- | --: | ---------------- | --: |
| `Ball.js`           | 230 | `Text.js`        | 185 |
| `Polygon.js`        | 173 | `Texture2d.js`   | 166 |
| `Buffer.js`         | 164 | `Concentrics.js` | 158 |
| `X-Drips.js`        | 157 | `Line.js`        | 137 |
| `Pixelate.js`       | 108 | `Waveform.js`    | 102 |
| `GridStretch.js`    | 100 | `Bar.js`         |  73 |
| `Webcam.js`         |  62 | `Smear.js`       |  42 |
| `Counter.js`        |  35 |                  |     |

### Canvas2D API surface actually used

Measured across all 41 module files, the facade must support exactly these 22
operations:

```
arc, beginPath, canvas, clearRect, clip, closePath, drawImage, fill,
fillRect, fillStyle, fillText, globalAlpha, imageSmoothingEnabled, lineTo,
lineWidth, moveTo, restore, rotate, save, stroke, strokeStyle, translate
```

Not used anywhere in the current corpus: gradients, patterns, bezier curves,
`ellipse`, `scale`, `setTransform`, `measureText`, `getImageData`,
`putImageData`, `strokeRect`, `quadraticCurveTo`.

That is a very small surface, and it changes the answer to the plan's open
question about `skia-safe` vs `vello`:

**Recommendation: implement a small `wgpu` 2D facade in Rust.** Reasons:

- The required API is 22 methods with no gradients/patterns/curves; a GPU 2D
  rasteriser for that is a few thousand lines, not a dependency.
- `skia-safe` pulls in a second GPU backend and a large binary, which defeats
  the CPU/RAM and packaging goals of the migration.
- `vello` is a CPU rasteriser; it would reintroduce a CPU readback per 2D
  module, which is exactly what the plan is trying to remove.
- The one genuinely hard operation is text (`fillText`/`strokeText`, used only
  by `Text.js` via `canvas-text-wrapper`). `cosmic-text` + `fontdb` covers it:
  shape the text on the CPU, upload glyph coverage as an `R8` texture, draw it
  with the same path pipeline.

The facade needs: a path builder (move/line/arc/close, flattened to
polylines), a fill/stroke rasteriser (scanline with analytic or supersampled
coverage), a transform stack, a clip stack (rectangular clip only is enough
for the current corpus — `clip()` is used once), a `drawImage` blitter with
bilinear filtering, and a text layer.

**Recommendation for text**: keep `canvas-text-wrapper`'s behaviour (word
wrap, alignment) by reimplementing its small API in Rust (`TextWrapper` with
`textAlign: center` and stroke support) over `cosmic-text`.

### Module contract

A 2D module exports `{meta, props, init?, resize?, update?, draw?}`. From
`renderers/2d/modules/Ball.js`:

- `meta`: `{name, author, version, type: "2d", audioFeatures: [...]}`.
- `init({data, canvas})` → `data` — called once at activation.
- `resize({data, canvas})` → `data`.
- `update({data, props, canvas, delta, store})` → `data` — may mutate in place.
- `draw({data, props, context, features, meyda, delta, bpm, kick, video,
videos, moduleId, isGallery})`.

`features` is the Meyda feature bag; the corpus uses `features.zcr`,
`features.rms` and `features.buffer`.

**Porting note:** the 15 hand-written 2D modules are JavaScript. They cannot
run inside the Rust engine. Two options:

- **(a)** Reimplement each as a Rust module. 15 modules, ~2.1k LOC of
  straightforward 2D drawing — mechanical work, low risk.
- **(b)** Run them in a JavaScript sandbox in the engine (e.g. `boa_engine` or
  a QuickJS runtime) and have them drive the Rust 2D facade through a
  host-call interface. High flexibility, much higher risk and latency.

**Recommendation: (a)** — hand-port the 15 modules. Keep option (b) as a
possible escape hatch for _user-authored_ 2D modules (the media manager's
`module/` folder lets users drop in a `.js` module), which is the only case
that genuinely needs a JS runtime in the engine. That user-module case is
listed as a known gap in M7, not a blocker for parity of the built-in corpus.

**Done when.** All 15 hand-written modules plus `Text.js` render in the Rust
engine with golden-image parity, and the facade passes a unit test suite
covering the 22 operations.

---

## WS-6 three renderer

**Source.** `src/renderer/src/application/renderers/three/index.js` (210 LOC)
and `renderers/three/modules/Cube.js` (119 LOC). This is the only `three`
module in the tree.

**Target.** A native WGSL cube renderer. Porting the three.js runtime itself
is not warranted for one module: `Cube.js` is a textured cube with
rotation/scale/position/colour props and an optional input texture.

Notes from the current implementation that the port must respect:

- The group canvas is uploaded into a `THREE.CanvasTexture` every frame
  (`renderers/three/index.js:84`). In Rust this is a bind-group swap.
- `threeModuleData` is keyed by `meta.name`, **not** by module id
  (`renderers/three/index.js:86`, `:161`), so two instances of the same module
  collide. The Rust port should key by module id — a behaviour change, but a
  bug fix, and it does not affect saved presets (the mesh is not serialisable).
- `three.removeActiveModule` is a no-op (`renderers/three/index.js:177-179`),
  so three resources leak per module instance today. The Rust port must
  release textures, pipelines and bind groups on removal.

The `three` renderer's `createPresetData` / `loadPresetData` /
`getModuleData` hooks (`renderers/three/index.js:181-197`) exist because
three.js state is not serialisable. In Rust these become no-ops as well, and
the preset format keeps the (empty) `data` blob.

**Done when.** `Cube.js` renders identically in the Rust engine, and the
`three` renderer is removed from the Electron renderer.

---

## WS-7 Point10 scenes

**Source.** `src/renderer/src/application/point10/` — the largest single risk
in the plan.

| Component            |    LOC | Notes                                                      |
| -------------------- | -----: | ---------------------------------------------------------- |
| `scenes/`            | 13,587 | 25 scene files                                             |
| `adapter.js`         |    919 | instance lifecycle, presentation, eviction, warm cache     |
| `sceneProps.js`      |    420 | per-scene prop schemas + prop→field binder                 |
| `audioBridge.js`     |    151 | applies responsiveness levers to the band payload          |
| `p10Audio.js`        |    120 | reader over the bridge                                     |
| `warm-cache.js`      |    101 | LRU warm cache                                             |
| `props.js`           |     74 | `RESPONSIVENESS_PROPS`                                     |
| `BaseScene.js`       |     83 | WebGL scene contract                                       |
| `BaseSceneWebGPU.js` |     60 | WebGPU/TSL scene contract, async `init()`                  |
| `timing.js`          |     24 | per-instance frame clock                                   |
| `canvas.js`          |     19 | `OffscreenCanvas` shim                                     |
| `shared/`            |    439 | textures, phrase colour, water wave, particle helpers      |
| `plane9/`            |    851 | `P9Audio.js` (190), `P9Glsl.js` (25 KB), `P9Expression.js` |

### Scene inventory, in port order

| #   | Scene                |  LOC | Backend        | Depends on                                                     |
| --- | -------------------- | ---: | -------------- | -------------------------------------------------------------- |
| 1   | `ChromaticWave`      |   77 | WebGL          | extends `ChromaticLine`                                        |
| 2   | `ChromaticLine`      |  234 | WebGL          | `p10Audio`                                                     |
| 3   | `ParticleField`      |   96 | WebGL          | `shared/particleSprite`                                        |
| 4   | `SpiralParticles`    |  108 | WebGL          | `shared/particleSprite`                                        |
| 5   | `Waveform`           |   85 | WebGL          | —                                                              |
| 6   | `SquareTunnel`       |  193 | WebGL          | `plane9/P9Glsl`, `shared/random`, `shared/textures`            |
| 7   | `LightTunnel`        |  226 | WebGL          | `plane9/P9Glsl`, `shared/random`                               |
| 8   | `Wormhole`           |  292 | WebGL          | `plane9/P9Glsl`, `shared/random`, `Boxy.preserveBloomAlpha`    |
| 9   | `ItIsComingForYou`   |  348 | WebGL          | `plane9/P9Glsl`, `shared/random`                               |
| 10  | `WaterRipple`        |  365 | WebGL          | `p10Audio`, `shared/waterWave`, `shared/phraseColor`           |
| 11  | `GlassLens`          |  418 | WebGL          | `p10Audio`, `shared/textures`, `ChromaticWave`                 |
| 12  | `UltraHDR`           |  444 | WebGL          | `plane9/P9Glsl`, `shared/assets`, `UltraHDRLoader`             |
| 13  | `DvdRoom`            |  447 | WebGPU         | extends `SmokeRoom`                                            |
| 14  | `MirrorCube`         |  489 | WebGL          | `plane9/P9Glsl`, `shared/random`                               |
| 15  | `ColorCube`          |  515 | WebGL          | `plane9/P9Glsl`, `Boxy.preserveBloomAlpha`                     |
| 16  | `Boxy`               |  529 | WebGL          | `plane9/P9Glsl`, `shared/random`, 6 textures                   |
| 17  | `Mycelium`           |  532 | WebGL          | `p10Audio`, `shared/phraseColor`                               |
| 18  | `AroundPlus`         |  535 | WebGL          | `plane9/P9Glsl`, `shared/waterWave`, `shared/textures`         |
| 19  | `SoundCube`          |  549 | WebGL          | `plane9/P9Glsl`                                                |
| 20  | `HollowCube`         |  580 | WebGL          | `plane9/P9Glsl`                                                |
| 21  | `AttractorParticles` |  601 | **WebGPU/TSL** | `shared/phraseColor`, `shared/particleAlpha`                   |
| 22  | `GeometricTunnel`    | 1056 | WebGL          | EffectComposer + **SSAOPass** + Line2/LineMaterial             |
| 23  | `RollingFire`        | 1491 | **WebGPU/TSL** | TSL `curlNoise`, `gaussianBlur`, `bloom`                       |
| 24  | `SmokeRoom`          | 1430 | **WebGPU/TSL** | TSL `curlNoise`, `gaussianBlur`, `bloom`, `RoundedBoxGeometry` |
| 25  | `LatentBackrooms`    | 1947 | WebGL          | EffectComposer + **AfterimagePass** + ShaderPass               |

`RollingFire.js` also exports `BallOfFire`, which is wrapped by
`Point10BallOfFire.js`. 26 wrapper modules exist in
`renderers/2d/modules/`; the wrapper list and the scene list differ by that
one alias.

### Port order rationale

The order above is not by complexity alone — it is by **shared dependency**:

1. **Shared helpers first** (`shared/`): `phraseColor.js` (drop colour pulse,
   equalised hue, hue rotate), `waterWave.js` (the water surface simulation),
   `particleAlpha.js` + `particleSprite.js` (the round particle sprite),
   `random.js` (seeded hash), `textures.js` + `assets.js` (asset loading, with
   the `file://` vs `fetch` split the current code needs because the worker
   has `nodeIntegrationInWorker`).
2. **`plane9/P9Glsl.js`** (25 KB GLSL library) and `plane9/P9Audio.js` — 12
   scenes depend on them, so they are the highest-leverage single port.
3. Then the scenes in the order above.
4. The four TSL/WebGPU scenes (`AttractorParticles`, `RollingFire`,
   `SmokeRoom`, `DvdRoom`) come last because TSL has no Rust analogue: their
   compute passes (curl noise, gaussian blur, bloom) are hand-written WGSL.

### Scene contract (Rust)

```rust
pub trait Point10Scene: Send {
    fn init(&mut self, ctx: &SceneContext) -> anyhow::Result<()>;
    fn update(&mut self, dt: Duration, audio: &AudioBridge);
    fn render(&mut self, ctx: &mut SceneContext);
    fn resize(&mut self, width: u32, height: u32);
    fn dispose(&mut self);
}
```

`SceneContext` owns the scene's own textures, pipelines and bind groups, and
renders into an engine-owned target texture. The current implementation
paints into its own `OffscreenCanvas` and the adapter blits it
(`point10/adapter.js:818-836`); in Rust the scene renders directly into
the group target, which removes the blit entirely and the one-frame lag the
WebGPU path has today (`adapter.js:818-831`).

### Post-processing chain

20 of the 25 scenes use a three.js `EffectComposer` chain:
`RenderPass` → `UnrealBloomPass` → `ShaderPass` → `OutputPass`. The Rust port
needs a shared implementation of:

- `RenderPass` — render the scene into an HDR target.
- `UnrealBloomPass` — bright-pass, mip-chain downsample, gaussian blur,
  upsample-add.
- `ShaderPass` — a generic full-screen pass with a user shader.
- `OutputPass` — tone mapping + sRGB conversion.
- `AfterimagePass` (LatentBackrooms) and `SSAOPass` (GeometricTunnel) — two
  scene-specific passes.

Implement these once in `modv-renderers/point10/postfx/` and reuse them
across scenes.

### Instance lifecycle, eviction and warm cache

From `adapter.js`:

- `MAX_LIVE_INSTANCES = 4`, `MAX_WARM_INSTANCES = 2`, `INSTANCE_IDLE_MS = 3000`.
- `evictInstances` reaps idle instances when over the live cap.
- `evictGalleryInstances` keeps at most one gallery preview alive.
- The warm cache (`warm-cache.js`) is an LRU that holds compiled templates;
  `scenes/prewarmRowScene` warms the **WebGPU** scenes for the next row.
- `removeSceneInstance` defers disposal if a frame transfer is in flight.

In Rust these become straightforward: an instance pool keyed by module id, an
LRU warm cache of `Arc<SceneTemplate>`, and `wgpu` resource lifetimes tied to
the instance. The Chromium context cap (`max-active-webgl-contexts=16`,
`src/main/index.js:69`) disappears entirely — wgpu has one device.

### Audio bridge

`audioBridge.js:67-125` applies the responsiveness levers to the band
payload: per-band gain (`gain`, `bassGain`, `midGain`, `trebleGain`), a
one-pole low-pass over the 12 lanes (`tau = 0.02 + smoothing * 0.6`), a 16-cell
grid scaled by band gain, a beat envelope, the 1024-sample wave and the
scaled `byteFrequencyData`. This is pure DSP and ports directly to Rust.

`p10Audio.js` exposes `band(channel, damping, bandnr)`, `beat()`,
`waveform(channel, pos)` and `spectrum(channel, pos)`; `plane9/P9Audio.js`
exposes the older aliases (`freqArray`, `freqMagL`, `freqMagR`, `dataArray`).
Both are ported as Rust readers over the shared `AudioBridge`.

### Props

`sceneProps.js` holds per-scene prop schemas and the binder that maps props
onto scene fields and uniforms (`SCENE_PROP_CONFIGS`, lines 273-412). The
Rust port reproduces the schemas exactly so saved presets keep working, and
implements the binder as a `match` on the prop key.

**Done when.** All 25 scenes render in the Rust engine with golden-image
parity against the legacy renderer, the warm cache and eviction behave as
specified, and the Point10 prop schema round-trips through a preset.

---

## WS-8 Scenes and performance mode

**Source.** `src/renderer/src/application/worker/store/modules/scenes.js`
(4211 LOC) plus the helpers in `worker/`:

| File                                   |  LOC | Role                                                |
| -------------------------------------- | ---: | --------------------------------------------------- |
| `store/modules/scenes.js`              | 4211 | the state machine                                   |
| `store/default-scenes.js`              | 4243 | versioned default Scene **data**                    |
| `worker/scene-rows.js`                 |  431 | row ↔ group matching by name, row advance/selection |
| `worker/scene-preview.js`              |  398 | hover preview rendering                             |
| `worker/scene-transition.js`           |  315 | whole-scene ISF transitions                         |
| `worker/scene-cell.js`                 |  171 | per-cell Scene helpers                              |
| `worker/scene-timing.js`               |  145 | timing normalisation, per-cell dwell                |
| `worker/scene-capture.js`              |  141 | capture live state to a Scene                       |
| `worker/row-transition.js`             |  115 | row crossfade blending                              |
| `worker/scene-preview-data.js`         |   93 | preview snapshot building                           |
| `worker/scene-columns.js`              |   90 | Scene column layout, empty Scene creation           |
| `worker/scene-link-defaults.js`        |   73 | default link/tween setup                            |
| `worker/scene-disk.js`                 |   41 | Scene file freshness check                          |
| `worker/scene-transition-pick.js`      |   36 | transition selection                                |
| `worker/input-link-value.js`           |   22 | NaN-safe link resolution                            |
| `application/scene-transition-defs.js` |  125 | 6 built-in ISF transitions                          |

**Target.** The scene engine moves into the Rust engine as-is; it is pure
state-machine logic with no DOM dependency.

### Behaviour that must be preserved

- **Per-cell dwell.** Each row picks its own dwell target once when it becomes
  active and then advances independently (`worker/scene-timing.js`
  `pickCellDwell`/`clampCellDwell`). "Max scene time" is a per-cell maximum,
  not a global one. Dwell only advances while no transition is running.
- **Row names.** A named row matches its Group by name (case-insensitive, exact
  or word-prefix, first match wins) instead of by position
  (`worker/scene-rows.js`). A digit-suffixed name (`effects2`) only falls back
  to its base Group (`Effects`) when no sibling row already owns that family
  (`rowFamilyIsShared`).
- **Remapping.** `scenes/setRowNames` remaps the live layer stack
  (`groups/orderByIds`) so named rows snap into place.
- **Gallery independence.** The render loop skips the Gallery group by
  name/identity, not by assuming it is last (`loop.js:630`), so gallery-less
  layouts render every performance row.
- **Transition selection.** While performance mode is running, every cell
  change is a straight crossfade regardless of the allowed-transitions list
  (`worker/scene-transition-pick.js:18-36`), so automatic transitions are
  never sudden "None" switches.
- **Stop performance.** `scenes/stopPerformance` completes any in-flight
  crossfade (transition state plus pending Group cleanup) before disabling
  performance mode, so a stopped performance mode never leaves a stale
  transition or orphaned Groups.
- **Row fade blending.** See WS-2 — the blended canvas is assigned to both
  the incoming and ghost group ids.
- **Transition groups are exempt from the alpha cull** so both halves of a
  fade exist at its start and end.

### Scene documents

A Scene is a preset document plus metadata. The Rust engine must accept the
exact JSON shape (see WS-13). Scene documents also support a legacy shape
(`group` + `layers`) that `normalizeScene` converts.

### Default Scenes

`worker/store/default-scenes.js` is versioned data (`DEFAULTS_VERSION = 9`).
`getScenesToSeed({defaultsVersion, seededNames, existingNames})` seeds only
newly introduced Scenes, matched by name, so existing installs receive later
batches without duplicates. Each Scene has an `introducedVersion`.

Porting this file is a **data** port, not a code port: convert it to JSON
(`crates/modv-engine/data/default-scenes.json`) so both the Rust engine and
any JavaScript tooling can read it. The seeding logic ports to Rust.

### Capture live state to Scene

`worker/scene-capture.js` collects only the applied Scene's Groups
(non-ghost, non-Gallery, stamped with `sceneId`/`sceneRow`), rebuilding
`groups` with `null` row holes, and refuses while performance mode is running.

### Scene preview (hover)

`worker/scene-preview.js` + `scene-preview-data.js` render one cell preview
at ~30 fps from saved Group/Module snapshots, with fresh module ids built
outside the live store. In Rust the preview is a separate small render graph
executed off the live graph, targeting a preview texture that is published to
the UI over the preview channel.

**Done when.** A preset containing every default Scene round-trips through
save/load unchanged; performance mode runs for 10 minutes without leaking
groups; row names, per-cell dwell and transitions behave identically to the
legacy renderer; and the Scene preview renders at ≥ 25 fps.

---

## WS-9 Audio analysis and transport

**Source.**

| File                                          | LOC | Role                                        |
| --------------------------------------------- | --: | ------------------------------------------- |
| `application/audio-bands.js`                  | 324 | AGC + band/cell/lane analysis               |
| `application/tempo-detector.js`               | 475 | onset-envelope / autocorrelation BPM        |
| `application/worker/phrase-detector.js`       |  96 | drop / phrase detection                     |
| `application/setup-beat-detektor.js`          | 133 | onset strength + BeatDetektor               |
| `application/worker/audio-features.js`        |  48 | feature smoothing                           |
| `application/system-audio.js`                 | 343 | monitor-source / PipeWire / display capture |
| `application/system-audio-pipewire.js`        | 250 | PCM bridging into the WebAudio graph        |
| `src/main/system-audio.js`                    | 132 | main-process PipeWire capture               |
| `src/main/system-audio-pipewire.js`           | 180 | `pw-record` spawning                        |
| `src/renderer/src/application/setup-media.js` | 455 | WebAudio graph construction                 |

**Target.** The Rust engine owns audio capture and analysis end to end. The
renderer no longer builds an `AudioContext`, no longer runs Meyda, and no
longer ships a structured-cloned feature bag across a `postMessage` boundary
every frame.

### Band analyser

`audio-bands.js` is the core. Port as-is:

- `BAND_EDGES` — 17 edges: 30, 50, 80, 130, 200, 300, 450, 700, 1000, 1500,
  2000, 3300, 5000, 7500, 11000, 16000, 20000 Hz.
- `BAND_CELLS` — bass = cells 0-4, mid = 5-9, treble = 10-15.
- `FFT_GAIN = 1/0.42`; `DAMPING_TAU = [0.05, 0.12, 0.25]`; `FEATURE_DB_FLOOR = -60`.
- AGC (`_agc`): time-domain peak AGC with a 3-tier linear decay
  (0.0765625 / 0.00765625 / 0.000765625 per second), floor `1e-4`.
- Cell envelope: attack `g + (target-g)*min(1, dt*30)`, release
  `g * exp(-dt*(6+c))`.
- Beat envelope: refractory-gated bass transient, re-arms below 0.2.

Payload shape (`_payload`, lines 161-202) must stay identical because modules
read it:

```json
{
  "bass": 0.0,
  "mid": 0.0,
  "treble": 0.0,
  "bands": [3],
  "lanes": [12],
  "cells": [16],
  "beat": 0.0,
  "wave": [1024],
  "freqMag": [1024],
  "byteFrequencyData": [1024],
  "raw": { "bands": [3], "lanes": [12], "cells": [16] },
  "sampleRate": 48000
}
```

### Tempo, phrase and beat detection

- `tempo-detector.js` is dependency-free (no Meyda) and ports directly.
  `MIN_BPM=70`, `MAX_BPM=200`; options `historySeconds:10`,
  `analysisRate:100`, `analysisInterval:0.2`, `minHistorySeconds:3`,
  `smoothingWindow:5`, `confidenceSmoothing:0.4`, `octaveMidpointRatio:0.35`,
  `confidenceThreshold:0.25`. The octave correction is what stops fast
  (170+ BPM) material being reported as half time — it must be ported
  exactly.
- `phrase-detector.js` — `DROP = {riseFrac:0.4, peakFrac:0.75, floor:0.05,
releaseSec:2, rearmSec:10}`, plus `switchChance(elapsed, min, max)`.
- `setup-beat-detektor.js` — spectral flux over the first 25 % of the complex
  spectrum, plus `lib/BeatDetektor.js` (a vendored JS library) for kick
  detection. `lib/BeatDetektor.js` must be ported to Rust; it is small.

### Meyda feature set

The renderer requests features from `store/modules/meyda.js:13-31`:
`buffer`, `complexSpectrum`, `bass`, `mid`, `treble`, `rms`, `zcr`, `energy`,
`spectralCentroid`, `spectralFlatness`, `spectralSlope`, `spectralRolloff`,
`spectralSpread`, `spectralSkewness`, `spectralKurtosis`,
`perceptualSpread`, `perceptualSharpness`.

Meyda is JavaScript-only, so the Rust engine implements these feature
extractors directly. They are textbook DSP and map cleanly onto `rustfft`:

| Feature               | Implementation                                   |
| --------------------- | ------------------------------------------------ |
| `buffer`              | the raw time-domain window                       |
| `complexSpectrum`     | FFT bins (re/im)                                 |
| `rms`                 | `sqrt(mean(x^2))`                                |
| `zcr`                 | zero-crossing rate                               |
| `energy`              | `sum(x^2)`                                       |
| `spectralCentroid`    | magnitude-weighted mean frequency                |
| `spectralFlatness`    | geometric mean / arithmetic mean of magnitudes   |
| `spectralSlope`       | linear regression of magnitude against frequency |
| `spectralRolloff`     | frequency below which 85 % of energy lies        |
| `spectralSpread`      | weighted standard deviation around the centroid  |
| `spectralSkewness`    | weighted third standardised moment               |
| `spectralKurtosis`    | weighted fourth standardised moment              |
| `perceptualSpread`    | frequency spread over Bark bands                 |
| `perceptualSharpness` | weighted mean of the Bark-band magnitudes        |

`bass`/`mid`/`treble` come from the band analyser, not from these
extractors (`application/index.js:611-616`).

The feature-smoothing machinery (`worker/audio-features.js`,
`getSmoothedFeature`) ports as-is: `MAX_SMOOTHING = 1`, `SMOOTHING_STEP =
0.001`.

### Capture

- **Audio input device**: `cpal` for enumeration and capture.
- **System audio**: a native PipeWire client replaces the current
  `pw-record` subprocess (`src/main/system-audio-pipewire.js`). The current
  transport is 48 kHz / 2 ch / f32 / 480-frame chunks; the Rust client should
  keep the same rate so the band analyser constants stay valid.
- **Screen audio** (macOS/Windows) is out of scope for the Linux-first port
  and stays in Electron until the engine has an equivalent.

### Transport

The current transport structured-clones the entire feature bag to the worker
every frame (`application/index.js:645`) — roughly 2–3 kB plus the 1024-entry
wave and spectrum arrays, every frame. In Rust the analysis happens in the
engine, so there is no transport at all: the features are read directly by
the renderers. The only thing crossing the socket is the band payload for the
UI's meters.

**Done when.** The Rust band analyser produces the same `bass`/`mid`/`treble`
values as the legacy `AudioBands` class within 1 % on a fixed audio fixture,
the tempo detector resolves the same BPM on the same fixture, and the
feature set is complete.

---

## WS-10 Media capture (cameras, screen, video)

**Source.** `src/renderer/src/application/index.js` (960 LOC; camera capture
loops at lines 662-929), `setup-media.js` (455 LOC), `media-inputs.js`
(158 LOC), `video-capabilities.js` (180 LOC), `media-device-manager.js`
(110 LOC), `createWebcodecVideo.js` (41 LOC), plus the worker's frame handlers
(`worker/index.worker.js:386-475`).

**Target.** The Rust engine owns capture and texture upload; Electron owns
the device-management UI.

### Cameras

Today: `getUserMedia` → a hidden `<video>` per device → `ImageCapture` per
device → `grabFrame()` loop → `postMessage` with a transferred `ImageBitmap` →
the worker draws it into a per-device auxiliary output
(`worker/index.worker.js:386-438`).

In Rust: `v4l2` for capture (or GStreamer — see the decision below), a decode
to `NV12`/`RGBA8`, and a direct `wgpu::Texture` upload. The per-device
auxiliary output becomes a `TextureHandle` in the engine's `OutputRegistry`.

**Recommendation: use GStreamer for cameras, screen and file video.** One
dependency covers all three, handles `NV12`/`VP8`/`VP9` hardware paths, and
lets the engine expose the same "device → texture" abstraction. The
alternative (`v4l2` + `ffmpeg-next`) is lower-level and splits the decode
path.

The 1-based camera index semantics must be preserved: the Webcam module's
`camera` prop indexes the saved camera order, falling back to the first camera
(`media-inputs.js:150-158`).

### Screen capture

The Linux path uses `getDisplayMedia` today with a portal-backed picker
(`components/dialogs/ScreenPickerDialog.vue`). In Rust, use
`xdg-desktop-portal` (`ashpd`) `ScreenCast` + PipeWire to obtain a PipeWire
node, then consume it with GStreamer. The picker UI stays in Electron, but
the portal flow must be driven from the engine (or the engine asked for a node
id over the control socket).

### Video files

`createWebcodecVideo.js` uses WebCodecs for `.mp4`/`.webv` files. In Rust,
GStreamer (`decodebin` → `videoconvert` → RGBA) handles both.

### Device settings

`video-capabilities.js` builds the picker's resolution/frame-rate options
from `MediaStreamTrack.getCapabilities()`. `v4l2` exposes the same
information via `VIDIOC_ENUM_FMT`/`VIDIOC_ENUM_FRAMEINTERVALS`, so the
capability query moves into the engine and the UI keeps rendering the same
picker.

### The `mediaStream` store module

`mediaStream` stays in the UI plane: device enumeration and constraint
management are UI concerns; the engine needs only "which device, which
resolution, which frame rate" plus the resulting frames.

**Done when.** A camera, a screen source and a video file all produce frames
in the Rust engine, and the Webcam module's `camera` prop selects the same
device as the legacy renderer.

---

## WS-11 NDI, MIDI, OSC

**Source.**

| Component         | LOC | Location                         |
| ----------------- | --: | -------------------------------- |
| NDI sender worker | 171 | `worker/ndi-worker.worker.js`    |
| NDI store         | 246 | `store/modules/ndi.js`           |
| NDI bridge        |  54 | `application/setup-grandiose.js` |
| MIDI capture      | 165 | `application/setup-midi.js`      |
| MIDI store        |  76 | `store/modules/midi.js`          |
| OSC store         |  80 | `store/modules/osc.js`           |

### NDI

The current sender reads the main canvas with `getImageData` and pushes RGBX
frames to `grandiose` (`worker/ndi-worker.worker.js:7-11`). The receiver path
(`store/modules/ndi.js:48-87`) pulls `receiver.data()` and writes it into an
aux canvas.

In Rust: link the NDI SDK directly (the Docker image already installs libndi
and aliases the soname — see `Dockerfile`), or use an `ndi` bindings crate.
The sender becomes a GPU→CPU readback of the final composited texture (NDI
requires CPU-side RGBA/RGBX), so this path _does_ have a readback — it is not
on the hot path for the display, only for the NDI output, and it is opt-in.

The `grandiose` native addon and its `asarUnpack` entry can be dropped once
the Rust sender works.

### MIDI

MIDI splits across the two planes. The device model stays in Electron; the
clock → BPM path moves into the engine.

| Part                                                     | Plane         | Reason                             |
| -------------------------------------------------------- | ------------- | ---------------------------------- |
| device enumeration, hot-plug, MIDI-learn, CC/note values | UI (Electron) | Web MIDI's device model            |
| MIDI clock (type 248) → BPM                              | Engine        | the engine owns the audio timebase |

#### Why enumeration stays in Electron

`setup-midi.js:141-146` commits `id`, `name` and `manufacturer` for every
input, and re-runs the enumeration on the `statechange` event
(`setup-midi.js:157-159`) so hot-plugged devices appear and disappear. The
`midi` store keys devices as `${id}-${name}-${manufacturer}`
(`store/modules/midi.js`), and the device list, the MIDI-learn flow and the
per-device `channelData` latch state all depend on that identity.

`midir` — the portable Rust equivalent — exposes a port name and a
`MidiInputPortInfo`, but no `manufacturer` field and no hot-plug notification;
you would have to poll `ports()` and synthesise the identity the store and
the UI expect. The MIDI _bytes_ are not the problem — a CC message is three
bytes and the engine already needs a `midi/WRITE_DATA` path, because
mutation-type input links subscribe to it (`worker/index.worker.js:80-121`).
The device model is the problem, and Web MIDI is the only portable source of
it.

#### Why the clock moves into the engine

The clock → BPM path measures inter-clock intervals against
`audioContext.currentTime` (`setup-midi.js:39`). The comment at
`setup-midi.js:9-10` is explicit that this is a **separate** `AudioContext`,
created only to get a clean timebase — modV's own context has timing jitter.

After WS-9 the renderer no longer owns the engine's `AudioContext`, so leaving
the clock in Electron would either strand it or force the renderer to keep a
throwaway `AudioContext` alive purely for timing. Neither is acceptable.

In the engine the clock becomes just another BPM source:

- The engine already owns the tempo detector (`application/tempo-detector.js`,
  ported in WS-9) and the `beats` registry with `bpm`, `bpmSource` and
  `bpmSources[]` (`store/modules/beats.js`).
- The existing derivation is preserved exactly: a 95-sample moving average of
  inter-clock intervals, `bpm = round((1000 / averageDiff / 24) * 60)`, emitted
  at most every 500 ms, with the >50 ms outlier reset
  (`setup-midi.js:38-80`).
- The engine's audio timebase is the same one the band analyser and tempo
  detector use, so a MIDI-clock BPM and an onset-detected BPM are directly
  comparable.

The renderer forwards raw MIDI clock messages (type 248) over the control
socket; it does not derive BPM itself.

#### The IPC path

The renderer forwards, per MIDI message:

```json
{
  "method": "midi/raw",
  "params": {
    "device": "<id>-<name>-<manufacturer>",
    "type": 248,
    "channel": 0,
    "data": 0
  }
}
```

and the engine applies the same `midi/WRITE_DATA` semantics the current store
uses (`store/modules/midi.js`), including the `ccLatch` / `noteOnLatch`
overrides that make a CC behave like a NoteOn
(`setup-midi.js:89-114`). CC values are normalised to `0..1` by dividing by
127 (`setup-midi.js:112-114`) — that stays in the engine.

**Done when** a MIDI CC drives a module prop with the same latency as
today, a MIDI-learn assignment round-trips through a preset, a hot-plugged
device appears in the UI list without a restart, and a MIDI clock produces
the same BPM as the legacy renderer on a fixed clock trace.

### OSC

`node-osc` is a Node-only dependency. `rosc` is the Rust equivalent and is
straightforward. The OSC server is created on `0.0.0.0:3333` at worker setup
(`application/index.js:186-189`); OSC addresses are written into
`state.data` via `lodash.set` (`store/modules/osc.js:69-71`), and module
renders receive `osc.data` (`loop.js:463`).

**Recommendation: move OSC into the engine.** It removes a Node dependency,
keeps latency low, and the address→value mapping is trivial.

**Done when.** NDI out and in work from the Rust engine, an OSC value updates
an input-linked prop with no measurable latency, and the MIDI criteria in the
section above are met.

---

## WS-12 Expressions, tweens and plugins

**Source.** `store/modules/expressions.js` (176 LOC),
`store/modules/tweens.js` (264 LOC), `store/modules/plugins.js` (156 LOC).

### Expressions

Expressions are compiled with `new Function("scope", …)` where
`scope = { value, time, inputValue, osc }`
(`store/modules/expressions.js:25-45`), and are stored in the preset as the
source text plus an `evaluateEveryFrame` flag.

Options:

| Option                                                                                                                           | Effort | Compatibility   | Risk                                               |
| -------------------------------------------------------------------------------------------------------------------------------- | ------ | --------------- | -------------------------------------------------- |
| **(a) Keep in JS** — the renderer evaluates the expression and pushes the resulting value into the engine as an input-link value | low    | exact           | extra IPC per expression per frame                 |
| **(b) `boa_engine`** in Rust — a full JS interpreter                                                                             | high   | near-exact      | ~10 MB binary, slow startup, JS semantics to fight |
| **(c) `rhai`** in Rust — a small embedded scripting language                                                                     | medium | syntax mismatch | users' saved expressions break                     |
| **(d) Custom expression evaluator** in Rust                                                                                      | high   | partial         | a new dialect to document and support              |

**Recommendation: (a) for M2–M6.** The engine already accepts per-frame prop
values through the input-link mechanism, so an expression is just another
value source. The volume is bounded by the number of expressions (typically
< 20). Revisit (b) only if the measured IPC cost becomes visible in the frame
budget.

### Tweens

Tweens pre-render every frame with `anime.js` `seek()`
(`store/modules/tweens.js:31-133`) and expose a current value; they are driven
by BPM (`bpmDivision`, `easeInOutSine`) and can be linked to inputs.

**Recommendation: keep in JS for M2–M6**, same as expressions, using the same
value-push mechanism. Porting the anime.js easing set to Rust is
straightforward (the easing names are a fixed list in
`store/modules/tweens.js`) and is an M7 item if desired.

### Plugins

`store/modules/plugins.js` exposes `preProcessFrame` and `postProcessFrame`
getters, consumed by `loop.js:194-204` and `:699-710`. The only built-in
plugin is `grab-canvas` (`application/plugins/grab-canvas.js`), which copies
the main canvas into a 2D canvas for LED mapping.

JS plugins cannot run inside the Rust engine. Options:

- Define a Rust plugin ABI with a small, fixed set of hooks
  (`pre_frame`, `post_frame`, `on_prop_change`) and re-implement
  `grab-canvas` as a Rust plugin.
- Keep JS plugins in the UI plane, running at UI rate.

**Recommendation: Rust plugin ABI**, with `grab-canvas` reimplemented as the
reference plugin. JS plugins in the media `module/` folder become a known gap
in M7 (documented, not silently broken).

**Done when.** Expressions and tweens drive module props with no measurable
additional latency, and the built-in plugin set works in the Rust engine.

---

## WS-13 Preset, Scene and playlist serialisation

**Source.** `worker/preset-serializer.js` (110 LOC),
`store/modules/common/swap.js` (104 LOC), and the `createPresetData` /
`loadPresetData` actions in `store/modules/{groups,modules,inputs,expressions,tweens,midi}.js`.

**Target.** The Rust engine owns preset load/save. The JSON format is
unchanged — this is a hard compatibility requirement, not a preference.

### Preset document

A preset has exactly six top-level keys (asserted by
`tests/unit/preset-serializer.test.js:60-67`):

| Key           | Type                                 | Rust type                            |
| ------------- | ------------------------------------ | ------------------------------------ |
| `groups`      | array of Group                       | `Vec<Group>`                         |
| `modules`     | object keyed by module `$id`         | `IndexMap<ModuleId, ModuleInstance>` |
| `inputs`      | `{focusedInput, inputs, inputLinks}` | `InputState`                         |
| `expressions` | `{assignments}`                      | `IndexMap<ExpressionId, Expression>` |
| `tweens`      | array of Tween                       | `Vec<Tween>`                         |
| `midi`        | `{devices, learning}`                | `MidiState`                          |

`groups` entries drop `context`, `writeToSwap`, `readFromSwap`, the Gallery
group, and any group with `transitionGhost === true`
(`store/modules/groups.js:264-268`). `modules` entries drop `$status` and any
module with `meta.isGallery` (`store/modules/modules.js:664`, `:672`).

### SWAP semantics

Loading a preset writes into a swap buffer and then commits `SWAP` for
`groups`, `modules`, `inputs` and `expressions`
(`worker/preset-serializer.js:76-78`). Each has a different
`sharedPropertyRestrictions` policy:

| Module        | Preserved across the swap                                                                              |
| ------------- | ------------------------------------------------------------------------------------------------------ |
| `groups`      | the Gallery group (`store/modules/groups.js:100-108`)                                                  |
| `modules`     | `registered: false`, gallery modules kept, `propQueue`, `metaQueue` (`store/modules/modules.js:13-24`) |
| `inputs`      | plain swap                                                                                             |
| `expressions` | plain swap                                                                                             |

After the swaps, the deferred cleanup functions returned by `groups` and
`modules` are invoked (`preset-serializer.js:80-82`).

The Rust port must reproduce this exactly, including the Gallery group
preservation — losing it breaks the Gallery UI.

### Scene documents

A Scene is a preset document plus metadata:

```json
{
  "groups": [...], "modules": {...}, "inputs": {...},
  "expressions": {...}, "tweens": [...], "midi": {...},
  "id": "uuid-v4",
  "name": "Scene 1",
  "enabled": true,
  "minSeconds": 8,
  "maxSeconds": 24,
  "timingOverride": false
}
```

Detection (`preset-serializer.js:88-97`): `id` is a non-empty string, `name`
is a string, and `groups` or `layers` is an array. Scene `groups` is
row-indexed and may contain `null` holes for rows the Scene does not define.

Legacy shape (`group` + `layers`) is normalised by `normalizeScene`.

### Playlists

```json
{
  "type": "modv-scenes-playlist",
  "version": 1,
  "scenes": [ ...Scene documents... ],
  "performance": {
    "minSeconds": 8, "maxSeconds": 24, "transitionSeconds": 2,
    "allowedTransitions": ["fade"],
    "order": "in-order",
    "rowNames": ["Effects", "Foreground"]
  }
}
```

Runtime state (`enabled`, `currentSceneId`) is excluded. Import merges by
`id` (or replaces). A legacy single `transition` string migrates to a
one-item `allowedTransitions` list (`store/modules/scenes.js:1520-1534`).

### On-disk layout

| What                     | Path                                  |
| ------------------------ | ------------------------------------- |
| Per-Scene file           | `<media>/<project>/scene/<name>.json` |
| Authoritative Scene list | `<userData>/scenes.json`              |
| Presets                  | `<media>/<project>/preset/*.json`     |

`<userData>` is `~/.config/VmodV` on Linux (`app.setName("VmodV")`).

`scenes.json` shape:

```json
{
  "version": 1,
  "defaultsSeeded": true,
  "defaultsVersion": 9,
  "seededDefaultScenes": ["Name", ...],
  "scenes": [ ... ],
  "performance": { ... }
}
```

The worker persists it 400 ms after any `scenes/*` mutation
(`worker/index.worker.js:150-168`). The Rust engine takes over this job.

**Done when.** Every preset and Scene in `tests/fixtures` round-trips through
the Rust loader and serialiser with a deep-equal result, the Gallery group
survives a preset load, and a legacy playlist (single `transition` string)
imports correctly.

---

## WS-14 Electron integration, lifecycle and packaging

**Source.** `src/main/**` (2396 LOC), `src/preload/**` (64 LOC),
`src/renderer/src/application/index.js` (960 LOC), `src/renderer/src/application/window-handler.js` (141 LOC).

### Main process

Add an engine supervisor in `src/main/`:

| File                      | Role                                              |
| ------------------------- | ------------------------------------------------- |
| `engine-supervisor.js`    | spawn, heartbeat, restart, snapshot replay        |
| `engine-protocol.js`      | the client side of the control + preview channels |
| `engine-log-forwarder.js` | engine `tracing` output → the shared daily log    |

The supervisor must:

- resolve the engine binary: `process.resourcesPath/modv-engine` in a packaged
  build, `crates/modv-app/target/release/modv-engine` in development;
- spawn it with `--socket <tmp>/modv-engine.sock` and `--state <userData>/engine-state.json`;
- monitor stderr and the heartbeat;
- restart on crash (max 5 restarts in 60 s), replaying the last state snapshot.

**Shutdown.** `src/main/shutdown.js` already coordinates cleanup with a
1 s per-task timeout and a 3 s watchdog. Register the engine shutdown as
another cleanup task: send `engine/shutdown`, wait ≤ 1 s, then SIGKILL. The
engine must flush its snapshot before exiting so the next launch can restore.

### Preload

Expose an `engine` API on `window` (`src/preload/index.js`):

```js
window.engine = {
  commit(type, payload),          // → engine
  dispatch(type, payload),        // → engine, resolves with the action result
  subscribe(cb),                  // → delta stream
  requestPreview(),               // → latest preview frame (ImageBitmap)
  onEngineState(cb),              // started / stopped / restarted
  snapshot(),                     // → current engine state
};
```

### Renderer

`src/renderer/src/application/index.js` currently creates the render worker
(`new ModVWorker()`, line 121) and forwards commits. Replace it with an IPC
client that talks to the engine over the control socket. The public surface
(`window.modV.store.commit`, `window.modV.store.dispatch`) stays identical,
so the 59 Vue components do not change.

The renderer also drops:

- the Web Worker (`src/renderer/src/application/worker/**`, 17.1k LOC) once
  the engine is authoritative;
- the `WebGL`/`WebGPU` renderers and their `import.meta.glob` registration
  (`worker/index.worker.js:172-268`);
- `max-active-webgl-contexts=16` and the Vulkan/WebGPU Chromium switches
  (`src/main/index.js:69`, `:83-91`) — the engine no longer needs them.

The renderer keeps:

- the media manager bridge and the media-manager read/save IPC;
- the audio device UI (`mediaStream` in the UI plane);
- the MIDI device bridge — Web MIDI enumeration, hot-plug and MIDI-learn stay
  here; raw messages (including clock) are forwarded to the engine
  (see [WS-11](#ws-11-ndi-midi-osc));
- the Gallery UI, which now displays Rust-rendered previews.

### Output windows

`src/renderer/src/application/window-handler.js` creates output windows with
`window.open("./output-window.html", "modal", …)` and transfers an
`OffscreenCanvas` into the worker. In the target architecture the engine owns
the output windows natively, so:

- `window.open` for output windows is removed;
- `setWindowOpenHandler` (`src/main/window-prefs.js:173-192`) stops needing the
  `"modal"` frame-name exception;
- the `windows` store module becomes a list the engine populates;
- the `output-window.html` renderer entry is deleted from
  `electron.vite.config.mjs`.

### Packaging

`electron-builder.yml` has **no** `extraResources` section today. Add one:

```yaml
extraResources:
  - from: crates/modv-app/target/release/modv-engine
    to: modv-engine
```

`extraResources` (rather than `files` + `asarUnpack`) is correct because the
binary must stay outside the asar with its executable bit intact.

### Docker / Make

Add a Rust stage to the `Dockerfile`:

```
FROM base AS rust
RUN rustup toolchain install stable && rustup component add rustfmt clippy
RUN apt-get install -y libvulkan-dev libasound2-dev libudev-dev \
                       libpipewire-0.3-dev libxkbcommon-dev libgl1-mesa-dev

FROM rust AS engine-build
COPY crates/ crates/
RUN cargo build --release --manifest-path crates/modv-app/Cargo.toml
```

and copy the binary into the `build` stage before `electron-builder` runs.
Add `.dockerignore` entries for `crates/*/target`.

Makefile targets:

| Target                         | Purpose                                                 |
| ------------------------------ | ------------------------------------------------------- |
| `make engine`                  | `cargo build --release` inside the container            |
| `make engine-test`             | `cargo test`                                            |
| `make engine-lint`             | `cargo clippy -- -D warnings` + `cargo fmt --check`     |
| `make build` / `make appimage` | unchanged; the Docker build now includes the Rust stage |

**Done when.** The packaged AppImage contains `modv-engine`, Electron starts
and stops it, the engine survives a forced kill and is restarted with its
state restored, and quitting the app leaves no orphan engine process.

---

## WS-15 Testing and benchmarking

**Source.** `tests/unit/**` (Vitest), `tests/e2e/**` (Playwright),
`vitest.config.mjs`, `playwright.config.js`.

### Rust unit tests

| Area               | What to test                                                                     |
| ------------------ | -------------------------------------------------------------------------------- |
| State              | group/module/input mutation, prop coercion, SWAP semantics                       |
| Scenes             | timing normalisation, per-cell dwell, row-name matching, transition selection    |
| Presets            | round-trip, legacy shapes, playlist merge                                        |
| ISF                | header parsing, macro expansion, ES1→3 upgrade, prop synthesis, multi-pass graph |
| Audio              | band analyser vs fixture, tempo detection, phrase detection                      |
| MIDI               | clock → BPM against a fixed clock trace; CC/note value normalisation             |
| Expressions/tweens | value resolution, easing parity with anime.js                                    |
| IPC                | framing, request/response, delta ordering, snapshot/restore                      |

### Golden-image tests

Headless `wgpu` rendering against **lavapipe** in CI. Each renderer and a
representative set of scenes get a golden image, compared with a per-pixel
tolerance (suggested: max channel delta ≤ 4/255 and mean absolute error ≤
1.5/255, since the Vulkan and GL rasterisers differ slightly in edge
coverage).

Suites:

| Suite          | Count | Notes                                                              |
| -------------- | ----: | ------------------------------------------------------------------ |
| Shader modules |    37 | all built-in shader modules                                        |
| ISF corpus     |   351 | compile check + a smoke render for a representative subset         |
| 2D modules     |    15 | hand-written 2D modules                                            |
| Point10 scenes |    25 | per scene, at a fixed size and fixed audio fixture                 |
| Compositor     |   ~20 | group order, inherit, pipeline, blend modes, row fade, transitions |

### E2E

The existing Playwright suite drives the Electron UI and the page object
reads the worker state via `self.store` / `self._get`
(`tests/e2e/pageObjectModel/`). After the migration the page object must read
the engine state over the control socket instead. The `is-modv-ready` IPC
becomes "the engine is connected and the first frame has been presented".

`make test-e2e` requires `make build` first, because the page object launches
the built app. The Rust engine binary must therefore be present in the
packaged output — the Docker build handles this.

### Performance harness

A dedicated harness (not the e2e suite) that:

- pins a fixed screen size and uses a fixed audio fixture file;
- runs a fixed scene for N minutes;
- records frame-time percentiles (p50/p90/p99), CPU %, RSS, and presented
  frames per second;
- runs the same scene against the legacy Electron + XWayland build.

Results are stored per release so the targets below can be checked
automatically.

---

## Protocol specification

### Control channel

A Unix domain socket. Electron creates the socket path and passes it to the
engine with `--socket`.

Framing:

```
[ u32 LE payload length ][ u8 kind ][ payload bytes ]
```

| Kind | Meaning           |
| ---- | ----------------- |
| 0    | JSON request      |
| 1    | JSON response     |
| 2    | JSON event        |
| 3    | binary (reserved) |

Request:

```json
{ "id": 42, "method": "scenes/applyScene", "params": { "sceneId": "..." } }
```

Response:

```json
{ "id": 42, "ok": true, "result": null }
```

```json
{
  "id": 42,
  "ok": false,
  "error": { "code": "unknown_scene", "message": "..." }
}
```

Event:

```json
{
  "event": "state/delta",
  "data": { "type": "groups/UPDATE_GROUP", "payload": { "...": "..." } }
}
```

The `state/delta` payload is exactly the `{type, payload}` record shape the
worker's `commitQueue` already uses (`worker/index.worker.js:136-144`), so the
renderer's mirror replay is unchanged.

### Method names

Method names mirror the existing Vuex mutation/action names so the UI's
`store.commit` / `store.dispatch` shims need no renaming:

| Method                                                                                                                                                                 | Maps to                                                                 |
| ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------- |
| `state/snapshot`                                                                                                                                                       | full engine state                                                       |
| `state/commit`                                                                                                                                                         | `{type, payload}` → a Vuex-style mutation                               |
| `state/dispatch`                                                                                                                                                       | `{type, payload}` → a Vuex-style action; resolves with its return value |
| `groups/*`, `modules/*`, `inputs/*`, `scenes/*`, `tweens/*`, `outputs/*`, `images/*`, `videos/*`, `ndi/*`, `osc/*`, `beats/*`, `meyda/*`, `fps/*`, `size/*`, `media/*` | the corresponding store actions                                         |
| `midi/raw`                                                                                                                                                             | a raw MIDI message forwarded from the Web MIDI bridge (WS-11)           |
| `engine/shutdown`                                                                                                                                                      | graceful stop                                                           |
| `engine/restore`                                                                                                                                                       | load a state snapshot                                                   |

`state/dispatch` must reproduce the current behaviour of resolving with the
action's return value, which today is JSON round-tripped through
`registerPromiseWorker` (`worker/index.worker.js:564-582`). Non-serialisable
return values are lost today; the Rust implementation should return a
structured error instead of silently dropping data.

### Preview channel

A second connection to the same socket path, used only for frames.

Header (32 bytes, little-endian):

| Offset | Type  | Field                         |
| -----: | ----- | ----------------------------- |
|      0 | `u32` | magic `VMP1`                  |
|      4 | `u32` | width                         |
|      8 | `u32` | height                        |
|     12 | `u32` | stride                        |
|     16 | `u32` | format (1 = RGBA8, 2 = BGRA8) |
|     20 | `u32` | sequence number               |
|     24 | `u64` | frame id                      |

followed by `height * stride` bytes of pixels.

Transport: the engine creates a `memfd` (or `/dev/shm` file) ring of N slots
(default 3) and publishes each slot's header over the socket before writing
the pixels. The renderer reads the newest published slot. If the renderer
falls behind, the engine overwrites the oldest slot and the renderer skips
frames — previews are never queued.

DMA-BUF is a later optimisation (`linux-dmabuf`); shared memory is the
baseline because it is portable and the preview path is not latency-critical.

---

## Data model (Rust)

The state model is described in [WS-1](#ws-1-engine-state-model). The
additional structures the engine needs:

```rust
pub enum PropValue {
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
    Enum(String),
    Vec2([f32; 2]),
    Vec3([f32; 3]),
    Vec4([f32; 4]),
    Color { r: f32, g: f32, b: f32, a: f32 },
    Texture(TextureHandle),
    Event,
    Osc(String),
}

pub enum BlendMode {
    Normal, Multiply, Overlay, Darken, Lighten, ColorDodge, ColorBurn,
    HardLight, SoftLight, Difference, Exclusion, Hue, Saturation, Color,
    Luminosity,
    Clear, Copy, Destination, SourceOver, DestinationOver, SourceIn,
    DestinationIn, SourceOut, DestinationOut, SourceAtop, DestinationAtop,
    Xor, Lighter,
}

pub enum InheritFrom {
    None,
    Previous,
    Group(GroupId),
}

pub enum GroupEnabled {
    Disabled = 0,
    Drawn = 1,
    DrawnNotToOutput = 2,
}
```

Every one of these has a JSON representation that matches the current preset
format exactly. The `PropValue` enum needs a custom `serde` implementation
that round-trips the current shapes: `vec2`/`vec3`/`vec4` are JSON arrays,
`color` is an object `{r,g,b,a}`, `texture` serialises as its source
descriptor rather than a GPU handle.

---

## Shader translation strategy

Three different shader corpora, three different strategies:

| Corpus                  | Size                      | Strategy                                                                                                                                   |
| ----------------------- | ------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| Built-in shader modules | 37 modules, ~800 LOC GLSL | **Hand-port to WGSL.** Small enough that machine translation costs more than writing it.                                                   |
| ISF corpus              | 351 `.fs` + 36 `.vs`      | **Machine-translate** with an ISF-aware preprocessor, plus per-shader WGSL overrides for the failures. A corpus test tracks the pass rate. |
| Point10                 | 25 scenes, TSL + GLSL     | **Hand-port.** TSL has no Rust analogue; the compute passes (curl noise, gaussian blur, bloom) are written directly in WGSL.               |

For the ISF corpus the pipeline is:

```
.fs source
  → ISF preprocessor (macros, ES1→ES3 upgrade)
  → naga glsl-in  → SPIR-V
  → naga back::wgsl → WGSL
  → (on failure) hand-written WGSL override
```

The preprocessor is the part that must be written carefully: the ISF macro
set is small (10 entries, listed in WS-4) but every one of them is used by
many shaders, and the ES1→ES3 upgrade has to match the JS `ISFUpgrader`
exactly or the results diverge silently.

---

## Migration map

Port in this order (each step independently testable):

1. **M0 – spike:** Rust window + wgpu clear colour + one ISF shader + one
   Point10 scene (Chromatic Line). Measure against Electron + XWayland.
2. **M1 – engine skeleton:** IPC, shared-memory preview, state snapshot/restore,
   Electron starts/stops it, output window on Wayland.
3. **M2 – shader/ISF:** scene graph with one Group, chained rows, ISF parser and
   uniforms, one shader module. Gallery previews still via Rust.
4. **M3 – 2D modules:** canvas facade, text, images, webcam/video texture
   upload, Ball/Bands style modules.
5. **M4 – Point10:** port shared helpers first, then scenes in ascending
   complexity (Chromatic Line, DVD Room, Boxy, Light Tunnel, Wormhole, then
   Smoke Room/Rolling Fire compute scenes). Keep WGSL modules behind the same
   `meta`/`props` contract.
6. **M5 – scenes/performance:** groups, transitions, ghosts, per-cell timing,
   row names, crossfades, scene previews, playlist import/export.
7. **M6 – media:** cameras, screen capture (PipeWire portal), system audio,
   NDI in/out, MIDI clock, OSC, tempo/phrase detection.
8. **M7 – parity and release:** preset compatibility, crash recovery,
   performance hardening, AppImage/deb packaging, docs.

At every step the engine and the legacy renderer consume the same preset JSON.

### Milestone entry and exit criteria

#### M0 — Spike

**Scope.** `crates/modv-app` skeleton; `winit` window; `wgpu` Vulkan surface;
clear colour; one hand-written WGSL ISF shader (a trivial one); one Point10
scene ported (ChromaticLine, 234 LOC); a frame-time harness.

**Exit.**

- A native Wayland window presents frames at 1080p60 on labwc, GNOME and KWin.
- The Vulkan adapter is reported (vendor, driver, backend) and recorded.
- Frame-time percentiles and CPU/RSS are recorded for the spike scene and for
  the equivalent Electron + XWayland scene.
- A written go/no-go recommendation exists **before** the Point10 port starts.

**Risk.** If native Wayland Vulkan fails on the reference compositors, the
project's premise is invalidated and M1 must not start.

#### M1 — Engine skeleton

**Scope.** The control + preview protocol; the engine supervisor; state
snapshot/restore; the shm preview path; a native output window driven by the
engine; engine crash/restart.

**Exit.**

- Electron starts the engine, the engine presents a clear-colour frame in its
  own native window, and the UI shows the engine's preview frame.
- Killing the engine leaves the UI alive; it restarts and restores the last
  snapshot within 5 s.
- The renderer creates no WebGL or WebGPU context (asserted in an e2e test by
  checking that `getContext` was never called with a GPU context type).

#### M2 — Shader / ISF

**Scope.** The render graph with one Group and chained rows; the ISF parser,
macro expander, uniform generator and multi-pass runtime; the GLSL→WGSL
translator; one shader module; the delta stream and the state model.

**Exit.**

- ≥ 95 % of the 351 ISF shaders translate; the rest have overrides.
- A preset loads and saves with a deep-equal round trip.
- The Gallery renders previews through the Rust engine.

#### M3 — 2D modules

**Scope.** The 2D facade, text via `cosmic-text`, image/video/webcam texture
upload, the 15 hand-written 2D modules, the `three`/Cube port.

**Exit.** All 15 2D modules and Cube render with golden-image parity; webcam
and video files feed textures; `fillText` matches the legacy metrics.

#### M4 — Point10

**Scope.** The shared helpers, `plane9`, the post-processing chain, the scene
trait, the instance pool and the warm cache, then the 25 scenes in dependency
order.

**Exit.** All 25 scenes render with golden-image parity; the warm cache and
eviction behave as specified; the Point10 props round-trip through a preset.

#### M5 — Scenes / performance

**Scope.** The scene engine, transitions, ghosts, per-cell timing, row names,
crossfades, scene previews, playlist import/export, the default-scene seeding.

**Exit.** Performance mode runs for 10 minutes with no group leaks; the
transition, dwell and row-name behaviours match the legacy renderer; every
default Scene applies and saves correctly.

#### M6 — Media

**Scope.** Cameras, screen capture, system audio, NDI in/out, the MIDI clock
path, OSC, tempo/phrase detection, the audio feature set.

**Exit.** All capture paths work; the band analyser matches the legacy within
1 % on a fixture; NDI out/in and OSC drive props correctly; a MIDI CC and a
MIDI clock behave as specified in WS-11.

#### M7 — Parity and release

**Scope.** Crash recovery hardening, performance tuning, packaging, docs, the
removal of the legacy renderer (and the `MODV_LEGACY_RENDERER` flag).

**Exit.** The full e2e suite passes against the Rust engine; the performance
targets are met and recorded; the AppImage and deb ship the engine binary and
run on a clean machine.

---

## Performance targets

Baseline: current Electron + XWayland build.

- Native Wayland output: working fullscreen at 1080p60 and 4K, fractional scale.
- p99 frame time at 1080p improved by ≥20% vs baseline for compositor-bound
  scenes; no regression >5% for GPU-bound Point10 scenes.
- CPU per output frame reduced by ≥30%; total app RSS reduced by ≥25%.
- End-to-end latency reduced by ≥1 frame.
- No CPU readback on the hot path; preview path may use DMA-BUF or one shm copy.
- Engine crash does not take the UI down; UI reconnects and restores state.

Measurement method: the performance harness described in
[WS-15](#ws-15-testing-and-benchmarking) runs a fixed set of scenes (one
compositor-bound, one GPU-bound Point10 scene, one ISF scene) at a fixed
resolution for 5 minutes each, on the same machine, with the same audio
fixture, and records p50/p90/p99 frame time, CPU %, RSS and presented FPS.
Both builds are measured in the same session to control for GPU load — the
shared-device GPU contention documented in `AGENTS.md` makes cross-session
comparisons unreliable.

The "no CPU readback" claim is verifiable statically: the engine must not call
`copy_texture_to_buffer` outside the NDI sender path and the preview path.
A CI check greps for it.

---

## Testing

- Rust unit tests for engine state, scene timing, presets, ISF parsing,
  uniforms, audio features.
- Headless wgpu golden-image tests (lavapipe in CI) for each renderer and
  representative scenes.
- Playwright e2e drives the Electron UI; the page object waits on the engine
  via IPC. Engine-specific tests run against a headless build.
- Performance harness: fixed screen, audio fixture, frame-time percentiles, CPU
  and RSS captured per release.
- Keep the existing 800+ Vitest unit suite for the Vue/UI side until parity.

The existing Vitest suite is not thrown away during the migration. The
pure-logic modules it covers (`scene-rows`, `scene-timing`, `scene-capture`,
`scene-preview-data`, `scene-link-defaults`, `input-link-value`,
`default-scenes`, `preset-serializer`, `audio-bands`, `tempo-detector`,
`phrase-detector`, `media-inputs`, `video-capabilities`, `renderer-disposal`)
are the reference implementations for the Rust ports. Each Rust port gets a
Rust test that asserts the same behaviour, and the JavaScript test stays until
the parity milestone for that area is signed off.

---

## Risks

| ID  | Risk                                                                                   | Likelihood | Impact | Mitigation                                                                                                                                                                               |
| --- | -------------------------------------------------------------------------------------- | ---------- | ------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| R1  | ISF GLSL → WGSL is not 1:1; some shaders fail to translate                             | High       | High   | Corpus test with a tracked pass rate; per-shader WGSL overrides; the ISF macro preprocessor is written against the JS runtime's behaviour                                                |
| R2  | Point10 is the largest port (13.6k LOC of scene code, 25 scenes, 4 of them TSL)        | High       | High   | Shared helpers and `plane9` first; scenes in dependency order; golden-image tests per scene; the post-processing chain is implemented once and reused                                    |
| R3  | The Canvas2D facade diverges from Canvas2D semantics                                   | Medium     | Medium | The required API is only 22 operations; implement and test them explicitly; golden-image tests per 2D module; `skia-safe` remains the escape hatch if the facade stalls                  |
| R4  | Text metrics differ (legacy `canvas-text-wrapper` vs `cosmic-text`)                    | High       | Low    | Only `Text.js` uses text; golden-image the text module; expose the legacy wrap/align behaviour explicitly                                                                                |
| R5  | Native Wayland Vulkan fails on some compositor/driver combination                      | Medium     | High   | M0 tests three compositors; X11 fallback retained; the fallback is a shipping feature, not a temporary hack                                                                              |
| R6  | NDI/MIDI/OSC/PipeWire integration is small LOC but high debugging cost                 | Medium     | Medium | Split MIDI across the planes (WS-11): Web MIDI device model in the UI, clock → BPM in the engine; use the existing `libndi` in the Docker image; test PipeWire against a scripted source |
| R7  | Fractional scaling / multi-GPU / VRR differences                                       | Medium     | Medium | Integer-scale fallback; one `wgpu::Device` per adapter set; document the supported matrix                                                                                                |
| R8  | Packaging: the AppImage/deb must ship the Rust binary and its runtime deps             | Low        | High   | `extraResources`; a Docker Rust stage; verify on a clean machine, not just in the build container                                                                                        |
| R9  | The state model diverges from the Vuex model, silently breaking the UI                 | Medium     | High   | Delta stream uses the existing mutation names; a deep-equal mirror test gates every milestone                                                                                            |
| R10 | User-authored modules (media `module/` folder) and JS plugins cannot run in the engine | High       | Medium | Document as a known gap in M7; keep `MODV_LEGACY_RENDERER=1` until then                                                                                                                  |
| R11 | Per-frame IPC volume (expressions, tweens, UI meters) becomes a bottleneck             | Low        | Medium | Measure in M2; the migration trigger for porting expressions/tweens to Rust is a measured regression                                                                                     |
| R12 | The `SWAP` preset mechanism has subtle per-module restrictions                         | Medium     | High   | Port the restrictions verbatim; deep-equal preset round-trip tests including the Gallery group                                                                                           |
| R13 | Engine crash loops instead of surfacing                                                | Low        | High   | Restart budget (5 in 60 s); after that the UI shows a fatal state and offers a legacy-renderer fallback                                                                                  |
| R14 | Scope: the whole plan is larger than the estimates                                     | High       | Medium | Milestones are independently shippable; the legacy renderer stays available until M7                                                                                                     |

Token/time estimates: hybrid renderer MVP ~3–8M model output tokens /
~40–120M total; feature parity ~8–18M output / ~100–250M total. These are
planning ranges, not commitments.

---

## Success criteria for the branch

1. Rust engine opens a native Wayland output window and presents a clear frame.
2. One ISF shader and one Point10 scene render identically to the Electron
   reference.
3. Electron UI starts/stops the engine, sends state, and displays Rust previews
   without creating a WebGL/WebGPU context.
4. Frame-time/CPU/RSS numbers are recorded against the baseline.
5. A documented go/no-go decision follows the spike, before the Point10 port.

---

## Open questions

| #   | Question                                                                                   | Recommendation                                                                                                                                                                                        | Decide by |
| --- | ------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------- |
| 1   | Is `skia-safe` acceptable for GPU 2D, or should the 2D facade target `vello`?              | Neither — implement a small `wgpu` 2D facade. The required API is 22 operations; `vello` is CPU-only and `skia-safe` adds a second GPU backend.                                                       | M3        |
| 2   | Do we ship one engine binary per output window or one engine with multiple native windows? | One engine, multiple native windows. One `wgpu::Device` and queue for all outputs; the current implementation already treats output windows as auxiliary outputs (`worker/store/modules/windows.js`). | M1        |
| 3   | Should previews use DMA-BUF immediately, or start with shared memory?                      | Start with shared memory. The preview path is not latency-critical; add DMA-BUF only if the shm copy shows up in the profile.                                                                         | M1        |
| 4   | Is the long-term UI also moving native, or does Electron remain the shell permanently?     | Out of scope for this plan. The engine/UI split is designed so the UI can be replaced later without touching the engine.                                                                              | —         |
| 5   | GStreamer or `v4l2` + `ffmpeg-next` for capture?                                           | GStreamer for cameras, screen and files — one dependency, one decode path, hardware support.                                                                                                          | M6        |
| 6   | Where does MIDI live — one plane or split across both?                                     | Split it: Web MIDI device enumeration, hot-plug and learn in the UI; MIDI clock → BPM in the engine, which owns the audio timebase (WS-11).                                                           | M6        |
| 7   | Port expressions/tweens to Rust, or keep them in the JS plane?                             | Keep them in the JS plane through M6; the migration trigger is a measured per-frame IPC regression.                                                                                                   | M5        |
| 8   | User-authored JS modules and plugins: keep, port, or drop?                                 | Drop from the parity scope and document; keep `MODV_LEGACY_RENDERER=1` as the fallback. Revisit with a Rust plugin ABI.                                                                               | M7        |

---

## Appendix A — porting inventory

### Rust engine (new code)

| Crate                    | Contents                                                | Rough size              |
| ------------------------ | ------------------------------------------------------- | ----------------------- |
| `modv-ipc`               | protocol, framing, socket client/server, shm ring       | ~1.5k LOC               |
| `modv-engine`            | state model, scenes, presets, inputs, timing            | ~8k LOC                 |
| `modv-render`            | device, render graph, texture pool, compositor, present | ~3k LOC                 |
| `modv-renderers/isf`     | ISF parser, macro preprocessor, runtime, translator     | ~4k LOC + 351 overrides |
| `modv-renderers/shader`  | 37 WGSL shader modules                                  | ~2k LOC                 |
| `modv-renderers/2d`      | canvas facade, text, 15 modules                         | ~4k LOC                 |
| `modv-renderers/three`   | Cube renderer                                           | ~0.5k LOC               |
| `modv-renderers/point10` | shared helpers, plane9, postfx, 25 scenes               | ~12–18k LOC WGSL + Rust |
| `modv-media`             | capture, decode, NDI, audio analysis                    | ~5k LOC                 |
| `modv-app`               | winit windows, supervisor loop                          | ~1k LOC                 |

### JavaScript removed from the renderer

| Area                            |    LOC | Notes                                         |
| ------------------------------- | -----: | --------------------------------------------- |
| `application/worker/**`         | 17,100 | the worker, the store and the scene machinery |
| `application/renderers/**`      | 53,261 | 49,060 of which is the ISF corpus             |
| `application/point10/**`        | 16,848 | the vendored scenes                           |
| `application/setup-media.js`    |    455 | WebAudio graph                                |
| `application/tempo-detector.js` |    475 | BPM                                           |
| `application/system-audio*.js`  |    593 | system audio                                  |
| `application/audio-bands.js`    |    324 | band analysis                                 |

`application/index.js` (960 LOC) is **rewritten, not deleted**: the worker
creation, audio pump and camera-capture halves move to the engine, while the
`modV` singleton, the engine IPC client and the MIDI device bridge stay.

### JavaScript retained in Electron

| Area                              |           LOC | Notes                                   |
| --------------------------------- | ------------: | --------------------------------------- |
| `src/main/**`                     |         2,396 | + the engine supervisor                 |
| `src/preload/**`                  |            64 | + the engine API                        |
| `src/renderer/src/components/**`  | 59 components | unchanged except for preview rendering  |
| `src/renderer/src/ui-store/**`    |           389 | the UI-only store                       |
| `application/setup-midi.js`       |           165 | Web MIDI device model, hot-plug, learn  |
| `src/renderer/src/application/**` |        ~4,110 | minus the worker, renderers and point10 |

### Corpus sizes used for planning

| Corpus                        |                Count |     LOC |
| ----------------------------- | -------------------: | ------: |
| ISF shaders                   | 351 `.fs` + 36 `.vs` | ~49,000 |
| Point10 scenes                |                   25 |  13,587 |
| Shader modules                |                   37 |   1,048 |
| 2D modules (hand-written)     |                   15 |   1,892 |
| 2D modules (Point10 wrappers) |                   26 |     191 |
| three modules                 |                    1 |     119 |
| Store modules                 |                   27 |  12,800 |
| Vue components                |                   59 |       — |

---

## Appendix B — parity matrix

| Area                 | Legacy source                           | Rust target                           | Parity test    |
| -------------------- | --------------------------------------- | ------------------------------------- | -------------- |
| Group order          | `store/modules/groups.js:111-123`       | `engine::state::render_order`         | unit           |
| Group inherit        | `worker/group-inherit.js:10-28`         | `engine::render::resolve_inherit`     | unit + golden  |
| Pipeline mode        | `loop.js:391-402`                       | `engine::render::FrameGraph`          | golden         |
| Alpha cull           | `loop.js:257-266`                       | `engine::render::should_draw`         | unit           |
| Row fade             | `worker/row-transition.js`              | `engine::render::RowFade`             | golden         |
| Scene transition     | `worker/scene-transition.js`            | `engine::render::SceneTransition`     | golden         |
| Blend modes (30)     | `util/composite-operations.js`          | `engine::render::BlendMode`           | golden         |
| Prop coercion        | `store/modules/modules.js:481-562`      | `engine::props::coerce`               | unit           |
| Input links          | `store/modules/inputs.js`               | `engine::inputs`                      | unit           |
| NaN-safe links       | `worker/input-link-value.js:9-22`       | `engine::inputs::resolve`             | unit           |
| Per-cell dwell       | `worker/scene-timing.js`                | `engine::scenes::timing`              | unit           |
| Row-name matching    | `worker/scene-rows.js`                  | `engine::scenes::rows`                | unit           |
| Transition pick      | `worker/scene-transition-pick.js`       | `engine::scenes::pick`                | unit           |
| Default seeding      | `worker/store/default-scenes.js`        | `engine::scenes::seed`                | unit           |
| Preset round trip    | `worker/preset-serializer.js`           | `engine::preset`                      | unit + fixture |
| SWAP restrictions    | `store/modules/common/swap.js`          | `engine::preset::swap`                | unit           |
| ISF parsing          | `interactive-shader-format` (npm)       | `modv-renderers/isf::parser`          | unit           |
| ISF props            | `renderers/isf/index.js:181-281`        | `modv-renderers/isf::props`           | unit           |
| Shader uniforms      | `renderers/shader/index.js:90-126`      | `modv-renderers/shader::uniforms`     | unit           |
| 2D facade            | Canvas2D                                | `modv-renderers/2d::Canvas`           | unit + golden  |
| Point10 props        | `point10/sceneProps.js`                 | `modv-renderers/point10::props`       | unit           |
| Point10 audio bridge | `point10/audioBridge.js`                | `modv-renderers/point10::AudioBridge` | unit           |
| Band analyser        | `application/audio-bands.js`            | `modv-media::audio::bands`            | unit + fixture |
| Tempo detector       | `application/tempo-detector.js`         | `modv-media::audio::tempo`            | unit + fixture |
| Phrase detector      | `application/worker/phrase-detector.js` | `modv-media::audio::phrase`           | unit           |
| Camera selection     | `application/media-inputs.js`           | `modv-media::camera`                  | unit           |
| Video capabilities   | `application/video-capabilities.js`     | `modv-media::camera::caps`            | unit           |
| NDI sender           | `worker/ndi-worker.worker.js`           | `modv-media::ndi::Sender`             | manual         |
| OSC server           | `store/modules/osc.js`                  | `modv-media::osc`                     | manual         |
| MIDI clock → BPM     | `application/setup-midi.js:38-80`       | `modv-media::audio::midi_clock`       | unit + fixture |
| MIDI device identity | `application/setup-midi.js:134-146`     | stays in the UI (Web MIDI)            | manual         |

---

## Appendix C — glossary

| Term              | Meaning                                                                                      |
| ----------------- | -------------------------------------------------------------------------------------------- |
| Engine plane      | Rust-owned state: groups, modules, scenes, inputs, render targets                            |
| UI plane          | JavaScript-owned state: the Vue UI store and the read-only engine mirror                     |
| Group             | A layer: an ordered list of modules composited onto the output                               |
| Module            | An instance of a registered module type, with props, meta and data                           |
| Row               | A performance-mode layer; rows map to Groups by name or position                             |
| Ghost             | The outgoing Group kept alive during a row crossfade                                         |
| Row fade          | The blend of the outgoing ghost and the incoming Group                                       |
| Scene transition  | A whole-scene ISF transition between two group sets                                          |
| ISF               | Interactive Shader Format — the `.fs` shader format with a JSON header                       |
| ISF pass          | One render target in an ISF multi-pass shader                                                |
| PERSISTENT buffer | An ISF persistent float render target (`RGBA32F`)                                            |
| Point10           | The vendored scene collection ported from musicvis                                           |
| TSL               | three.js Shading Language — the node-based shader language used by the WebGPU Point10 scenes |
| Field callback    | The `wl_surface.frame` Wayland callback used for frame pacing                                |
| SWAP              | The preset-loading mechanism that swaps live state with a swap buffer                        |
| Legacy renderer   | The current Electron worker renderer, kept behind `MODV_LEGACY_RENDERER=1` until M7          |
| MIDI split        | Web MIDI device model in the UI; MIDI clock → BPM in the engine (WS-11)                      |
