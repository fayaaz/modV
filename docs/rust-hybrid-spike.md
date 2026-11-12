# Rust hybrid engine — M0/M1 spike report

Status: spike complete; go/no-go recommendation at the bottom.

Scope of this document: validate that a native `winit` + `wgpu` engine can
present frames through a real GPU surface on this machine, that the engine can
be packaged and supervised by Electron (WS-14), and to measure the delta
against the Electron + XWayland baseline. It deliberately does **not** attempt
the Point10 scene port — that is M4 and gated on this report.

Machine: AMD Radeon Graphics (RADV GFX1201, Vulkan, Mesa 25.0.7), Wayland
session (ozone wayland), host display ~120 Hz, 16 cores. Everything inside
`make engine*` runs in Docker; the host ran the legacy AppImage for the
baseline.

---

## What was built

M0 (spike):

- `crates/modv-render` — wgpu device/queue, Fifo surface/present (native
  Wayland Vulkan), headless offscreen target + RGBA8 readback, frame stats
  (p50/p90/p99, CPU % from `/proc/self/stat`, RSS via `VmHWM`).
- `crates/modv-renderers` — Point10 **ChromaticLine** ported exactly (ring
  buffer, per-texel 23 ms mean-|v| windows, `_audioUniforms` band curve,
  WGSL translation of the GLSL with the `gl_FragCoord` y-flip; 5 unit tests).
- `crates/modv-app` — `modv-engine` binary: winit 0.30 `ApplicationHandler`,
  CLI (`--frames/--headless/--bench/--bench-seconds/--screenshot/
  --audio-fixture/--print-adapter/--socket/--state`), screenshot writer.

M1 (engine skeleton):

- `crates/modv-ipc` — the WS-14 control protocol: `[u32 LE len][u8 kind]
  [payload]` framing, JSON Request/Response/Event, non-blocking Unix socket
  server, the `VMP1` shared-memory preview ring (3 slots, seqlock read-back).
  32 unit tests.
- `crates/modv-app` — the engine-side supervisor: binds `--socket`, polls it
  in the frame loop, `state/snapshot` round-trip, `state/commit` /
  `state/dispatch` accepted-and-logged, `engine/shutdown` (flush + exit),
  1 Hz `engine/heartbeat`, preview publishes at ≤30 fps (flat clear colour
  until WS-2/M2 — the ring/transport is the final shape), snapshot every
  120 frames + on exit, restored at startup. Native Wayland window verified
  presenting at 120 fps. 4 Rust tests.
- Electron (`MODV_ENGINE=1`, legacy stays the default): `engine-supervisor.js`
  (spawn, connect retry, heartbeat watchdog SIGTERM→SIGKILL, restart budget
  5/60 s, fatal state), `engine-protocol.js`, `engine-ipc.js`,
  `engine-log-forwarder.js`; `window.engine` preload surface; renderer
  engine client replaying `state/delta` into the mirror store; shutdown path
  sends `engine/shutdown`; supervisor restarts on crash and the engine
  restores its snapshot. Unit tests spawn a real fake engine (6 tests);
  e2e spec `tests/e2e/spec/engine/engineSkeleton.spec.js` is green against the
  packaged AppImage (preview frames publish; engine kill is survived by the
  UI; supervisor restarts it; renderer creates zero WebGL/WebGPU contexts).

## Measurements

Same machine, same session (sibling processes compete for the GPU), RADV
GFX1201, 1280×720, 110 Hz sine fixture breathing through the scene:

| Build                        | Mode                          | fps     | p50/p90/p99       | CPU   | RSS     |
| ---------------------------- | ----------------------------- | ------- | ----------------- | ----- | ------- |
| Rust engine (`modv-engine`)  | windowed Wayland (Fifo vsync) | 120.8   | 8.3 / 8.6 / 16.5  | 3.4 % | 103 MB  |
| Rust engine (`modv-engine`)  | headless offscreen            | 6137    | 0.15 / 0.16 / 0.43 | 102 % | 103 MB  |
| Rust engine (lavapipe, CPU)  | headless offscreen            | 986     | 0.9 / 1.3 / 1.7   | 1097% | 129 MB  |
| Electron + XWayland baseline | windowed, output 1280×720     | 60 (vsync) | n/a*            | 16 %  | ~1.47 GB tree |

