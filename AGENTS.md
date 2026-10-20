# AGENTS.md

modV — modular audio visualisation app. Electron + Vue 3 + Vuex, built with **electron-vite** (Vite) and **electron-builder**.

## Commands

All yarn work is Dockerised. The host only needs Docker (with Buildx) and make — there is no host Node or Yarn.

- `make docker-deps` — build the `modv-toolchain:local` image from the Dockerfile `toolchain` stage (Node 24 + dependencies + Electron runtime).
- `make install` — rebuild the toolchain image and reseed `node_modules` (clean `yarn install`; applies `patches/` and fetches the Electron binary).
- `make dev` — Dockerised `yarn dev` with HMR. On Linux, a Wayland socket is forwarded when available (falling back to X11), along with PulseAudio and `/dev/dri` (GPU); override with `DISPLAY_BACKEND=x11` or `DISPLAY_BACKEND=wayland`. Extend with `DOCKER_RUN_ARGS="..."`. For CDP attach: `make dev ARGS="-- --remote-debugging-port=9223"`.
- `make build` — **the canonical Linux build** (see `Dockerfile`); empties `dist/` in place and writes the AppImage and `.deb` into it. Also used by CI. Set `APP_VERSION` to override the version, e.g. `APP_VERSION=1.2.3 make build`; `make clean` removes `dist/`. Do not use a local `electron-builder` build.
- `make yarn ARGS="build"` — bundle main / preload / renderer into `out/`.
- `make lint` — Dockerised `yarn lint` (`eslint . --fix`, Prettier runs through ESLint).
- `make format` — Dockerised `yarn format`.
- `make test-e2e` — Dockerised Playwright. Electron is launched with `--no-sandbox` and the Wayland/X11, PulseAudio and GPU forwarding above. **Requires `make build` first**: the page object launches the built app from `./dist` via `findLatestBuild`, falling back to extracting the newest AppImage. Single spec: `make test-e2e ARGS="tests/e2e/spec/<name>.spec.js"`.
- `make yarn ARGS="<args>"` — any other yarn command, e.g. `make yarn ARGS="add three"`.
- `make docker-reset` — delete the `modv-node-modules` volume; the next yarn target reseeds it from the toolchain image (use after dependency changes if `make install` is not enough).
- `make shell` — interactive shell in the toolchain container with the repo bind-mounted.

Commands run against the repo bind-mounted at `/app`. Dependencies live in the `modv-node-modules` named volume, seeded from the `toolchain` image and chowned to the host UID/GID so generated files stay writable.

CI is the `nightly.yml` release workflow (Docker build on push to `main` and a nightly schedule) plus `browserslist.yml`. There is **no lint/test CI workflow** — run `make lint` locally.

## Architecture (three execution contexts)

- **Electron main**: `src/main/index.js` (entry; Chromium switches), `src/main/window-prefs.js` (BrowserWindow prefs + IPC handlers), `src/main/menu-bar.js`, `src/main/media-manager/` (file watching + read/save handlers for presets, isf, video, image, palette, scene).
- **Preload**: `src/preload/index.js`, `src/preload/colorPicker.js`.
- **Vue renderer**: entries `src/renderer/{index,output-window,color-picker,splash-screen}.html` → `src/renderer/src/main.js`; UI in `src/renderer/src/components/`, UI-only state in `src/renderer/src/ui-store/`.
- **Web Worker (the real engine)**: `src/renderer/src/application/worker/index.worker.js` runs the render loop and holds the authoritative Vuex store.

Key wiring: the worker store (`src/renderer/src/application/worker/store/`) is the source of truth. The renderer's `modV.store.commit/dispatch` forwards to the worker via `postMessage`; the worker batches mutations into a `commitQueue` message the renderer replays into its mirror store. `src/renderer/src/application/index.js` exports the `modV` singleton as `window.modV` / `$modV`.

Gallery sections are driven by module `meta.category` (`effects`, `2d-scenes`, `3d-scenes`, `three`) with type-based fallbacks in `src/renderer/src/components/Gallery.vue`: shader/ISF default to Effects, `2d` to 2D Scenes, and `three` to ThreeJS.

