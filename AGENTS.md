# AGENTS.md

VmodV (fork of modV) — modular audio visualisation app. Electron + Vue 3 + Vuex, built with **electron-vite** (Vite) and **electron-builder**.

## Commands

All yarn work is Dockerised. The host only needs Docker (with Buildx) and make — there is no host Node or Yarn.

- `make docker-deps` — build the `modv-toolchain:local` image from the Dockerfile `toolchain` stage (Node 24 + dependencies + Electron runtime).
- `make install` — rebuild the toolchain image and reseed `node_modules` (clean `yarn install`; applies `patches/` and fetches the Electron binary).
- `make dev` — Dockerised `yarn dev` with HMR. On Linux, a Wayland socket is forwarded when available (falling back to X11), along with PulseAudio and `/dev/dri` (GPU); override with `DISPLAY_BACKEND=x11` or `DISPLAY_BACKEND=wayland`. Extend with `DOCKER_RUN_ARGS="..."`. For CDP attach: `make dev ARGS="-- --remote-debugging-port=9223"`.
- `make build` — **the canonical Linux build** (see `Dockerfile`); empties `dist/` in place and writes the AppImage and `.deb` into it. Also used by CI. Set `APP_VERSION` to override the version, e.g. `APP_VERSION=1.2.3 make build`; `make clean` removes `dist/`. Do not use a local `electron-builder` build.
- `make appimage` — AppImage-only local build (same layer cache, skips the slow `.deb` packaging); writes just the AppImage into `dist/`. Use `make build` when you need the `.deb` or release artifacts.
- `make yarn ARGS="build"` — bundle main / preload / renderer into `out/`.
- `make lint` — Dockerised `yarn lint` (`eslint . --fix`, Prettier runs through ESLint).
- `make format` — Dockerised `yarn format`.
- `make test-unit` — Dockerised Vitest unit tests (`tests/unit/**/*.test.js`, Node environment). Single file: `make test-unit ARGS="tests/unit/<name>.test.js"`. Watch mode: `make yarn ARGS="test:unit:watch"`.
- `make test-e2e` — Dockerised Playwright. Electron is launched with `--no-sandbox` and the Wayland/X11, PulseAudio and GPU forwarding above. **Requires `make build` first**: the page object launches the built app from `./dist` via `findLatestBuild`, falling back to extracting the newest AppImage. Single spec: `make test-e2e ARGS="tests/e2e/spec/<name>.spec.js"`. WebGPU Point10 scenes do not present in this container (see "Debugging the built app on a real GPU" below); use the host AppImage + CDP for those.
- `make yarn ARGS="<args>"` — any other yarn command, e.g. `make yarn ARGS="add three"`.
- `make docker-reset` — delete the `modv-node-modules` volume; the next yarn target reseeds it from the toolchain image (use after dependency changes if `make install` is not enough).
- `make shell` — interactive shell in the toolchain container with the repo bind-mounted.

Commands run against the repo bind-mounted at `/app`. Dependencies live in the `modv-node-modules` named volume, seeded from the `toolchain` image and chowned to the host UID/GID so generated files stay writable.

CI is the `nightly.yml` release workflow (Docker build on push to `main` and a nightly schedule) plus `browserslist.yml`. There is **no lint/test CI workflow** — run `make lint` locally.

## Test-driven development

**All changes are test-first.** Write a failing test *before* writing implementation code. Never implement a feature or fix a bug by editing source first and adding tests afterwards. Do not consider a task complete while its tests are absent, failing, or skipped.

Workflow for every change:

1. Write the test that encodes the desired behaviour (a unit test wherever possible).
2. Run it and confirm it fails for the right reason (assertion failure, not an import/syntax error). If it passes immediately, the test is not exercising the new behaviour — fix the test.
3. Write the minimum implementation to make it pass.
4. Re-run the test, then refactor with the test green.
5. Run the relevant suite(s) and `make lint` before reporting completion.

