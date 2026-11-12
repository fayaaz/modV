# Testing the Rust hybrid engine

The engine is opt-in (`MODV_ENGINE=1`); the default app is unchanged. Everything
runs in Docker, so the host only needs Docker + make.

## 1. Quick check (~3 min)

```bash
make engine-test                      # 911 Rust tests (lavapipe GPU tests run/skip)
make test-unit                        # Vitest incl. the engine mirror-parity replay
make engine-lint                      # clippy -D warnings + rustfmt
make lint                             # ESLint/Prettier for the Electron side
```

Single crate / single test:

```bash
make engine-test ARGS="-p modv-app"
make engine-test ARGS="-p modv-renderers point10"
make test-unit ARGS="tests/unit/engine-mirror.test.js"
```

GPU tests use lavapipe inside the container. They print a skip line and pass
when no adapter is available; the compositor/scene pixel tests only assert
real pixels when a device exists.

## 2. Full verification (~10 min + build)

The e2e page object launches the **packaged** app, so build first:

```bash
make appimage                         # Docker: engine-build stage + electron-builder
make test-e2e                         # 143 specs (legacy renderer)
make test-e2e ARGS="tests/e2e/spec/engine/engineSkeleton.spec.js"
```

The engine spec launches its own instance with `MODV_ENGINE=1` and asserts:
preview frames publish, killing the engine leaves the UI alive, the supervisor
restarts it, and the renderer created **no** WebGL/WebGPU context.

Notes:

- Build targets refuse to start under 20 GB free. `make disk-clean` prunes
  BuildKit cache (never the node_modules/cargo volumes).
- Known flakes from `AGENTS.md` still apply (shared-GPU contention; re-run the
  spec on its own).

## 3. Manual testing

### Dev loop with the engine

```bash
make dev ENGINE_MODE=1
```

HMR Vue UI + the Rust engine presenting in its own native **Wayland** window.
The engine binary is rebuilt from `crates/` with `make engine` — or run
`make engine` first if you changed Rust code (the supervisor spawns the built
binary, not a watcher).

### Packaged app

```bash
make appimage
rm -rf /tmp/vmodv-engine-test
XDG_CONFIG_HOME=/tmp/vmodv-engine-test MODV_ENGINE=1 \
  nohup ./dist/VmodV-1.0.0.AppImage --appimage-extract-and-run \
  --remote-debugging-port=9223 --no-sandbox > tmp/app-run.log 2>&1 &
```

`XDG_CONFIG_HOME` keeps the run isolated from your real `~/.config/VmodV`.
Add `ELECTRON_ENABLE_LOGGING=1` to stream both processes' console output to
`tmp/app-run.log`.

### Standalone engine (no Electron)

```bash
make engine-run ARGS="--frames 600 --bench"     # native window + frame report
make engine-run ARGS="--screenshot tmp/engine.png"
make engine-run ARGS="--headless --frames 900 --bench"
make engine-run ARGS="--print-adapter"
```

Without an applied Scene/Group the standalone engine renders an empty
(transparent/black) frame; use `--state <path>` to load a saved snapshot with
groups, or drive it over the control socket.

## 4. Inspecting the running app (CDP)

With `--remote-debugging-port=9223` (the `browser-harness` skill or plain CDP):

```js
// The renderer's mirror of the engine state (same Vuex store the UI reads)
window.modV.store.state.scenes.scenes.length        // 59 after seeding
window.modV.store.state.groups.groups              // live layer stack
window.modV.store.state.modules.active             // live module instances

// Apply a default Scene (renders in the native engine window)
const s = window.modV.store.state.scenes.scenes.find((s) => s.name === "Lone Chrome");
await window.modV.store.dispatch("scenes/applyScene", { sceneId: s.id });

// Performance mode: rows advance on their own dwell timers with crossfades
await window.modV.store.dispatch("scenes/setPerformanceEnabled", { enabled: true });
// ... later ...
await window.modV.store.dispatch("scenes/setPerformanceEnabled", { enabled: false });

// Ask the engine for the latest composited preview frame (VMP1 ring)
const { frame, pixels } = await window.engine.requestPreview();
// frame = { slot, width, height, stride, format, sequence, frameId }
// pixels = RGBA Buffer at 320x180
```

Engine state markers:

- Mirror deltas: `window.modV.store.subscribe` / the `window.engine.subscribe`
  API (each `{type, payload}` is exactly a Vuex mutation).
- The engine writes `engine-state.json` and `scenes.json` next to `<userData>`
  (`~/.config/VmodV` in the real app, under `XDG_CONFIG_HOME` when overridden).
- Logs: `~/.config/VmodV/logs/vmodv-YYYY-MM-DD.log`; engine (`tracing`-style)
  lines are forwarded through the supervisor with an `[engine]` scope.

## 5. Expected behaviour checklist

- `MODV_ENGINE=1` startup: UI appears, a **separate native engine window** opens
  and presents frames; the renderer creates no GPU context (the e2e spec
  asserts this).
- Scenes tab lists the seeded defaults (59) even though the engine seeds them
  before the UI connects (`state/sync`).
- Applying a Scene rebuilds the layer stack and the engine window shows it;
  scene documents referencing not-yet-ported Point10 modules log
  `could not link …`/module `$status` warnings and render only the ported parts
  (5 of 25 scenes are ported).
- Performance mode: rows advance independently, crossfades run, and stopping
  leaves no `transitionGhost` groups.
- Kill the engine process (`for p in $(pgrep -f 'modv-engine.*--socket'); do
  kill $p; done`): the UI stays alive, the supervisor restarts it from the
  snapshot, preview frames resume.
- Quit the app normally (or close the window) and confirm no engine is left:
  `pgrep -af modv-engine` should be empty. `PR_SET_PDEATHSIG` also covers an
  abruptly SIGKILLed Electron.

## 6. What the suites cover

| Area | Command |
| --- | --- |
| State model, props, SWAP, presets, scenes core, default data, playlists | `make engine-test ARGS="-p modv-engine"` |
| Render graph, blending, row fade, texture pool | `make engine-test ARGS="-p modv-render"` |
| Shader modules, ISF translation/runtime, 2D facade + modules, Point10 batch | `make engine-test ARGS="-p modv-renderers"` |
| Audio analysis, MIDI/OSC/NDI | `make engine-test ARGS="-p modv-media"` |
| Engine store/compositor/performance/scene apply, fixtures | `make engine-test ARGS="-p modv-app"` |
| Protocol framing, preview ring, sockets | `make engine-test ARGS="-p modv-ipc"` |
| Rust delta stream ↔ JS mirror parity | `make test-unit ARGS="tests/unit/engine-mirror.test.js"` |
| Packaged app + engine lifecycle | `make test-e2e ARGS="tests/e2e/spec/engine/engineSkeleton.spec.js"` |
