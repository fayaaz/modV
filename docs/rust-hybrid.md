# Rust hybrid engine plan

Status: proposal / spike branch
Branch: `rust-hybrid`

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
│  - NO WebGL / NO WebGPU      │         │  - NDI in/out, MIDI, OSC         │
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

Suggested crates: `wgpu`, `winit`, `naga`, `raw-window-handle`, `bytemuck`,
`serde`/`serde_json`, `tokio` (IPC/media async), `cpal`, `rustfft` or
`realfft`, `midir`, `rosc`, `pipewire`/`ashpd` (portal), `v4l2` or
`gstreamer`, `ndi` bindings / direct NDI SDK FFI, `cosmic-text` + `fontdb` for
text, `skia-safe` or a canvas-2D facade for 2D modules, `tracing` for logs.

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

### Rendering pipeline in Rust

1. `winit` creates a native Wayland (or X11 fallback) window per output.
2. `wgpu` creates a Vulkan surface/device/queue and swapchain.
3. A render graph evaluates Groups in row order, each pass reading the previous
   row's texture (`lastCanvas` equivalent) and writing its own target.
4. Modules are GPU passes:
   - ISF: parse ISF JSON, upload uniforms/textures, run fragment pass.
   - shader: GLSL/TSL-equivalent WGSL pipelines with prop uniforms.
   - 2D: canvas-like draw API implemented over `skia-safe` (GPU) or a small
     wgpu 2D path, with text via `cosmic-text`.
   - Point10: WGSL compute/render ports of the TSL scenes.
   - three: either WGSL ports or a Rust scene-graph renderer; last priority.
5. Transitions and performance mode: same row/group machinery, blend the
   outgoing ghost with the incoming Group in a scratch texture.
6. Present: write the final texture to the swapchain; pace with Wayland frame
   callbacks / `wp_presentation` where available.

No CPU readbacks on the hot path. Reuse textures via a pool. Cache pipelines
and bind groups. Keep a persistent `wgpu::Device` for the engine lifetime.

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

## Migration map

Port in this order (each step independently testable):

1. **M0 – spike:** Rust window + wgpu clear colour + one ISF shader + one
   Point10 scene (Smoke Room). Measure against Electron + XWayland.
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
   NDI in/out, MIDI, OSC, tempo/phrase detection.
8. **M7 – parity and release:** preset compatibility, crash recovery,
   performance hardening, AppImage/deb packaging, docs.

At every step the engine and the legacy renderer consume the same preset JSON.

## Performance targets

Baseline: current Electron + XWayland build.

- Native Wayland output: working fullscreen at 1080p60 and 4K, fractional scale.
- p99 frame time at 1080p improved by ≥20% vs baseline for compositor-bound
  scenes; no regression >5% for GPU-bound Point10 scenes.
- CPU per output frame reduced by ≥30%; total app RSS reduced by ≥25%.
- End-to-end latency reduced by ≥1 frame.
- No CPU readback on the hot path; preview path may use DMA-BUF or one shm copy.
- Engine crash does not take the UI down; UI reconnects and restores state.

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

## Risks

- ISF GLSL → WGSL is not 1:1; some shaders may need a GL/GLES backend or manual
  rewrites.
- Point10 TSL → WGSL is the largest and highest-risk port (~15k LOC, 25 scenes).
- Canvas2D module API needs a compatible Rust facade; text metrics and blending
  are the likely pain points.
- NDI/MIDI/OSC/PipeWire integration can be small LOC but large debugging cost.
- Wayland compositor/driver differences (fractional scaling, multi-GPU, VRR).
- Packaging: AppImage/deb must ship the Rust binary and its runtime deps; the
  Docker toolchain needs a Rust stage.
- Token/time estimates: hybrid renderer MVP ~3–8M model output tokens /
  ~40–120M total; feature parity ~8–18M output / ~100–250M total. These are
  planning ranges, not commitments.

## Success criteria for the branch

1. Rust engine opens a native Wayland output window and presents a clear frame.
2. One ISF shader and one Point10 scene render identically to the Electron
   reference.
3. Electron UI starts/stops the engine, sends state, and displays Rust previews
   without creating a WebGL/WebGPU context.
4. Frame-time/CPU/RSS numbers are recorded against the baseline.
5. A documented go/no-go decision follows the spike, before the Point10 port.

## Open questions

- Is `skia-safe` acceptable for GPU 2D, or should the 2D facade target `vello`?
- Do we ship one engine binary per output window or one engine with multiple
  native windows?
- Should previews use DMA-BUF immediately, or start with shared memory?
- Is the long-term UI also moving native, or does Electron remain the shell
  permanently?