- **Default to a unit test.** Pure logic (audio analysis, detectors, data transforms, reducers, helpers) must be covered by a unit test first: add `tests/unit/<area>.test.js` and run `make test-unit ARGS="tests/unit/<area>.test.js"`. If the logic is tangled with Electron/WebGL, extract the pure part so it can be unit-tested rather than skipping straight to e2e.
- Use an e2e spec **only** for behaviour that genuinely needs Electron, the worker store, or rendering: `make test-e2e ARGS="tests/e2e/spec/<name>.spec.js"` (requires `make build` first). An e2e test that covers logic which could be unit-tested is not a substitute for the unit test.
- Keep regression tests for bugs fixed; the failing test must reproduce the reported behaviour before the fix.
- Tests must be committed alongside the change — a change without its test is incomplete.

## Architecture (three execution contexts)

- **Electron main**: `src/main/index.js` (entry; Chromium switches + single-instance lock), `src/main/window-prefs.js` (BrowserWindow prefs + IPC handlers), `src/main/menu-bar.js`, `src/main/media-manager/` (file watching + read/save handlers for presets, isf, video, image, palette, scene), `src/main/shutdown.js` (quit coordinator), `src/main/system-audio.js` (PulseAudio/PipeWire capture IPC).
- **Preload**: `src/preload/index.js`, `src/preload/colorPicker.js`.
- **Vue renderer**: entries `src/renderer/{index,output-window,color-picker,splash-screen}.html` → `src/renderer/src/main.js`; UI in `src/renderer/src/components/`, UI-only state in `src/renderer/src/ui-store/`.
- **Web Worker (the real engine)**: `src/renderer/src/application/worker/index.worker.js` runs the render loop and holds the authoritative Vuex store.

Key wiring: the worker store (`src/renderer/src/application/worker/store/`) is the source of truth. The renderer's `modV.store.commit/dispatch` forwards to the worker via `postMessage`; the worker batches mutations into a `commitQueue` message the renderer replays into its mirror store. `src/renderer/src/application/index.js` exports the `modV` singleton as `window.modV` / `$modV`.

Gallery sections are driven by module `meta.category` (`effects`, `2d-scenes`, `3d-scenes`, `three`) with type-based fallbacks in `src/renderer/src/components/Gallery.vue`: shader/ISF default to Effects, `2d` to 2D Scenes, and `three` to ThreeJS.

Screen capture is lazy: `App.vue` only mounts `dialogs/ScreenPickerDialog.vue` when `dialogs.open` includes `screenPicker`. The Screen Capture panel's "Pick Screen/Window…" button opens it, and sources enumerate on open — nothing captures or prompts at startup.

## Lifecycle & shutdown

- `src/main/index.js` takes a single-instance lock (`requestSingleInstanceLock`) to stop relaunching from stacking process trees; a `second-instance` event restores and focuses the main window. `app.setName("VmodV")` keeps dev and packaged builds sharing one userData/media tree.
- `src/main/shutdown.js` is the quit coordinator. On `before-quit` it sets the quitting flag, destroys all windows (including `window.open` output windows), stops the media watcher and system-audio capture with short timeouts (`CLEANUP_TIMEOUT_MS`), and starts a `WATCHDOG_TIMEOUT_MS` watchdog that force-exits if cleanup hangs. `stopActive` in `src/main/system-audio.js` is registered as the audio cleanup task rather than using its own `before-quit` hooks.
- Closing the main window is intercepted synchronously (`preventDefault`) unless already quitting; it asks for confirmation outside dev/e2e, sends `modv-destroy`, waits for the renderer's `destroyed` IPC (bounded), runs cleanup, then calls `app.quit`. Both the worker and NDI worker tear down through this path.
- The main window's `close` handler must not block on `await` before deciding: the old async path could leave the window open or skip `app.quit`.

## Auto-registration (`import.meta.glob` in `index.worker.js`)