\* The legacy build has no user-visible per-frame timer; fps is the
`metrics/SET_FPS_MEASURE` store value (vsync-limited). CPU/RSS summed over
all 8 `vmodv` processes (browser, GPU, 2 renderers, utilities) while the
Point10 Chromatic Line module was loaded and the output sized 1280×720.

Read on the numbers:

- The Rust engine at vsync ties the legacy build at frame rate and beats it
  by ~5× CPU and ~14× RSS. The gap is the architecture, not the scene:
  Chromium's process tree, V8, and the worker/OffscreenCanvas compositing
  path are all gone from the frame loop.
- The legacy number is dominated by Chromium; comparisons at any resolution
  favour the engine by roughly the same factors.
- These are one-session, one-GPU samples; treat absolute values as ±10 % and
  the *ratios* as the signal. The performance harness (WS-15) remains the
  authority for release-to-release comparisons.

## Wayland

Native Wayland Vulkan works out of the box on this compositor (labwc/GNOME/
KWin matrix **not** run — see "Not run" below). XWayland fallback verified in
the e2e container: the engine panics at startup with
`Library libxkbcommon-x11.so could not be loaded` unless the X11 library is
present; `libxkbcommon-x11-0` was added to both the `toolchain` and `rust`
Docker stages for this reason. Packaging note for M7: the AppImage must not
rely on libxkbcommon-x11-0 being present (it dlopens); the X11 fallback
should degrade gracefully with a clear log line rather than a panic. (This is
a two-line change in `modv-app` plus a doc line in M7 — tracked there.)

## Risks retired vs still open

Retired by the spike:

- wgpu/winit/Vulkan path on this GPU: presentation, pacing (Fifo), headless
  readback, packaging (engine inside the AppImage via `extraResources`),
  Electron supervision/restart/restore, renderer-side IPC with zero GPU
  contexts in the UI.
- The scene contract (`init/update/render/dispose` shape) and a GLSL→WGSL
  translation that compiles and matches semantics (y-flip convention).
- The wire protocol (framing/messages/socket) and the preview transport.

Still open (owned by later milestones):

- ISF GLSL→WGSL corpus translation (WS-4) — the highest-risk item in the
  plan; nothing here de-risks it directly.
- Point10 TSL scenes (`RollingFire`, `SmokeRoom`, `AttractorParticles`,
  `DvdRoom`) need compute passes written in WGSL (WS-7).
- The Canvas2D facade + text shaping (WS-5, cosmic-text).
- Wayland compositor matrix (labwc/GNOME/KWin); fractional scale; present
  mode tuning.
- The X11 `libxkbcommon-x11-0` packaging caveat above.
- Performance harness (WS-15) with a fixed fixture and per-release results.

## Recommendation: GO

Proceed to M2 (shader/ISF) and then M3/M4 per the plan, with these gating
conditions:

1. **ISF translation is the project.** WS-4's corpus test must track pass rate
   as a committed artifact; if it plateaus below ~95 % clean translations,
   revisit scope for the ISF corpus before writing scene code.
2. **Keep the native Wayland result honest**: run the M0 matrix on labwc and
   GNOME before M4; if native Vulkan fails on either, the X11 fallback is
   already proven and acceptable, but it is a shipping feature with the
   xkbcommon caveat documented above.
3. **Do not start M4** until M2's deep-equal preset round-trip test is green
   — the state model must be trustworthy before scenes ride on top of it.
4. **Packaging**: keep the engine as `extraResources`; verify the AppImage
   launches with no FUSE on a clean machine in M7.
5. **The legacy renderer stays** behind `MODV_LEGACY_RENDERER=1` until M7;
   `MODV_ENGINE=1` remains the opt-in path through M2–M6.

None of these are blockers discovered by the spike — every path the plan
depends on compiles, presents, packages, and survives a kill. The remaining
risk is the ISF corpus, which the plan already schedules honestly.

## Not run / explicitly out of scope for M0

- Labwc/GNOME/KWin compositor matrix; fractional scale; multi-monitor.
- Shader modules render parity; ISF corpus (WS-3/WS-4).
- Point10 scenes beyond ChromaticLine (WS-7).
- Audio fixtures vs the legacy band analyser on real music (WS-9).
- Performance harness per the WS-15 method (this table is indicative only).
- Wayland `wp_presentation`/`VK_GOOGLE_display_timing` latency stats.