Screen capture is lazy: `App.vue` only mounts `dialogs/ScreenPickerDialog.vue` when `dialogs.open` includes `screenPicker`. The Screen Capture panel's "Pick Screen/Window…" button opens it, and sources enumerate on open — nothing captures or prompts at startup.

## Auto-registration (`import.meta.glob` in `index.worker.js`)

- Vuex store modules: drop a `.js` in `src/renderer/src/application/worker/store/modules/` → namespaced module.
- Renderers: drop a `.js` in `src/renderer/src/application/renderers/<name>/index.js` exporting `{ render, setupModule, updateModule, resizeModule, tick, resize, createPresetData, loadPresetData, getModuleData, addActiveModule, removeActiveModule }`.
- Modules: drop a `.js` in `renderers/2d/modules/`, `renderers/three/modules/` or `renderers/shader/modules/`; ISF modules are `.fs` (+ optional `.vs`) in `renderers/isf/modules/isf/`, loaded with `?raw`.

## Adding a module

Default-export `{ meta, props, ...lifecycle }`. `meta` carries `name`, `author`, `version`, `type` (`2d` | `shader` | `isf` | `three`), optional `category` (`effects` | `2d-scenes` | `3d-scenes` | `three`) for the Gallery, and `audioFeatures` (e.g. `["bass", "mid", "treble"]`) for requested audio features. 2D modules implement `init`, `resize`, `update`, `draw({ data, props, context, features })` (see `renderers/2d/modules/Ball.js`). Shader modules provide `fragmentShader`/`vertexShader` with props as uniforms (see `renderers/shader/modules/Bands.js`).

Presets: store modules implement `createPresetData`/`loadPresetData`; the worker assembles/loads them via `generatePreset`/`loadPreset`.

## Point10 (vendored scenes ported from musicvis)

`src/renderer/src/application/point10/`:

- `adapter.js` — `createPoint10Module({ name, SceneClass, webgpu, meta })`. Each module instance gets its own `OffscreenCanvas` + renderer; the adapter blits into modV's 2D group canvas. Point10 modules default to the 3D Scenes Gallery category; fullscreen 2D scenes can override `meta.category`.
- `BaseScene.js` (WebGL, `three`) and `BaseSceneWebGPU.js` (`three/webgpu` + TSL) define the scene contract: `init()`, `ready`, `available`, `update(dt, audioData)`, `render()`, `resize()`, `dispose()`.
- `audioBridge.js` + `props.js` — the responsiveness levers (`gain`, `smoothing`, `bassGain`, `midGain`, …) are applied in the bridge, **not** inside the vendored scene files, so `scenes/*.js` stay portable with musicvis.
- `shared/` (`textures.js`, `waterWave.js`, `phraseColor.js`) and `textures/` (Vite assets imported with `?url`).

Gotchas:

- WebGPU presentation uses `canvas.transferToImageBitmap()` with a `readRenderTargetPixelsAsync` fallback — full-frame readback costs ~9 ms/frame at 640×480, transfer ~0.3 ms.
- `max-active-webgl-contexts` is **16** (`src/main/index.js`). The shared shader/gallery-shader/ISF/gallery-ISF/three renderers already hold five contexts; the old cap of 6 left no room for Point10 and force-lost the ISF contexts, so effects silently stopped applying. The adapter still disposes idle scenes (`MAX_LIVE_INSTANCES = 4`, idle > 3 s).
- Textures: `import url from "../textures/foo.jpg?url"` then `loadTexture(url)` — `fetch()` in dev, `fs.readFileSync` for `file://` (asar) in the packaged worker.
- `three` is `^0.185.1` everywhere: WebGL code imports `three`, WebGPU code imports `three/webgpu`, addons come from `three/addons/...`. The main process enables `Vulkan,WebGPU` + `enable-unsafe-webgpu`.

## Audio