- Vuex store modules: drop a `.js` in `src/renderer/src/application/worker/store/modules/` → namespaced module.
- Renderers: drop a `.js` in `src/renderer/src/application/renderers/<name>/index.js` exporting `{ render, setupModule, updateModule, resizeModule, tick, resize, createPresetData, loadPresetData, getModuleData, addActiveModule, removeActiveModule }`.
- Modules: drop a `.js` in `renderers/2d/modules/`, `renderers/three/modules/` or `renderers/shader/modules/`; ISF modules are `.fs` (+ optional `.vs`) in `renderers/isf/modules/isf/`, loaded with `?raw`.

## Adding a module

Default-export `{ meta, props, ...lifecycle }`. `meta` carries `name`, `author`, `version`, `type` (`2d` | `shader` | `isf` | `three`), optional `category` (`effects` | `2d-scenes` | `3d-scenes` | `three`) for the Gallery, and `audioFeatures` (e.g. `["bass", "mid", "treble"]`) for requested audio features. 2D modules implement `init`, `resize`, `update`, `draw({ data, props, context, features })` (see `renderers/2d/modules/Ball.js`). Shader modules provide `fragmentShader`/`vertexShader` with props as uniforms (see `renderers/shader/modules/Bands.js`).

Presets: store modules implement `createPresetData`/`loadPresetData`; the worker assembles/loads them via `generatePreset`/`loadPreset`. `worker/preset-serializer.js` is the canonical serialiser: it collects every store module with `createPresetData`, and also recognises Scene documents (`isSceneDocument`) and Scenes playlists (`isScenesPlaylist`, type `modv-scenes-playlist`). A Scene is a preset document plus `id`/`name`/timing metadata.

## Point10 (vendored scenes ported from musicvis)

`src/renderer/src/application/point10/`:

- `adapter.js` — `createPoint10Module({ name, SceneClass, webgpu, meta })`. Each module instance gets its own `OffscreenCanvas` + renderer; the adapter blits into modV's 2D group canvas. Point10 modules default to the 3D Scenes Gallery category; fullscreen 2D scenes can override `meta.category`.
- `BaseScene.js` (WebGL, `three`) and `BaseSceneWebGPU.js` (`three/webgpu` + TSL) define the scene contract: `init()`, `ready`, `available`, `update(dt, audioData)`, `render()`, `resize()`, `dispose()`.
- `audioBridge.js` + `props.js` — the responsiveness levers (`gain`, `smoothing`, `bassGain`, `midGain`, …) are applied in the bridge, **not** inside the vendored scene files, so `scenes/*.js` stay portable with musicvis.
- `sceneProps.js` — per-scene input schemas (smoke/fire, DVD room, Boxy, tunnels, …). The adapter maps modV props onto the live scene's fields/uniforms, again keeping `scenes/*.js` portable.
- `shared/` (`textures.js`, `waterWave.js`, `phraseColor.js`, `particleAlpha.js`, `particleSprite.js`) and `textures/` (Vite assets imported with `?url`). `particleAlpha`/`particleSprite` build the shared circular particle alpha map so `THREE.Points` and billboard quads render round dots instead of squares.
- Some scenes are deliberately background-free/transparent (`ItIsComingForYou`, `RollingFire`/`Ball of Fire`, `Boxy`) so they composite over the layer below.

Gotchas:

- WebGPU presentation uses `canvas.transferToImageBitmap()` with a `readRenderTargetPixelsAsync` fallback — full-frame readback costs ~9 ms/frame at 640×480, transfer ~0.3 ms.
- `max-active-webgl-contexts` is **16** (`src/main/index.js`). The shared shader/gallery-shader/ISF/gallery-ISF/three renderers already hold five contexts; the old cap of 6 left no room for Point10 and force-lost the ISF contexts, so effects silently stopped applying. The adapter still disposes idle scenes (`MAX_LIVE_INSTANCES = 4`, idle > 3 s).
- Textures: `import url from "../textures/foo.jpg?url"` then `loadTexture(url)` — `fetch()` in dev, `fs.readFileSync` for `file://` (asar) in the packaged worker.
- `three` is `^0.185.1` everywhere: WebGL code imports `three`, WebGPU code imports `three/webgpu`, addons come from `three/addons/...`. The main process enables `Vulkan,WebGPU` + `enable-unsafe-webgpu`.
- Spectrum Bars, Ferrofluid and RingOfFire were removed; `point10-removed-effects.test.js` guards against reintroducing them and default Scenes must not reference them.
- Renderer disposal is centralised in `application/renderers/disposal.js` (`disposeISFRenderer`, `disposePexCommand`, `createBoundedCache`). ISF modules share one WebGL context and one unit-quad vertex buffer, so `disposeISFRenderer` releases shaders/programs and image textures but must **never** delete the shared vertex buffer — doing so silently blanks every other live ISF layer.

## Audio

- `src/renderer/src/application/audio-bands.js` — AGC-normalised `bass`/`mid`/`treble` plus 16 log-spaced band cells; published to `store.state.meyda.bands` via `meyda/SET_BANDS`.
- `src/renderer/src/application/worker/phrase-detector.js` — dependency-free phrase/drop detector used by performance mode.
- `src/renderer/src/application/tempo-detector.js` — onset-envelope/autocorrelation BPM tracker used by `setup-beat-detektor.js`; resolves octave ambiguity so fast (170+ BPM) material is not reported as half time.
- `src/renderer/src/application/system-audio.js` — the "System Audio (loopback)" input. It captures a PulseAudio/PipeWire **monitor source** as an input device first (no picker), and only falls back to `getDisplayMedia({ video: true, audio: true })` on macOS/Windows, where Electron's desktop-audio loopback works. Electron's Linux loopback is broken — it returns a stream with no audio track and has been observed to take the browser process down — so the main process never asks for `audio: "loopback"` on Linux and the `PulseaudioLoopbackForScreenShare` Chromium flag is deliberately not enabled.
- The audio graph in `setup-media.js` is built **before** the previous `AudioContext` is closed, so a failure leaves the running graph intact instead of half-replacing it.

## Video inputs

- `src/renderer/src/application/media-inputs.js` holds the pure selection/restore helpers: `isLiveVideoTrack`, `shouldReconnectVideoCapture`, `restoreAudioSource`, `restoreVideoSources` and `selectWebcamSource` (1-based camera index, falling back to the first camera).
- Each checked webcam gets its own capture and output. The Webcam module's **Camera** prop is 1-based and indexes the saved camera order; when a track ends or the device reappears the capture re-opens, and the saved camera selection plus physical audio input are restored on startup (system audio auto-restores only on Linux).
- `src/renderer/src/application/video-capabilities.js` builds the picker's resolution/frame-rate options from `MediaStreamTrack.getCapabilities()` (ranges or value lists) rather than advertising unsupported modes. `mediaStream` keeps per-device `videoSettings`, reported `videoCapabilities` and applied settings, with the legacy global resolution/frame rate as fallback. UI: `components/InputDeviceConfig/AudioVideo.vue`.

## Logging

- `src/main/logger.js` and `src/renderer/src/application/logger.js` write to `<userData>/logs/vmodv-YYYY-MM-DD.log` (`~/.config/VmodV/logs` on Linux), keeping the last seven daily files. `src/shared/logger-format.js` holds the shared line formatter.
- The renderer has no filesystem access of its own, so it forwards its lines to the main process over the `log` IPC channel and everything lands in the same file. The main process also logs `render-process-gone` / `child-process-gone` and uncaught exceptions.
- The "Open Logs" button in the Audio/Video panel (`get-logs-folder` / `open-logs-folder` IPC) shows the user where the file is.

## Scenes & performance mode

- `worker/store/modules/scenes.js` — `applyScene`, `removeScene`, `setPerformanceEnabled`, `updatePerformance`, `loadScenesFromDisk`, `captureLiveStateToScene`, `exportPlaylist`/`importPlaylist`, `setRowNames`, `copySceneCellToScene`; `worker/loop.js` dispatches `scenes/tick` every frame for crossfades/performance mode.
- `performance.allowedTransitions` is a list of transition names for manual Scene changes; each picks one at random (legacy single `transition` values migrate to a one-item list). Fade is the fallback when the list is empty or unavailable. While performance mode is running, every cell change is a straight crossfade regardless of the list (`worker/scene-transition-pick.js`), so automatic transitions are never sudden "None" switches or jarring shader wipes.
- A row fade keeps the old Group alive as a fading `transitionGhost`. `worker/row-transition.js` blends the live outgoing ghost and incoming Group into a per-row scratch canvas, and `loop.js` gives that blend to later rows via `lastCanvas`. Rows above a fading Group inherit through `lastCanvas`, so without the blend Foreground/Effects jump straight to the incoming Group's canvas and hide the fade (the "background/first row changes suddenly" bug). Transition Groups are exempt from the `alpha < 0.001` render cull so both halves exist at the start/end of the fade.
- Global performance timing applies to Scenes without an explicit per-Scene override (`timingOverride`); editing a Scene's Time fields creates an override. `worker/scene-timing.js` normalises timings and picks a per-cell dwell target (`pickCellDwell`/`clampCellDwell`): each row chooses its target once when it becomes active and then advances independently, so "max scene time" is a per-cell maximum rather than all rows changing together. Dwell only advances while no transition is running.
- `scenes/stopPerformance` completes any in-flight crossfade (transition state plus pending Group cleanup) before disabling performance mode, so a stopped performance mode never leaves a stale transition or orphaned Groups behind.
- Predictive Point10 prewarm: while a row plays, `scenes/tick` picks the row's next Scene once (`runtime.nextSceneId`) and `scenes/prewarmRowScene` warms its **WebGPU** scenes through `worker/point10-prewarm.js`. Applying that Scene later adopts the template (`point10/warm-cache.js` keeps at most two; `adapter.js` shares an in-flight warmup and drops templates on stop/reset) so the heavy init does not stall the crossfade. WebGL Point10 scenes stay lazy to protect the shared context budget.
- Scene JSON lives in the project media directory under `scene/` (read/save handlers in `src/main/media-manager/`, `delete-scene` IPC in `src/main/window-prefs.js`). UI: `src/renderer/src/components/Scenes.vue`.
- Default Scenes are versioned in `worker/store/default-scenes.js` (`DEFAULTS_VERSION = 6`; each Scene has `introducedVersion`). `getScenesToSeed({ defaultsVersion, seededNames, existingNames })` seeds only newly introduced Scenes, matched by name, so existing installs receive later batches without duplicates.
- Playlists: `scenes/exportPlaylist` emits a `modv-scenes-playlist` document (Scenes + performance config, no runtime state); `scenes/importPlaylist` merges by id (or replaces). IPC `export-scenes-playlist`/`import-scenes-playlist` in `src/main/window-prefs.js`, and a playlist JSON can be opened like a preset (`isScenesPlaylist` in `preset-serializer.js`).
- Rows can be named (`worker/scene-rows.js`): a named row matches its Group by name (case-insensitive, exact or word-prefix, first match wins) instead of by position, and `performance.rowNames` is persisted in `scenes.json` and travels in playlists. Unnamed rows keep the positional lookup. A digit-suffixed name (`effects2`) only falls back to its base Group (`Effects`) when no sibling row already owns that family (`rowFamilyIsShared`). `scenes/setRowNames` remaps the live layer stack (`groups/orderByIds`) so named rows snap into place. `loop.js` skips the Gallery Group by name/identity, not by assuming it is the last entry, so Gallery-less layouts render every performance row.
- Ctrl+drag a grid cell onto a cell of another Scene to copy that Group exactly (`scenes/copySceneCellToScene`): the Group and Module snapshots are copied verbatim (ids included — Scene documents are applied by rebuilding Groups/Modules with fresh ids), together with the saved inputs, input links, expression assignments and tweens. Rows the target Scene does not define become `null` holes in `groups`; the apply loop skips them and keeps the row indexes aligned. The drop only fires if `dragover` calls `preventDefault()` — the highlight is drawn from component state, so it looks correct even when the browser rejects the drop.
- Saving live state: the column-header context menu (`src/renderer/src/menus/context/sceneHeaderContextMenu.js`) dispatches `scenes/captureLiveStateToScene`. `worker/scene-capture.js` collects only the applied Scene's Groups (non-ghost, non-Gallery and stamped with `sceneId`/`sceneRow`), rebuilding `groups` with `null` row holes; it refuses while performance mode is running.
- Hover previews: `worker/scene-preview.js` + `scene-preview-data.js` + the `scenePreview` store module render one cell preview at ~30 fps from saved Group/Module snapshots (fresh module ids, built outside the live store). Preview modules must be disposed through `application/renderers/disposal.js`; deleting the shared ISF vertex buffer blanks every live ISF layer (the old hover-preview bug).
- Automatic Scene links: `worker/scene-link-defaults.js` sets a slow BPM Tween (`bpmDivision: 32`, `easeInOutSine`) and excludes structural Twirl/Kaleidoscope props so filters breathe instead of warping on every beat. `worker/input-link-value.js` skips writing a linked value until the audio feature exists, so `NaN` never overwrites a saved input value.