- `src/renderer/src/application/audio-bands.js` — AGC-normalised `bass`/`mid`/`treble` plus 16 log-spaced band cells; published to `store.state.meyda.bands` via `meyda/SET_BANDS`.
- `src/renderer/src/application/worker/phrase-detector.js` — dependency-free phrase/drop detector used by performance mode.
- `src/renderer/src/application/tempo-detector.js` — onset-envelope/autocorrelation BPM tracker used by `setup-beat-detektor.js`; resolves octave ambiguity so fast (170+ BPM) material is not reported as half time.

## Scenes & performance mode

- `worker/store/modules/scenes.js` — `applyScene`, `removeScene`, `setPerformanceEnabled`, `updatePerformance`, `loadScenesFromDisk`; `worker/loop.js` dispatches `scenes/tick` every frame for crossfades/performance mode.
- `performance.allowedTransitions` is a list of transition names; each scene change picks one at random (legacy single `transition` values migrate to a one-item list). Fade is the fallback when the list is empty or unavailable.
- Global performance timing applies to Scenes without an explicit per-Scene override (`timingOverride`); editing a Scene's Time fields creates an override. Dwell only advances while no transition is running.
- Scene JSON lives in the project media directory under `scene/` (read/save handlers in `src/main/media-manager/`, `delete-scene` IPC in `src/main/window-prefs.js`). UI: `src/renderer/src/components/Scenes.vue`.

## Lint & formatting

`.eslintrc.cjs`: `eslint:recommended` + `plugin:vue/vue3-recommended` + `@electron-toolkit` + `@vue/eslint-config-prettier`, `parserOptions.ecmaVersion: 2023`. Formatting is Prettier (`@vue/eslint-config-prettier`); EditorConfig = 2-space, LF.

## Build / platform gotchas

- Renderer has multiple entries and matching preloads (see `electron.vite.config.mjs`); workers build with `worker.format: "es"`.
- The worker runs with `nodeIntegrationInWorker: true` (`src/main/window-prefs.js`), so worker code may `require("fs")`/`require("path")`.
- Native deps: `grandiose` (NDI) needs libndi on Linux; `grandiose` and `ffmpeg-fluent` are `asarUnpack`ed (see `electron-builder.yml`). The `Dockerfile` installs the NDI SDK and aliases the `libndi.so.5.6.1` soname to the `libndi.so.5.1.1` its `binding.gyp` pins. Windows needs the node-gyp toolchain — **WSL/WSL2 is unsupported**.
- `patch-package` patches live in `patches/` (golden-layout, hapticjs, smooth-dnd). After `make yarn ARGS="remove <pkg>"`, run `make install` again to reapply patches (see issue #554).
- electron-builder: `appId: gl.vcync.modv`, output directory `dist/`. `Dockerfile` is multi-stage (`deps` → `build` → `package` → `appimage`/`deb`/`artifacts`, plus a `toolchain` stage for the Dockerised yarn commands); `docker build --target artifacts --output type=local,dest=dist .` emits the installers, and `make build` is the supported local build. Do not run `electron-builder` locally.

## Release

Nightly releases are produced by `.github/workflows/nightly.yml` (Docker build on `main` pushes plus a nightly schedule) and published as the rolling `nightly` prerelease. There is no `semantic-release` workflow; do not manually bump `version` in `package.json` (CI overrides it per build).

## e2e testing notes

Tests use a singleton page object `modVApp` (`tests/e2e/pageObjectModel/`) that launches the built app once in `beforeAll` and waits for the `is-modv-ready` IPC. Helpers `evaluateMainState`, `evaluateWorkerState`, `evaluateWorker` and `checkWorkerAndMainState` read state from both contexts; `evaluateWorkerState` runs in the dedicated worker using `self.store` and `self._get` (lodash.get, exposed for tests). Custom matcher `toBeJSON` is in `tests/e2e/extentions/toBeJSON.js`.

Specs under `tests/e2e/spec/{scenes,point10,packaging}/` cover scene snapshots/performance, Point10 effect/transition registration, and the trimmed Linux bundle.