## Lint & formatting

`.eslintrc.cjs`: `eslint:recommended` + `plugin:vue/vue3-recommended` + `@electron-toolkit` + `@vue/eslint-config-prettier`, `parserOptions.ecmaVersion: 2023`. Formatting is Prettier (`@vue/eslint-config-prettier`); EditorConfig = 2-space, LF.

## Build / platform gotchas

- Renderer has multiple entries and matching preloads (see `electron.vite.config.mjs`); workers build with `worker.format: "es"`.
- The worker runs with `nodeIntegrationInWorker: true` (`src/main/window-prefs.js`), so worker code may `require("fs")`/`require("path")`.
- Native deps: `grandiose` (NDI) needs libndi on Linux; `grandiose` and `ffmpeg-fluent` are `asarUnpack`ed (see `electron-builder.yml`). The `Dockerfile` installs the NDI SDK and aliases the `libndi.so.5.6.1` soname to the `libndi.so.5.1.1` its `binding.gyp` pins. Windows needs the node-gyp toolchain — **WSL/WSL2 is unsupported**.
- `patch-package` patches live in `patches/` (golden-layout, hapticjs, smooth-dnd). After `make yarn ARGS="remove <pkg>"`, run `make install` again to reapply patches (see issue #554).
- electron-builder: `appId: gl.vcync.modv`, output directory `dist/`. `Dockerfile` is multi-stage (`deps` → `build` → `package` → `appimage`/`deb`/`artifacts`, plus a `toolchain` stage for the Dockerised yarn commands); `docker build --target artifacts --output type=local,dest=dist .` emits the installers, and `make build` is the supported local build. Do not run `electron-builder` locally.

### Vite / build toolchain

- Version matrix (Vite 6 upgrade): `vite` `^6.4.4` (from `^5.0.12`), `electron-vite` `^3.1.0` (from `^2.0.0`, for Vite 6 support), `@vitejs/plugin-vue` `^5.2.4`, `vite-plugin-node-polyfills` `^0.24.0`, `vite-svg-loader` `^5.1.3`, `vitest` `^5.0.5` (from `^2.1.9`; removes the duplicate Vite 5 toolchain — Vitest 5 requires Vite >= 6.4). The unused `vite-plugin-electron-renderer` was removed.
- In `electron.vite.config.mjs`, `nodePolyfills()` must run **before** `vue()`/`svgLoader()` — plugin order is sensitive.
- Vite 6 enables `commonjsOptions.strictRequires` by default. Worker `require()` paths (`fs`, `path`, `grandiose`, `node-osc`) should be smoke-tested whenever build config changes.
- Verified after the upgrade: the unit suite passes, `make lint` is clean, `make build` produces the AppImage and `.deb`, and the e2e smoke passes.

## Release

Nightly releases are produced by `.github/workflows/nightly.yml` (Docker build on `main` pushes plus a nightly schedule) and published as the rolling `nightly` prerelease. There is no `semantic-release` workflow; do not manually bump `version` in `package.json` (CI overrides it per build).

## Unit testing notes

Vitest 5 (`^5.0.5`) runs on Vite 6 (`^6.4.4`) and is configured in `vitest.config.mjs` (`environment: "node"`, `include: ["tests/unit/**/*.test.js"]`, `@renderer` alias). Import modules with relative paths from `tests/unit/`. `tests/unit/` covers audio (`audio-bands`, `tempo-detector`, `phrase-detector`, `system-audio*`), video/media (`media-inputs`, `media-stream`, `video-capabilities`, `media-manager`), scenes (`scene-rows`, `scene-timing`, `scene-capture`, `scene-preview-data`, `scene-link-defaults`, `input-link-value`, `default-scenes`, `preset-serializer`), renderers/disposal (`renderer-disposal`, `disposal`), logging/shutdown (`logger`, `shutdown`), and the Point10 scenes/inputs/particle/audio-bridge modules. Keep unit tests free of Electron/WebGL so they run in plain Node; extract pure logic from Electron/WebGL code rather than reaching for e2e.

## e2e testing notes

Tests use a singleton page object `modVApp` (`tests/e2e/pageObjectModel/`) that launches the built app once in `beforeAll` and waits for the `is-modv-ready` IPC. Helpers `evaluateMainState`, `evaluateWorkerState`, `evaluateWorker` and `checkWorkerAndMainState` read state from both contexts; `evaluateWorkerState` runs in the dedicated worker using `self.store` and `self._get` (lodash.get, exposed for tests). Custom matcher `toBeJSON` is in `tests/e2e/extentions/toBeJSON.js`.

The container runs as the host UID/GID, so it does **not** inherit the host's supplementary `video`/`render` groups. `make test-e2e` adds the `/dev/dri` device GIDs via `--group-add`; without that the forwarded GPU nodes are unreadable, Chromium's GPU process aborts (`exit_code=133` / SIGTRAP) under load, and every renderer sharing the app goes with it. If e2e crashes in `performance mode dwell`, check `/dev/dri` access first.

Docker runs use `--ulimit core=0` and `.dockerignore` excludes `core*`: a crashed renderer used to write a ~1.1 TB sparse core dump into the repo, which then got copied into the Docker build context and filled the disk.

The page object logs Electron stderr (`ELECTRON_ENABLE_LOGGING=1`), listens for `crash`/`close`, and `getWorker()` waits for the worker before evaluating, so a renderer crash fails with a clear message instead of `Cannot read properties of undefined`. `src/main/index.js` logs `render-process-gone` / `child-process-gone` reasons and exit codes.

The page object seeds/hydrates Scenes before announcing `is-modv-ready`, so tests never observe the temporary Groups built while defaults are seeded. It resolves the built app through `pageObjectModel/resolveLatestBuild.js`, which falls back to extracting the newest AppImage when `findLatestBuild` has no unpacked directory (the canonical Docker build emits installers). It also kills a crashed Electron main process so Playwright does not idle ~30s before restarting.

Specs under `tests/e2e/spec/{scenes,point10,main,packaging}/` cover scene snapshots/performance, Scenes defaults, live-state capture, gallery-less layouts, per-cell timing, hover-preview isolation, Point10 effect/transition registration plus Boxy transparency, quit cleanup, and the trimmed Linux bundle. The full suite must be green before reporting completion.

Known flake: `performance mode dwell advances in real time` can still take the Electron renderer down natively (`reason=crashed`, `exitCode=5` / SIGTRAP) — in one host-GPU state the suite was `58/58` three times in a row, in another it failed the same spec every time. The container shares the GPU with the desktop compositor and other apps, so the rate depends on host GPU load. It is not a Vite regression and is independent of the GL backend (SwiftShader), of WebGPU, and of the WebGL context cap (16 or 32). Re-run the spec to confirm.

Known flake: `scenes/performanceTransitions.spec.js` fails one of its four tests on a full-suite run, and it is a different test each time (the transition specs time out waiting for the shader to compile). It reproduces on an unmodified checkout, so it is a GPU-load artefact of the shared device rather than a regression. Re-run the spec on its own to confirm.

Runs use **1 worker** (`MODV_E2E_WORKERS` overrides). Every worker launches its own Electron app, and the suite is GPU-bound: two concurrent apps over-subscribe the single GPU and Chromium aborts the renderer (`reason=crashed`, `exitCode=5`), failing whichever specs were running (reproducibly the ISF transition and performance-dwell specs). One worker is as fast as two and stable; use `--workers=2` only with GPU isolation.

## Debugging the built app on a real GPU (AppImage + CDP)

The Dockerised e2e container cannot exercise WebGPU: the app exposes `navigator.gpu`, but Point10 WebGPU scenes fail while building their TSL pipeline (`THREE.NodeBuilder: Uniform "storageTexture" not implemented.`), so `scene.render()` never settles and no frame is presented. To debug rendering — including WebGPU Point10 scenes — run the built AppImage directly on the host with remote debugging and drive it over CDP with the `browser-harness` skill:

```bash
make appimage
nohup ./dist/VmodV-<version>.AppImage --appimage-extract-and-run \
  --remote-debugging-port=9223 --no-sandbox > tmp/app-run.log 2>&1 &
```

```bash
BU_CDP_URL=http://127.0.0.1:9223 browser-harness <<'PY'
targets = cdp("Target.getTargets")["targetInfos"]
main = [t for t in targets if "VmodV" in t.get("title", "")][0]
switch_tab(main["targetId"])
print(js("({title: document.title, store: typeof window.modV.store})"))
print(capture_screenshot("tmp/main.png"))
PY
```

- `--appimage-extract-and-run` avoids a FUSE dependency. `Target.getTargets` lists the main window, `output-window.html` and `color-picker.html`; the render worker is **not** a separate target. Drive it through the main renderer's `window.modV.store.dispatch/commit`, which forwards to the worker and resolves with the action's return value (e.g. `groups/createGroup` returns the Group, `modules/makeActiveModule` returns the Module — use `.id` / `.$id`).
- The output window's canvas has been transferred to the worker (`transferControlToOffscreen`), so `getContext("2d")` there throws — use `capture_screenshot` / `Page.captureScreenshot` instead of reading pixels.
- Inspect frames through the daily log at `~/.config/VmodV/logs/vmodv-YYYY-MM-DD.log`: `[point10-diag]` (from `point10/adapter.js`) reports each scene bitmap's `lit`/`bitmapAgeMs`/`presenting`, and `[render-diag]` (from `worker/render-diag.js`) reports the root canvas before/after `clearRect`, each Group's canvas and its inherit source, so frame-feedback loops are visible as a climbing `lit`/`alpha`.
- `app.setName("VmodV")` means the host AppImage shares the real `~/.config/VmodV` userData/media tree — it seeds/loads the user's Scenes and the user's log file. Note the log offset before a run and read only appended lines.
- Quit with the specific Electron PIDs (`for p in $(pgrep -x vmodv); do kill "$p"; done`); do not use `pkill -f`.
