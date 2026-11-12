# Rust hybrid engine — code review

Review of the committed `rust-hybrid` branch against `origin/main`. Written for
the engineer(s) building the engine; intended to be read alongside
`docs/rust-hybrid.md` and `docs/rust-hybrid-progress.md`.

## Status after the fix wave

All findings below were re-checked against the fixed tree. The fixes landed in
separate commits by area (engine/ipc, media, render/2D/Point10, ISF,
app/src/main/renderer).

| Finding | Status | Notes |
|---|---|---|
| R-1 ISF trailing implicit pass | **Fixed** | last pass' target only; magenta regression test |
| R-2 / R-3 PipeWire negotiation | **Fixed** | F32/S16 negotiation, readiness after accept, negotiated rate, latched errors |
| R-4 cold-start sync | **Fixed** | `engine/get-sync` + idempotent replay (tweens/expressions included) |
| R-5 / R-6 BPM/FPS tweens | **Fixed** | bpm>0 enforced, per-entry serialisation, 120-BPM fallback |
| R-7 non-loop seek | **Fixed** | restored frame for both branches |
| R-8 / R-9 tween reload/playheads | **Fixed** | prune instead of reload; per-entry decode |
| R-10 scene-preview thrash | **Fixed** | retained instances per cached cell; scoped eviction |
| R-11 sync drops tweens/expressions | **Fixed** | pure replay helper |
| R-12 preview IPC | **Fixed** | magic/sequence/bounds/bytesRead validation + retry |
| R-13 stop/remove transitions | **Fixed** | both complete transitions; capture dropped |
| R-14 migration duplicate removal | **Fixed** | one seeded Scene per rule |
| R-15 scene-capture rows | **Fixed** | bounded BTreeMap, null holes preserved |
| R-16 frame-graph usage | **Fixed** | COPY_DST validated |
| R-17 2D self-sampling/clip/clear | **Fixed** | snapshot, intersect, ordered erase |
| R-18 OSC bounds | **Fixed** | array/argument/bundle/store caps |
| R-19 tempo options | **Fixed** | finite/bounded |
| R-20 socket unlink | **Fixed** | stale-only + inode-checked drop, 0600 |
| R-21 preview ring | **Fixed** | permissions, invalid unwritten slots, publish ordering |
| R-22 RT allocations | **Fixed** | per-stream scratch buffers |
| R-23 unsupported engine features | **Partial** | Output Window/MIDI/OSC disabled with hints + boot warning; plugins/fonts only warned; BPM MIDI source still selectable |
| R-24 supervisor spawn/restart | **Fixed** | error routing, await-exit, budget reset |
| R-25 preload scope / channel list | **Fixed** | main window only + sender validation |
| R-26 ISF robustness edges | **Fixed** | size clamp, keywords, texture2DRect, fingerprints, validation, FRAMEINDEX, `$` scalars, bool truthiness, matrix locations |
| R-27 corpus/overrides | **Partial** | overrides embedded + fingerprinted + baseline gate; the 7 corpus shaders still have no override files, so they stay unrendered |
| R-28 Point10 drift/mips | **Fixed** | band/beat/clock/mips/cube clip-z; ItIsComing keeps preserveBloomAlpha (documented deliberate transparency) and AroundPlus keeps its GL-shaped matrix (used for its screen-space water maths) |
| R-29 restore/performance | **Fixed** | runtime cleared + rows republished; incoming above ghost; row-name clones removed. Whole-scene Fade now interleaves per row rather than JS's unnamed-row block (documented; inheritance remapped) |
| R-30 ISF bool coercion | **Fixed** | `IsfValue::coerce_bool` used by the compositor |

P3 hygiene items (engine_state slicing, preset null holes, hostile
`sceneRow` decode, hex timings, `stroke_text` mask re-uploads, final delta
flush, windowless supervisor parking) remain open and are tracked here.

## Scope and method

- **Reviewed:** everything committed on `rust-hybrid` vs `origin/main`, i.e.
  the merge base `76816a0f` through the tip at the time of each review pass
  (latest pass: **`36672a33`** — see the [Round log](#round-log)). This is
  ~107k added lines across 205 files: six Rust crates (`crates/`, ~90k lines),
  the Electron/main-process and renderer integration, tests, build files and
  docs.
- **Method:** static review only. Nothing was compiled, run, linted or built;
  every statement below comes from reading the code and cross-checking it
  against the legacy JS being ported (worker store, renderers, `point10/`,
  `isf/`) and against `docs/rust-hybrid.md`. GPU behaviour was verified by
  reading only. Line numbers refer to the file **at the reviewed commit** and
  drift as the branch advances; paths are repo-relative.
- **Verification marks:** `✔` = re-verified by the reviewer at the reviewed
  commit; `◐` = reported by a reviewer pass, checked by reading the specific
  site but not exhaustively cross-checked.
- **Severity:** P1 = likely real-world defect (wrong output, crash, data
  loss); P2 = robustness / edge-case / parity break worth fixing; P3 = minor,
  hygiene, or latent.
- The review covers **committed code only**. Uncommitted work in the main
  checkout (e.g. the `--audio-source` wiring described in the freshly edited
  `AGENTS.md`, and `crates/modv-app/src/audio.rs`) is out of scope and will be
  reviewed once it lands.
- The `/tmp/opencode/wt` review worktree was deleted externally mid-review;
  the remainder was reviewed from a `git archive` extraction of the committed
  tip in `/tmp/opencode/` and from the main checkout's git object store, with
  the working tree never touched.

## Verdict

The engine is real progress: the state model, delta stream, blend table, delta
it replays into the mirror, the VMP1 seqlock ring, the NDI FFI signatures, the
AGC bands port, the tween fixture (re-derived independently from anime.js
3.2.2 — it is genuine output) and the delta-replay integration are all
carefully done and well tested for the scale. The porting discipline (JS-named
functions, JS line citations, deliberate-deviation comments) is unusually good
and made this review productive.

It is **not** ready to be the default renderer. There is at least one P1 that
makes a class of ISF modules render wrong output, one P1 that makes the
PipeWire capture path analyse garbage while reporting success, one P1 that
leaves the renderer mirror permanently stale on a cold start, and (Round 3)
a hero Point10 scene whose hoops render as solid washers. The Tweens and
Scene-preview subsystems (the two newest, least baked workstreams) both have
correctness defects of the data-loss/perf-correctness class rather than mere
gaps. Several are cheap fixes; none of this is structural.

## P1 — fix before anything else

### R-1 `crates/modv-renderers/src/isf/runtime.rs:838` ✔
Trailing-implicit final passes are blitted over, breaking real ISF shaders

```rust
let last_target = passes.iter().rev().find_map(|pass| pass.target);
```

takes the last **targeted** pass. The JS sets `lastTarget = null` on every
implicit pass and only blits when the *final* pass has a target — an implicit
final pass is the destination. **33 corpus shaders have a trailing implicit
final pass** (measured by scanning `src/renderer/.../isf/modules/isf/*.fs`:
Glow, Soft Blur, Lens Flare, Long Exposure, Auto Levels, Echo Trace, Ghosting,
VVMotionBlur 3.0, …, full list: `Auto Color Tone, Auto Levels, Circle Trails,
Color History, Digital Clock, Echo Trace, Ghosting, Glow, HorizVertHold, Key
Frame Artifacts, Lens Flare, Long Exposure, Micro Buffer (RGB), MultiFrame
(2x2, 3x3), Noise Adapt, Optical Flow (Distort, Generator), RE RGB Gradient
Generator, RGB Strobe, RGB Trails 3.0, Saturation Bleed, Shockwave Pulse,
Smudged Lens, Soft Blur, Strobe, Time Glitch RGB, VVMotionBlur 3.0, Vertical
Tearing, hexagons, rgbtimeglitch, rotozoomer`). For them, after the final
(implicit) pass renders into the destination, this code performs an extra
nearest-filtered blit of the pre-final buffer over the result
(`runtime.rs:1095`), discarding the final pass. The same wrong contract is
baked into the docs (`runtime.rs:19-21`, `overrides/README.md:77-78`,
`mod.rs`), the 344/351 “success” narrative and the multi-pass unit test
(which only asserts the all-targeted shape).
**Fix:** `let last_target = passes.last().and_then(|pass| pass.target);` and
add a trailing-implicit test that would have caught this.

### R-2 `crates/modv-media/src/audio/pipewire.rs` (`param_changed` ~L256, `process` ~L284) ✔
Unsupported negotiated format is decoded as f32 anyway

On a format mismatch, `param_changed` calls `queue.set_error(...)` and returns,
but the `process` callback then blindly runs `f32::from_le_bytes` on whatever
arrives, regardless of the negotiated format (e.g. S16LE @ 44.1 kHz → garbage
floats). Worse, `ChunkQueue::take_error` is one-shot (`capture.rs:192`), so
the error is logged once and never again, and `PipeWireMonitor::start`
reports the *requested* 48 kHz unconditionally (`pipewire.rs:105-110`), while
the analyser is built at 48 kHz from a stream that may be 44.1 kHz/16-bit.
**Fix:** park the stream on `process` (or bail the callback) until a supported
format is negotiated; keep the error latched instead of one-shot; report the
negotiated rate/format to the caller rather than the requested one.

### R-3 `crates/modv-media/src/audio/pipewire.rs` (`ready.send(Ok(()))` ~L373) ✔
PipeWire capture reports ready before format negotiation

`ready.send(Ok(()))` fires synchronously after `stream.connect()` returns,
before any `param_changed`. A silent/wrong-rate device reports a healthy
capture and the engine runs on with no indication (XRun counters or a
bytes-received check exist for this is on the caller).
**Fix:** send `Ok(())` on the first accepted `param_changed` (or after the
first buffer batch), error otherwise.

### R-4 `src/renderer/src/application/index.js:288` ✔
Cold start drops `state/sync`; the mirror is permanently stale

The engine emits `preview/ready` + `state/sync` immediately on accepting a
client (`crates/modv-app/src/supervisor.rs:218-234`). The main process taps
the client and forwards `engine/state-sync` to the main window only after
spawning (`src/main/engine-ipc.js:218`); the renderer subscribes in
`application/index.js:142` at module init. Electron does not queue IPC: a
renderer that has not subscribed yet silently loses the sync. `onStateSync`
has no late-joiner fallback — unlike `onState`, which pins the current
transition via `engine.getState()` — and nothing in the renderer calls
`snapshot()`. The mirror then never receives the pre-connect Groups,
Modules, `registered` map, seeded Scenes, or restored tweens, and only ever
sees post-boot deltas. (`tests/e2e/spec/engine/engineParity.spec.js` does
assert `registered > 300`, so the harness catches a fully-lost sync on the
happy path; it cannot recover the state and cannot detect a partial replay.)
**Fix:** mirror the `onState` pattern — pin a `state/sync` request at renderer
boot (an invoke that returns the current sync payload, replayed idempotently),
or have the engine re-emit sync on renderer handshake. Either way, add an e2e
case that kills the engine between spawn and renderer-ready.

### R-31 `crates/modv-renderers/src/point10/scenes/geometric_tunnel.rs:1326` ✔
GeometricTunnel frame hoops render ~16–24× too deep (neon washer walls)

The `Ring` band mesh is built with `depth = 1.0` (`build_ring_band(*sides, 1.0, 0.08, 1.0)` ~L1862) and every band instance carries its own z-scale — inner `0.035` (~L1355), ros tiles `0.02` (~L1493), target `0.045/0.03` (~L1496), braces `0.08` — **except the frame (outer) instance**: `frame_model = mat4_mul(&group, &rotation_z(frame_rotation))` has no z-scale, so the hoops extend ±0.5 × `depth_scale` (0.82 + bass·0.22 + flash·0.18) ≈ ±0.41–0.61 world units deep. The JS builds the identical hoop with `TUNNEL_HOOP_DEPTH = 0.05` (`GeometricTunnel.js:244`), i.e. ±0.025. Everything renders — just not the tunnel that was authored; the hoops read as solid washers.
**Fix:** give the frame instance a z-scale of `TUNNEL_HOOP_DEPTH` (or build the frame `Ring` band with `depth = TUNNEL_HOOP_DEPTH`).

## P2 — parity breaks and robustness

### R-5 `crates/modv-app/src/store_ops.rs:1030` + `tweens.rs:228` + `store_ops.rs:1440` ✔
`beats/SET_BPM 0` destroys every BPM tween descriptor in the state

`beats/SET_BPM` accepts any finite `bpm >= 0` and feeds it to
`update_tween_bpm`, which does `(3600.0 / 0.0) * division` → `Inf`
(`tweens.rs:895`) and stores it as `descriptor.duration`. `to_json()` →
`serde_json::to_value` refuses `Inf`, falls back to `{}`, and assigns that to
`self.state.tweens` — **every** BPM tween descriptor silently vanishes from
the authoritative state, the next snapshot and the next preset/scene capture.
`set_fps` guards this asymmetrically (`store_ops.rs:1309`, `.max(1.0)` — see
R-6).
**Fix:** clamp bpm at the action boundary (`bpm > 0`, else no-op like other
invalid params), and make the `to_json` fallback per-entry lenient (skip + warn
the offender) instead of collapsing the whole map to `{}`.

### R-6 `crates/modv-app/src/store_ops.rs:1309` ✔
`fps/SET_FPS` rebuilds tweens with `bpm().max(1.0)`

Every other rebuild path passes the raw bpm and relies on
`effective_duration` → 120 fallback (`tweens.rs:210`). `max(1.0)` gives a
division-32 BPM tween a ~57-second / ~3456-frame rebuild instead of ~480 ms
at bpm≈0, and diverges from the JS `store.state.beats.bpm || 120`
(`tweens.js:46`).
**Fix:** drop `.max(1.0)`.

### R-7 `crates/modv-engine/src/tweens.rs:719` ✔
Non-looping tween rebuilds lose the `seek` position

The no-loop branch returns `(frames, 0)`; the JS sets
`frames[id] = Math.floor(len * seek)` for both branches
(`tweens.js:129`). Every `beats/SET_BPM` / `fps/SET_FPS` resets non-looping
tweens to frame 0 — position jumps, visible mid-show.
**Fix:** `return (frames, restored_frame(frames.len(), seek));`

### R-8 `crates/modv-app/src/scene_ops.rs:1875` + `store_ops.rs:1412` ✔
Scene restore re-seeds tween playheads to frame 0 every time

`restore_scene_inputs` ends with `reload_tweens()`, which rebuilds the entire
`TweenSet` from the document with `frame = 0` — after `register_tween` has
already upserted each restored tween (making that call redundant). In
multi-row performance mode every Scene advance resets **all** tweens —
including the preset-level ones on rows that stay — so linked props snap to
their start values on each transition.
**Fix:** drop the trailing reload (the per-entry `register_tween` already
syncs the runtime), or make reload preserve playheads for id-stable
descriptors.

### R-9 `crates/modv-app/src/store_ops.rs:1412` ✔
`reload_tweens` discards all tweens on one bad descriptor

Any decode error fails `TweenSet::load_from_json` → the whole runtime becomes
empty, while the offending descriptor stays in `state.tweens`, so every
subsequent Scene apply re-wipes. JS creates per-tween and lets the rest
survive.
**Fix:** decode per entry; skip-and-warn failures (mirrors the unknown-easing
posture).

### R-10 `crates/modv-app/src/scene_preview.rs:273` + `compositor.rs:1948` ✔
“Warm” hover previews thrash the GPU instances they supposedly keep

Two cooperating defects:
(a) Every `render_frame` calls `prune(state)`; the preview state contains only
the cell being rendered, so **every render of cell B disposes cell A's shader
slots, 2D instances, Point10 scenes and ISF pipelines** (and `reset()` zeroes
the whole private compositor on LRU eviction, including the up-to-3 surviving
slots). Alternating a hover across cached cells pays full module re-init —
including Point10 scene init — every poll.
(b) `create_slot` eviction calls `compositor.reset()` on a shared private
compositor, invalidating the instances of all cached cells (state survives,
GPU objects do not).
**Fix:** make the private compositor prune scoped to the instances of the
slot being rendered — e.g. keep a `previews.scene_ids` set and prune by it, or
give each slot its own isolated device scope; at minimum, eviction should
dispose only the evicted cell's instances (they are keyed by fresh ids).

### R-11 `src/renderer/src/application/index.js:288` ✔
`_applyEngineSync` drops `tweens` and `expressions` from `state/sync`

The engine ships the preset document verbatim inside sync (verified
`store_ops.rs:1716`): `tweens` descriptors and `expressions` are in it. The
JS replay handles groups/modules/`registered`/inputs/links/scenes/performance
and seeding, but nothing replays tweens or expressions. Any tween present in
the engine at renderer boot (or after a snapshot restore) never reaches the
mirror: tween-typed props read `undefined`, `dataTypes/tween` paths break,
presets with expressions replay only partially.
**Fix:** in `_applyEngineSync`, commit an `inputs/ADD_TWEEN`-equivalent per
snapshot tween (or a `tweens/REPLACE_TWEENS` mutation) and replay
`expressions` assignments likewise.

### R-12 `src/main/engine-ipc.js:110-135` ✔
`engine/preview` hands uninitialised memory to the renderer; no seqlock

Still at HEAD: `Buffer.allocUnsafe(length)` allocated before the read
(ignored `readSync` bytesRead — a short read ships uninitialised pages into
`putImageData`), no `slot < info.slots` bound, and no magic/sequence check —
a mid-publish slot read yields a torn half-old/half-new frame even for
legitimate engine behaviour. The Rust ring (`modv-ipc/src/preview.rs`) has a
correct seqlock; the renderer must mirror it: validate magic, read sequence
(even), read pixels, re-read sequence, retry on change; then clamp `length`
by `info` and check `bytesRead`.

### R-13 `crates/modv-app/src/scene_ops.rs:1233` ✔
`scenes/removeScene` does not finish in-flight crossfades first

The JS completes transitions and row transitions before deleting Groups
(`scenes.js:2818-2819`); the engine removes Scene-stamped Groups (including
ghosts) directly. The performance runtime then holds `row_transitions` /
`transition` referencing deleted Groups, `tick_scenes` calls `set_group_alpha`
/ `complete_row_transition` on dead ids, and emits deltas for them; dwell for
that row stalls until the fade would have finished.
**Fix:** call `complete_all_row_transitions()` and
`complete_whole_scene_transition()` at the top of `remove_scene_action`, and
re-seed `performance.rows`/`SET_ACTIVE_ROWS` from the post-removal state.

### R-14 `crates/modv-engine/src/default_scene_migrations.rs:328` ✔
`apply_removal` deletes **all** same-named Scenes, against its own contract

`scenes.retain(...)` removes every Scene with the retired name; the surviving
“a same-named user Scene survives” comment (L348) and `seeded_names` check do
not prevent it. The production path (`default_scenes.rs:505`, mirror of the JS
`seen` set) removes at most one match. A user Scene that happens to share a
retired default name is silently deleted the moment the first real migration
rule lands in `DEFAULT_SCENE_MIGRATIONS` (currently `&[]`). **Data-loss
class.**
**Fix:** remove only the first (seeded) match, or carry seeded Scene **ids** in
the migration records; make doc, comment and code agree.

### R-15 `crates/modv-engine/src/scene_capture.rs:125,161` ✔
Unbounded allocation from hostile/corrupt `sceneRow`

`vec![None; max_row + 1]` (and `rows.resize(row + 1, None)`) size directly
from `Group.scene_row`, which comes from JSON. A snapshot with
`"sceneRow": 4294967295` forces a >100 GB allocation (abort); `usize::MAX`
panics in debug builds. The JS used sparse arrays and never allocated.
**Fix:** build the rows map as `BTreeMap` and materialise only the trailing
hole up to the last index, or clamp `max_row`/`row` to a grid bound
(`get_row_count`/`MAX_SCENE_ROWS`).

### R-16 `crates/modv-render/src/frame_graph.rs:573,721,926` ◐
Composite target not validated for `COPY_DST`

`run_composite` checks `RENDER_ATTACHMENT | TEXTURE_BINDING` on the dst, then
emits `copy_texture_to_texture` — requiring `COPY_DST`. In-tree callers
happen to include it, but the validator should; a bad caller becomes an
uncaptured wgpu validation error (which panics in the default handler).
Include `COPY_DST` in the required-usage check.

### R-17 `crates/modv-renderers/src/two_d/modules/{grid_stretch,pixelate,buffer}.rs` + `canvas.rs:637` ◐
Self-sampling 2D modules read the live canvas, not a snapshot

`GridStretch`/`Pixelate`/`Buffer` bind the live module-canvas view as an
external texture; per the logging of `Canvas::submit` (snapshot-split), that
is the texture being written — reading it with different content is
undefined; today it yields one-frame-lagged feedback plus undefined pixels on
the first frame / after resize (fresh texture, never zero-initialised). Also:
`Canvas::clip()` **replaces** the clip stack item instead of intersecting
(Canvas2D `clip()` intersects); and full-viewport `clearRect` drops earlier
same-frame commands via `clear_all()` instead of erasing (`canvas.rs:522,1175`).
**Fix:** route all three through `draw_image_self*` (like `Smear`),
zero-initialise canvas textures on creation, intersect clip on `clip()`, and
implement full-canvas `clearRect` as a transparent-black `Replace` draw.

### R-18 `crates/modv-media/src/osc.rs:57,303,373` ◐
OSC store is network-controlled blow-up surface

`MAX_ARRAY_INDEX = 1<<20`; a single datagram `/layers/1048575` materialises a
~1M-entry `Value::Array` (~25 MB); a flood of distinct addresses grows the
persisted JSON store without bound (binds 0.0.0.0:3333, like node-osc). Bundle
recursion in `dispatch_packet` is bounded only by datagram size.
**Fix:** cap array index growth (e.g. 64k), cap store size / per-key counts,
and cap bundle depth (8) with rejection beyond it.

### R-19 `crates/modv-media/src/tempo.rs:158` ◐
Unclamped `analysis_rate` / `history_seconds` → inf durations and OOM vectors

`frame_duration = 1.0/rate` is `inf`/`NaN` for rate ≤ 0; `history_frames`
becomes an unbounded `vec![0.0; …]`. Public constructor — clamp rate to a sane
range (as the other options already are).

### R-20 `crates/modv-ipc/src/socket.rs:125,334` ◐
Control socket unconditionally unlinks its path; mode not hardened

`bind()` does `fs::remove_file(path)` unchecked — it deletes whatever file is
there (any file, not just a stale socket), and `Drop` unlinks even if another
process recreated the path meanwhile. Mode is umask-default (world-accessible
under umask 000), and `engine/shutdown` over that socket exits the process.
**Fix:** unlink only when `connect` fails with ECONNREFUSED and the path is a
socket-node; `chmod 0600` after bind; same before-unlink check on `Drop`.

### R-21 `crates/modv-ipc/src/preview.rs:206,149,303` ◐
Preview ring `/dev/shm/modv-preview-*` world-readable; zero-slot ambiguity

(a) The VMP1 ring is created with umask mode — preview frames (fullscreen
capture contents) are readable by any local user. `chmod 0600` or `memfd`.
(b) Slots are initialised sequence=0 (even, “valid”) — a never-written slot
reads back as a black frame indistinguishable from a real one; init `sequence`
to odd (1) with an invalid magic, or track written-ness.
(c) A failure on the final header write leaves an odd sequence in the slot
forever; poison it so readers skip it deterministically.

### R-22 `crates/modv-media/src/audio/capture.rs:457` ◐
Per-callback allocation on the audio thread

`convert_buffer` news a `Vec` per device callback — GC/alloc
failure on the real-time thread can abort the process. Keep a scratch buffer
owned by the closure (same in PipeWire `process`).

### R-23 `src/renderer/src/application/engine-client.js:199` ✔ (read at HEAD)
Engine-mode UI silently black-holes what the engine doesn't implement

`osc/createServer`, `midi/ADD_DEVICE`, `midi/WRITE_DATA`, `windows/ADD_WINDOW`,
fonts enumeration, etc. hit the engine's `other => Ok(Null)` catch-all
(`store_ops.rs:371`). Consequences the user sees: MIDI devices never appear
(all MIDI input dies at `handleInput`'s `!device` guard), the **Output
Window feature is a silent no-op**, and OSC is inert — with zero feedback
beyond one warn-once line in the engine log.
**Fix:** at minimum surface "not supported by the engine" in the UI for these
(status toast / disabled item with a tooltip); ideally return a typed
`engine/unsupported` error the dispatcher renders.

### R-24 `src/main/engine-supervisor.js:276` ✔ (read at HEAD)
Spawn-async-failure + restart-budget accounting

`child.on("error")` only logs; a fast-failing spawn (ENOENT / EACCES /
not-executable packaged `modv-engine`) sits in "starting" until the ~100×
connect retry fails (~5–20 s) and routes through `_onExit` → restart budget.
Connect-timeout restarts also overlap a dying engine (SIGTERM → ~500 ms later
respawn + `unlinkSync` of the same socket path while the old engine still
holds it), and `restartTimestamps` is never reset on `shutdown()`/`start()`,
so a clean stop/start cycle inherits a stale crash budget.
**Fix:** route `child.on("error")` into the same `_scheduleRestart` decision;
await exit before respawn; reset `restartTimestamps` on clean stop/start;
document the overlap.

### R-25 `src/preload/index.js:11` + `src/main/engine-ipc.js` ✔ (read at HEAD)
Engine API exposed to every window; preview read path unvalidated

Every window (splash, output, color picker, output windows) gets
`dispatch/commit/snapshot/requestModulePreview/requestScenePreview`, far wider
than any of them needs. Separately, see R-12: no bytesRead check, no slot
bound, no seqlock. And `ENGINE_IPC_CHANNELS` is exported as "for destroy()
symmetry" but wired to nothing — either consume it in `destroy()` or delete it.

### R-26 `crates/modv-renderers/src/isf/source.rs:1151,409,802,710` + `runtime.rs:135,680` + `translate.rs:275` ◐
ISF robustness/edges

- `evaluate_size` has no upper clamp: `"WIDTH": "1073741824"` requests
  gigapixel textures → wgpu validation error → module silently dead every
  frame. Clamp to `max_texture_dimension_2d`.
- `RESERVED_INPUT_NAMES` contains GLSL control keywords; `rewrite_fragment_body`
  then `replace_ident`s **every** occurrence of `for`/`if`/`while`…, so a
  spec-legal ISF input named `return` makes translation unconditionally fail.
- `replace_ident(out, "texture2DRect", "texture")` maps pixel-coordinate
  samplers onto normalised-coordinate `texture()` — silent wrong sampling for
  ISF 1.0-era shaders.
- Overrides key off the **file stem** (`Soft Blur.fs` → `Soft Blur.wgsl`)
  with no content hash; any user media shader of that exact name inherits
  corpus overrides.
- `if !wgsl.contains("fn main")` is a substring match; a comment containing
  `// fn main` defeats it, and overrides that violate the `IsfUniforms`
  binding contract are never validated → unhandled wgpu errors, black module.
- Comment/code claim `FRAMEINDEX` starts at 1 vs the JS's 0 (off-by-one drift,
  `runtime.rs:135`).
- Size-formula substitution supports `$WIDTH/$HEIGHT/$<input>` only — the JS
  substitutes every uniform (`$TIME`, `$FRAMEINDEX`, `$DATE`, `$PASSINDEX`
  now error → module dead forever).
- `bool|event` arm: numeric `1` → `IsfValue::Bool(false)` (JS `!!1 == true`).
- `count_declarators` charges 1 location per declarator; `mat3/mat4` consume
  2 each → vertex sidecars can mis-number locations (report-only impact today).

### R-27 ISF corpus claim hygiene (`docs/isf-corpus-report.md`, `docs/rust-hybrid-progress.md`) ◐ — fix in flight

The corpus gate is `cargo test`, which (a) asserts only `translated > 0`, and
never asserts the 98.0 % bar, and (b) **rewrites the committed report on every
run** (`translate.rs:36,1002`), so the number in docs is machine/naga-version
dependent and silent regressions are invisible. Counts of `PERSISTENT` (43 vs
42, notes stale) and `FLOAT` (16 vs 15) in `docs/isf-translation-notes.md` are
stale. Also 7 override shaders are announced but the engine ships none
(`cargo build` does not include `crates/modv-renderers/src/isf/overrides`; the
`electron-builder.yml` `isf` resource ships only `resources/isf`), so in a
packaged build those shaders get **no overrides at all**.
**Fix:** make the corpus test assert the rate (≥95 %) against a checked-in
manifest; stop the test rewriting the docs (diff instead); ship overrides in
`extraResources` or embed them with `include_str!`.
*Note: the working tree currently carries an in-flight rewrite of this path
(`EMBEDDED_OVERRIDES` + `data/isf-overrides.json` fingerprint manifest +
`ShaderOrigin::Corpus` guard, +870 lines in `translate.rs`) which addresses
exactly this and the R-26 stem-collision issue — review once committed.*

### R-28 Point10 parity drift (`crates/modv-renderers/src/point10/**`) ◐

- `scenes/dvd_room.rs:1024`: glass opacity driven by `smoke_bass`; the JS
  uses the DVD-Room lane `smBass = band(1, 9)` (`DvdRoom.js:445`). Diverges
  whenever bin 0 is loud.
- `scenes/mirror_cube.rs:719`: `perm` driven by `band(-1, 1, 1)` bass instead
  of the `p9audio.beat(0.1)` kick envelope the JS uses.
- `scenes/it_is_coming_for_you.rs`: `preserve_bloom_alpha(true)` set — that's
  the Boxy/Wormhole semantic; `ItIsComingForYou` does **not** call it in JS,
  so bloomed pixels lose the alpha boost the JS adds (`ItIsComingForYou.js:250`).
- `scenes/it_is_coming_for_you.rs:440`, `chromatic/wave.rs:313`: `self.time`
  advances with uncapped `dt` (every other scene uses capped `frame_dt`); a
  catch-up frame jumps the clock.
- `scenes/cube_mesh.rs:380`: `perspective()` emits GL clip-z (near→−1) where
  wgpu needs [0,1]; everything renders today (near ≈ 0.1, camera units away)
  but it is not three.js's camera; `cube_mesh.rs:42` also contradicts
  `sound_cube.rs`'s actual depth state.
- `shared/assets.rs:283` (P2): small mip levels uploaded with
  `bytes_per_row = width*4` violate WebGPU's 256-byte `bytesPerRow` alignment;
  minified sampling reads uninitialised mips and every scene init logs a
  validation error. Pad the stride or stop the chain early.

### R-29 Scene/performance bookkeeping carried into restores ◐

- `store_ops.rs` `restore`: `performance.rows`, `row_transitions`, `transition`,
  `applied_scene_id` survive a restore that deleted the referenced Groups;
  deltas for dead ids flow until fades converge; `SET_ACTIVE_ROWS` is never
  re-emitted. Fix: clear performance runtime state in `restore()` and
  re-emit.
- `scene_ops.rs:681,1356`: whole-scene Fade keeps the **outgoing** Scene's
  Groups on top (created Groups sort before ghosts in `(scene_row, ghost)`
  order); the JS keeps the created Groups last → incoming draws on top.
- `performance.rs:606` + `Vec<String>` rows passed by clone: cosmetic.
- `compositor.rs` whole-scene Fade … (see R-13).

### R-30 ISF value coercion: `bool|event` arms ◐

Numeric `1` → `IsfValue::Bool(false)` (`as_bool().unwrap_or(false)`);
JS `!!1 === true`. Live events/bools with truthy numeric input read false
silently (events are never triggered, so impact limited to bools).

## P3 — minor and hygiene

- `engine_state.rs:419` ✔ (read at HEAD) — `order[..current_render_pos]` can
  slice out of bounds (sibling branch uses `.get()`); make both defensive.
- `preset.rs:36,224` ✔ — Scene `null` group holes are rejected by
  `parse_preset` (Scene files can't open as presets; JS manufactures empty
  Groups). Deliberate per code comment; document as user-facing behaviour or
  add a row-faithful path.
- `engine_state.rs:110` ✔ — `scene_row: Option<usize>` makes the whole
  snapshot undecodable on a single `-1`/`2.9` (JS truncates). Tolerate on the
  restore path.
- `scene_timing.rs:71` ✔ — `to_finite_number` rejects JS-legal `0x..` hex
  timing strings; add hex support or document.
- `two_d/canvas.rs:826` — `stroke_text` re-uploads the **same** mask ~13
  times per frame (12 stamps × new R8 texture); upload once, share the view.
- `restore()` — `flush_link_deltas()` output never broadcast before shutdown;
  engine exits with pending mirror deltas (`window.rs:56`, `store_ops.rs:1492`).
- `window.rs:179` — `EngineSupervisor::new` failure with no window parks the
  event loop forever (`resumed` returns early; `run()` never reaches the
  failed check); the Electron watchdog is the only exit. Call `event_loop.exit()`.
- `window.rs:340` — `Occluded` → redraw re-arm under `Poll` busy-spins a
  minimised engine; back off.
- `window.rs:340` — (same theme) `REDUNDANT_SUPERVISOR…` n/a.
- `module_registry.rs:1020` ◐ — `registered` prop descriptors drop `label`,
  `enum_members`, `step`, `component`, `VALUES/LABELS` → engine-mode enum
  props (Buffer/Texture2D/Waveform; ISF ENUMs) get empty dropdowns; props
  lose their labels.
- `store_ops.rs:1030` — `beats/SET_BPM` family emits **no deltas** (add/remove
  tween, fps/SET_FPS); mirror's tween map drifts until resync. Also
  `create_tween` reports success when the upsert failed.
- `store_ops.rs:1318` ◐ — `registered` map is rebuilt per reconnect (~440
  entries incl. ISF corpus meta); cache it in the `Store`. And test constant
  still says `37 + 15 + 18` (Point10 is 21).
- `store_ops.rs` / `input_links.rs:152` ◐ — sourceless `meyda/*` getters keep
  the disabled-Module gate (deliberate; JS does not gate). Warn once when a
  link is skipped for this reason, like the easing fallback.
- `input_links.rs` ◐ — client-supplied tween `id` with `'` breaks
  `parse_tween_link_location` silently; validate/escape ids at ingress.
- `scene_ops.rs:306` ◐ — `preview/scene` deep-clones the Scene +
  `row_names` per request including warm cache hits; clone on miss only.
- `modv-ipc/Cargo.toml:10` ◐ — `memmap2` declared, never used; drop it.
- `preview.rs:206` — ring file world-readable (see R-21); `preview.rs:149` —
  never-written slots read as valid black frames; `preview.rs:303` — failed
  final header write strands an odd-sequence slot; init `sequence` to odd.
- `analyser.rs:829-907` ◐ — the “1 % parity” analyser fixture compares 0 ≈
  0 over ~80 % of asserted lanes/cells (tone is 60 Hz + 6 kHz; mid/treble
  expected ~0). It validates bass only; regenerate against browser-measured
  capture or widen tone content past 2 kHz; also the dead
  `frame_dt_is_zeroed…` test asserts only `is_some()`; also `beat_detektor`
  infinite seed when `dt == 0` forever.
- `osc.rs:303` ◐ — bundle recursion unbounded; cap depth (see R-18).
- `midi.rs:281` ◐ — device key with `.`/`/` aliases lodash path traversal
  (renderer-local; keep a key→index map or escape).
- `ndi.rs:460,771` ◐ — SDK not `NDIlib_destroy`ed at exit; `send_rgba(&self)`
  has no internal synchronisation (document single-threaded or take `&mut`).
- `analyser` DT == 0 beat-detektor seed (see above); `pool.rs:199` ◐ —
  zero-size pool key → zero `bytes_per_row` write (wgpu panic); clamp or reject.
- `blend.rs:557` / `pipeline.rs:670` ◐ — two tests pin trivial contracts;
  replace with behavioural ones (golden rows exist elsewhere — good).
- Tween runtime: `ease()` infinite-loops for `t < -0.09` in `easeInBounce`
  (`pub` fn outside clamp domain); `steps(4, 2)` drops the second arg; anime
  multi-arg families (`easeOutElastic(1, .5)`, `spring(…)`, `cubicBezier(…)`)
  silently become `linear` though the comment claims the JS throws;
  `update_bpm` writes `Infinity` duration for bpm 0 (see R-5); easings
  dispatcher ignores anime's arg parsing (`tweens.rs:507`); the
  `tween-reference.json` fixture is genuine (independently re-verified) but
  has no generator, no completeness gate (`PENNER_EASING_NAMES.len()==41`
  only), and is validated only by `cargo test`.
- `tweens.rs` ◐ — `state.tweens` keeps test-prefixed `frames` keys? No
  — `frames` are excluded deliberately (JS parity). OK.
- Point10: `attractor_particles` camera `radius = 9.9` drifts from JS
  `distanceTo(centroid)` ≈ 9.68 (test pins 9.9 — undocumented deviation);
  `camera_distance` write-only field (dead store); dead uniform lanes
  (`logo`/`glass` `params.z/w`) — harmless upload traffic; `boxy` nomusic
  `0.0` vs JS `0.04` creeps slower in silence; HollowCube silence creep
  ~11× JS (deliberate, documented in module header);
  `capability`… `_rand` reversed-edge smoothsteps fine.
- `it_is_coming_for_you` additive bloom + `preserve_bloom_alpha` (see R-28).
- `scene_tests.rs:617` ◐ — doc comment says “SmokeRoom” on the DvdRoom test.
- Scenes.vue ◐ — engine hover preview: exception path in
  `pullEngineScenePreview` (try/finally, no catch) → unhandled rejection +
  30 fps re-arm loop; Parity spec never exercises `preview/scene` end-to-end;
  `is-modv-ready` latch is ordering-dependent, not a sync gate.
- `engine-client.js` — `scenePreviewFromResponse` comment/code drift
  (accepts oversized padded buffers); `onDelta/onState/onStateSync` drop
  previous unsubscribes instead of calling them (R-25 siblings).
- `Scenes.vue` — repeated hover churn re-uploads full RGBA per cell at
  30 fps; backoff is per-cell; same cadence as legacy but each poll is now a
  real isolated-compositor render through IPC — readme-worthy.
- `docs/rust-hybrid-progress.md:139,134` ◐ — stale “hover previews in flight”
  / “migration in flight” lines under their completed `[x]` counterparts;
  “Point10 batch 5a — 21 of 25” docs vs `DEFAULT_SCENE_MIGRATIONS = &[]`
  (AGENTS.md now documents the rewriting toolchain — that is unreleased
  working-tree state, treat as such).

## P2 — parity breaks and robustness (continued, Round 3)

### R-32 `crates/modv-renderers/src/point10/scenes/smoke_room.rs:1137` ◐
SmokeRoom alpha compositing order is inverted vs the JS

Panes render before the opaque lava sphere; the sphere (blend None) then overwrites the pane pixels, erasing the near-pane fade/fresnel in front of it. `SmokeRoom.js` marks the glass `transparent, opacity 0.35, depthWrite: false` and three renders opaque first, so panes blend over the sphere.
**Fix:** draw the opaque sphere first, then the transparent panes (or set the sphere transparent pass ordering deliberately and document it).

### R-33 `crates/modv-renderers/src/point10/scenes/smoke_room.rs:1043,647` ◐
Lava sphere ignores `boxScale`

JS parents the sphere under `_boxGroup` and calls `_boxGroup.scale.setScalar(scale)`, so the sphere radius is `0.9 × boxScale`; the port scales the glass and puff spawn radius but draws the sphere at identity — with `boxScale ≠ 1` the fire spawns from an invisible larger sphere than the one rendered.
**Fix:** apply `scale = boxScale` to the sphere model (or reuse `glass_model`'s scale factor).
### R-34 `crates/modv-renderers/src/point10/scenes/rolling_fire.rs:1332` ◐
No-audio path freezes the fire sim

`update()` returns before the puff loop/camera lerp/ball roll when `!audio.is_available()`; the JS `_animate()` keeps the sim running, and the sibling SmokeRoom (`smoke_room.rs:1092`) advances puffs unconditionally. A host with no audio device gets a static floor and sphere instead of idling fire — and the two WS-5/WS-7 siblings diverge.

## P2 — parity breaks and robustness (continued, Round 4)

### R-35 `crates/modv-app/src/compositor.rs:978` ◐
GPU-pass failure shows one unblended frame before the Fade fallback kicks in

When `apply_scene_transition` returns `false` (e.g. pipeline rejected), only
`transition_from` is dropped — the store transition stays active, so for one
frame the incoming Scene composites full-frame with the outgoing Groups
hidden (no mix at all), and the downgrade lands only on the next frame's
window-loop (`window.rs:441`). The comment at `compositor.rs:976` promises a
"falls back to the Fade ghost path next frame" — it is not the same frame,
and the WARN fires from the capture-missing branch, not from the
pass-failure branch, so the log is misattributed.
**Fix:** downgrade the store transition in the same frame (or at minimum log
at the failure site with the real reason).

### R-36 `crates/modv-app/src/performance.rs:918` ◐
Fade fallback resets alphas to frame-0 values mid-transition

`fall_back_scene_transition_to_fade` snaps incoming-group alphas to `0.0` and
outgoing to the recorded alpha **regardless of current progress**. The JS
(`scenes.js:3750-3758`) only clears `transition.shader` and lets the next tick
compute `to.alpha*eased` / `from.alpha*(1-eased)` from the current eased
progress — no snap. Resizing (or any `apply_scene_transition` failure) at
90 % of a 2 s transition visibly snaps the incoming Scene off. The frame-0
capture-failure path coincides with the JS behaviour, which masks this in
tests.
**Fix:** read the current eased progress and lerp alphas from it.

## Round 4 errata on earlier rounds

- Shader whole-Scene transitions (R-37..R-42 below) landed in `36672a33`;
  **R-13 (removeScene not completing transitions) is unchanged** — the delta
  touched `scene_ops.rs` only around the apply paths (`apply_scene_action`,
  `apply_scene_row_now`), not `remove_scene_action`.
- **R-29 (Fade z-order: outgoing on top of incoming) is confirmed by the
  transition work too** — `apply_scene_transition` pushes the hidden `from`
  groups and `emit_group_order` stably re-sorts them after their incoming
  twins of the same row; for the shader path they are hidden so it is
  invisible, for the Fade fallback path it is still the wrong order vs the
  JS `ADD_GROUP` push semantics.
- `docs/rust-hybrid-progress.md` now carries **428** as the module count?
  No — verified lines 24 and 41 of `36672a33`: both still say **424**
  (registered modules, engineParity smoke text). Stale in the newest docs.
- R-31 (tunnel hoops) and R-32..R-34 are unaffected by this delta.

## P3 — minor and hygiene (continued, Round 4)

- **Frozen outgoing frame (documented, not a bug report)** ◐ — the Rust port
  captures the outgoing Scene once at transition start and renders a hidden
  static frame; the JS re-renders the outgoing Groups every frame, so active
  outgoing scenes keep animating in JS. Also a deliberate full-output-size
  pass instead of the JS 1920×1080 cap. Both are documented as "behaviour
  parity, not pixel parity" in the progress doc — worth restating here so
  reviewers don't treat the freeze as a regression.
- **`scene_transition.rs:313` — `Circle` never fully closes at aspect ≳ 2.3**
  ◐ — `radius = smoothstep(p)*1.2` with a soft edge spanning 1.12..1.28 cannot
  cover ≥21:9 corner distances at progress 1, so the outgoing corners linger
  then snap away at completion. Faithful port of the ISF constant (parity is
  the priority) but the completion pop will be visible on ultrawide.
- **`scene_transition.rs:176` — `to_ne_bytes`** ◐ — uniform payload uses
  native-endian bytes; declaration says little-endian. No-op on x86/aarch64,
  garbage on BE hosts. Use `to_le_bytes` (and fix the test's `from_ne_bytes`).
- **Neither transition fallback branch is tested end-to-end** ◐ — the
  capture-failure→Fade and resize-mid-transition→Fade glue in `window.rs`
  (precisely where R-35/R-36 live) has no unit or e2e coverage; the
  store-level `fall_back_scene_transition_to_fade` *is* unit-tested, and GPU
  pixel tests cover mix/capture lifecycle, but window↔store↔compositor are
  never joined in a test.
- **`scene_transition.rs:422,548` — pipeline-build "failure" path is
  unreachable defence-in-depth** ◐ — `create_shader_module`/pipeline
  validation failures go to wgpu's default error handler (panic), not
  `Option::None`; the `failed[]` cache cannot trigger via that route. Same
  for OOM in `capture_transition_source` (`create_output_texture`). The
  documented downgrade safety-net on those paths is unattainable by this
  route. (Consistent with the rest of the compositor; not a regression.)
- **`performance.rs:226` / `scene_transition.rs:151` — checked, fine** ◐ —
  `progress()` is total; the timeline uses the *raw* delta (as JS
  `transition.elapsed += rawDt`), so even a stalled frame completes the
  transition in one tick; `clamp_progress` NaN→0 is a deliberate improvement
  over JS `Math.min/max`.
- **Completed: `remove_scene_action` and non-apply destructive ops still do
  not call `complete_whole_scene_transition()`** — see R-13; unchanged.

## P3 — minor and hygiene (continued, Round 3)

- **Engine-mode registry is missing three legacy wrappers** ◐ — `Point10 Ball Of Fire` (`BallOfFire`, a `RollingFire` subclass with `_freezeBall`), `Point10 Earth`, `Point10 Sun` exist in the legacy adapter (31 Point10 wrapper modules in `renderers/2d/modules/`) but have no Rust scene and no `Point10SceneKind`/registry entry (25 registered). Two shipped default Scenes, **“Solar Flare” and “Terminal Bloom”**, embed `"Point10 Ball Of Fire"` (verified in the committed `crates/modv-engine/data/default-scenes.json` and `default-scenes.js`) — in engine mode those layers silently never load (one `failed_point10` log). Register BallOfFire as an alias of `RollingFire` (or port it), and Earth/Sun or drop them from the documented inventory; also correct the "all 25 scenes registered" claim (25 of the WS-7 list, 28 of the legacy modules). Note: `"Point10 Crt"` and `"Point10 Old Movie"` — also referenced by shipped default Scenes — **are** registered in the engine (`point10_effects.rs:447,720`); the gap really is just Ball Of Fire (defaults) and Earth/Sun (inventory).
- **`electron-builder.yml` `deb:` declares no `libpipewire-0.3-0`** ✔ — `modv-media` links `pipewire 0.9.2` unconditionally, so the packaged engine fails to `dlopen` on hosts without the library — *regardless of `--audio-source`* — and burns the supervisor's restart budget. The AGENTS.md note "the `.deb` picks it up from the distro" is not true (deb depends list absent → electron-builder defaults only). Fix: `deb.depends: ["default", "libpipewire-0.3-0"]` or dlopen like grandiose. (Default installs unaffected — engine is opt-in.)
- **PipeWire mid-stream death is silent** ◐ — no state/error listener calls `queue.set_error()`, so `take_error()` (one-shot, verified `capture.rs:192`) never fires and `window.rs:255-284`'s renderer-pump fallback is never reached: the engine renders silence with no log or fallback once the stream dies after startup.
- **Engine-owned audio vs UI meter: mixed sources** ◐ — the engine republishes bands at 10 Hz, but the renderer still commits its own 60 Hz `meyda/SET_BANDS` to the mirror (`application/index.js:843`), so “republishes UI meter bands at 10 Hz” is true on the wire only; the mirror is a mixed source. Pick one owner or document the interleave.
- **`window.rs:67,~556`** ◐ — final `flush_link_deltas()` is never broadcast (dead code; comments in `store_ops.rs:1859` claim the opposite); the 1 Hz `size/SET_SIZE` republish hard-codes `dpr: 1.0`, clobbering the real `scale_factor` pushed on `Resized` (wrong mirror DPR on HiDPI almost permanently); `surface_size` is never seeded from the initial surface (status bar stays 0×0 if winit never emits an initial `Resized`).
- **`store_ops.rs:1956`** ◐ — `restore()` drains deltas but leaves `engine_meter.pending` (stale pre-restore meter payload can be pushed later; `disable_engine_audio()` clears it properly).
- **Engine-owned capture has zero automated coverage** ◐ — `pump_engine_audio`/`disable_engine_audio` (`window.rs:255-298`) and the startup fallback are tested nowhere; `engineParity.spec.js` injects synthetic `meyda/SET_BANDS`, which a host with `MODV_AUDIO_SOURCE` set would have the engine ignore — the spec's Input-Link section is not hermetic.
- **`supervisor.rs:61`** ◐ — `MAX_PENDING_PREVIEWS` silently bumped 32→512 with no comment/test; still bounded and harmless, but the rationale (Gallery-full-of-tiles steady state) belongs in the source.
- **Docs arithmetic** ◐ — `docs/rust-hybrid-progress.md` still says **424** registered modules ("37 shader + 15 2D + 21 Point10 + 351 ISF") in two places; it is 428 with 25 Point10 scenes. Header also claims "M4 complete (25 of 25)" while a batch-5b checkbox remains unchecked — doc and code disagree in both directions.
- **`geometric_tunnel.rs:1548`** ◐ — dust X-rotation reads the already-reassigned `y` (should use the pre-rotation value); nearly invisible (|angle| ≤ 0.04) but wrong. **`:1673`** — billboard `forward` targets the origin, not the look-at target `(0,0,-TUNNEL_LEN/2)` (~6° tilt on dust masks; bolts unaffected). **`:2348` vs `:2425`** — `update()` truncates instances to MAX_INSTANCES(512) but `render()` draws by buffer slices using untruncated lengths (unreachable today at 182; latent validation error).
- **Afterimage wiring is partly dead** ◐ — `Composer::enable_/disable_/afterimage` (`postfx/composer.rs`) is unused; `LatentBackrooms` drives its own `AfterimagePass`. Also: latent_backrooms re-creates `grade` bind group **every frame** (`latent_backrooms.rs:1692`) — necessary because the afterimage history view alternates, but cacheable as a 2-state set; and no `onActivate`/`reset` hook exists in the Rust scene contract (JS clears the trail on activation; only fresh module ids per Scene apply protect the trail today). The Afterimage history targets use `linear_sampler` where three's AfterimagePass uses `NearestFilter` (texel-centre UVs make it invisible, but it is a silent substitution from the "bit-exact" claim).
- **GeometricTunnel depth culling dropped** ◐ — bolts and dust draw with `depth_stencil: None`; the JS Line2 bolts and Points dust are depth-tested against hoops. Undocumented; additive and low-contrast so likely benign, but it is a deviation.
- **SSAO/Afterimage/`TUNNEL` bounds** ◐ — otherwise the new postfx match three's r187 semantics (SSAO params `TUNNEL_SSAO` ported exactly; afterimage blend `max(new, damped)` correct; `damp` clamped 0..1 NaN-safe). Uniform sizes asserted in tests.
- **`cli.rs:364-410,109`** ◐ — `--audio-device` with `--audio-source renderer` is silently ignored; `pipewire:<target>` suffix accepted-and-tested but absent from `--audio-source` help; engine-owned capture is reachable only via `MODV_AUDIO_SOURCE`/flag env inheritance — `engine-supervisor.js` passes only `--socket/--state/--isf-dir`, so packaged users have no UI/launch path to it either.
- **`smoke_room.rs:655`** ◐ — emitter flicker hard-coded (`params = [time, 3.5, 1.0, 0.0]`) where JS drives `uFlicker` per frame; `params.y` (3.5) upload is unconsumed. **`hiStart` bin off-by-one** ◐ — both fire scenes start the hi-hat band at bin `floor(0.85*1024)=870`; JS `hiStart = 1024-floor(153.6) = 871`.

## Round 3 errata on earlier rounds

- Round 2 carried no geometric_tunnel finding — R-31 is new in this batch (the file itself is new).
- R-4 (cold-start `state/sync`) is **unchanged** — the sync wiring commit added the payload, not a late-joiner fallback.
- R-12, R-13, R-14, R-15 (all ✔-marked, still at HEAD of `c088335d`) — delta did not touch those files.
- Round 2's `module_registry` dead-code note is resolved — `registered_modules()` is now consumed by `sync_state`.

## Checked and clean (no findings — verified, not assumed)

- **Blend math** — all 28 modes, `soft_light` form, Porter-Duff table incl.
  `destination-out`/`xor`, HSL setLum/clipColor/setSat, `globalAlpha`
  premultiplication, W3C Compositing-1 identities; RowFade alpha port; MSAA
  2D blit chain; snapshot-split self-sampling (Smear); 160-byte uniform
  stride vs WGSL span; `vec4` array strides; bind-group/layout parity
  (10 fixed bindings + aliases).
- **State core** — `swap.rs` (4 passes, gallery preservation, `setDefault`
  semantics incl. JS truthiness quirks); `props.rs` clamp/abs/round incl. the
  `Math.round(-2.5)` case; `inputs.rs` (0-is-valid, min-only/max-only links →
  None); `engine_state.rs` round trip with `$status`/`drawToCanvasId` drops
  (documented); `delta.rs` minimal-JSON writer (2⁵³ fencepost); delta ordering
  (deltas before acks, sync before first delta).
- **Audio** — AGC bands/lanes/cells vs `audio-bands.js` (constants, floor,
  FFT_GAIN); tempo autocorrelation + octave resolution; BeatDetektor dual
  average, quantisation, `typeof(draft==undefined)` quirk reproduced;
  MIDI clock ring/24 PPQN/500 ms throttle vs `setup-midi.js`; Meyda extractor
  port (rect window, kurtosis/slope sign quirks reproduced) — maths correct
  as far as it can be checked without a browser; canvas 16→512 byte floor
  semantics vs `byte-frequency-data.js`.
- **NDI FFI** — all `extern "C"` signatures match SDK; recv→free ordering;
  dynamic loading paths; no dangling fn pointers; `#![forbid(unsafe_code)]`
  holds in modv-ipc.
- **Framegraph/executor** — main registered/unregistered around group passes
  so the only cleared `main` is the presented one; RowFade avoids
  attachment-sampling feedback; pool eviction is LRU for objects actually in
  use; `prune` frees group targets only for vanished ids.
- **VMP1 ring writer** — seqlock (odd/even bump), little-endian both sides,
  stride/height/len checks, `ispoweroftwo` guards — the one consumer gap is
  R-12 in JS.
- **Previews isolation** — fresh ids per slot (`scene-preview:{gen}:…`),
  private Compositor for previews, fresh `EngineState` per cell,
  `scene_id: None` self-sampler semantics, LRU accounting, 32/4 bounds,
  packets px-length-checked both ends; no wrong-client delivery (monotonic
  client ids); previews never dispatch or evaluate links.
- **Tween fixture** — all 41 penner easings re-derived independently (≤1e-12);
  `framesRequired`/`Math.round` semantics (negative durations → 1 frame,
  documented deviation); steps easing `ceil(minMax(t,1e-6,1)*n)/n`;
  read-then-advance link semantics incl. disabled-module gate restricted to
  `meyda` links (parity).
- **Point10** — audio bridge vs `audioBridge.js` (clamps incl. NaN→0);
  WaveSim vs `waterWave.js` (pinned to the JS Node reference inside tests);
  phrases/ROYGBIV state machine; GlassLens refraction direction algebra;
  SoundCube ping-pong order; MirrorCube protocol; UltraHDR drive functions;
  particle alpha map byte-equal; no per-frame panics; dispose/re-init parity
  (`every_scene_survives_dispose_and_reinit`, 21 scenes).
- **Supervisor/IPC** — framing caps, partial-read buffering, 64 MiB cap, dead
  reaper, restart budget tests genuinely integration-tested against a
  `fake-engine.mjs` speaking the real framing; SIGKILL path handled via
  `PR_SET_PDEATHSIG`.
- **Delta fixtures** — `engine-mirror.test.js` replays engine-emitted
  fixtures into the real Vuex mirror with deep `toEqual`; no vacuous asserts
  found in sampled tests; no `#[ignore]`/`skip` outside the GPU-adapter
  guards.
- **Default Scenes** — `DEFAULTS_VERSION=12` tables byte-identical to JS;
  embedded `default-scenes.json` validated (59 unique scenes); seed/remove/
  refresh semantics line-matched to `default-scenes.js`.

## Open questions for the engine author

1. Which of R-1..R-4 gate M7 (engine as default)? Suggest: all four, plus
   R-13/R-15, as a minimum "engine is safe to flip" bar.
2. Is the PipeWire capture path intended to ship at all in the packaged
   `--audio-source pipewire` mode (it links `libpipewire-0.3` dynamics, which
   the AppImage/.deb does not declare)? If yes, R-2/R-3 are blocking; if it's
   still experimental, the feature flag should say so.
3. The `Preview ring` should be created with an explicit mode at creation
   sites; is `chmod 0600` acceptable on `/dev/shm` files, or is `memfd_create`
   preferred?
4. `packaged engine + ISF overrides`: overrides are not shipped (R-27). Was
   that deliberate (size) or an oversight? If the latter, `extraResources`
   needs `crates/modv-renderers/src/isf/overrides`.
5. Scene hover previews are 30 fps `putImageData` of full RGBA through base64
   IPC (≈1.2 MB/s per hovering client at 320×180). Acceptable, or is the
   shared-memory ring the intended end state (same as main previews)?
6. Tween playheads on Scene apply (R-8): is resetting pre-existing tweens to
   frame 0 on every Scene advance the intended parity, or an accepted gap?

## Limitations of this review

- No code was compiled, tested, or run — this is a reading review with JS
  cross-checks. Rust test claims (1279 passing) were not re-run.
- Line numbers drift with each commit; `✔` findings were re-verified at the
  reviewed tip, `◐` findings are single-pass reads (verify before fixing).
- GPU-dependent behaviour (mip upload, clip-z, bloom alpha) verified by
  reading wgpu/three.js semantics, not by running.
- `crates/modv-engine/data/default-scenes.json` content was validated
  structurally by a reviewing agent, not by re-serialising against the JS
  source.
- Review rounds are appended below; see [Round log](#round-log) for exactly
  what was covered and when.

## Round log

| Round | Commit reviewed | What it added | Notes |
|-------|-----------------|---------------|-------|
| 1 | `e8d78f18` (worktree, as handed to the reviewer) | Entire Rust workspace, JS integration, tests, build files | Worktree deleted externally mid-review; findings re-verified against `3548101f` by reading; docs/build drift noted |
| 2 | `e8d78f18..3548101f` (delta, from `git archive`) | Tweens, Scene hover previews, engine sync wiring, Point10 batch 5a (21/25) | All P1s from round 1 re-verified still present; new tween and preview findings R-5..R-11 |
| 3 | `3548101f..c088335d` (delta) | Point10 batch 5b (25/25), `--audio-source` engine-owned capture wiring, installer plumbing | New P1 R-31 (tunnel hoop depth); R-32..R-34 parity findings; packaging gap (no libpipewire in deb depends); docs/tests coverage gaps |
| 4 | `c088335d..36672a33` (delta) | Shader whole-Scene transitions (crossfade/dissolve/pixelate/glitch/wipe/circle), progress docs, `M7 — in progress` status | No P1; new R-35..R-36 fallback-path defects; transition window glue untested; fallback-Fade alpha snap |

**In flight at the time of writing (uncommitted, not yet reviewed):** the
working tree carries fixes matching nearly every finding in this file —
`scene_capture.rs` (R-15), `default_scene_migrations.rs` (R-14), `socket.rs` /
`preview.rs` (R-20/R-21), `pipewire.rs` / `tempo.rs` / `osc.rs` / `capture.rs`
(R-2/R-3/R-19/R-18/R-22), `tweens.rs` (R-5/R-6/R-7), `scene_preview.rs` (R-10),
the ISF pipeline (R-1/R-26/R-27, incl. embedded overrides + fingerprint
manifest), `two_d` (R-17), Point10 (R-28, R-31..R-34), `frame_graph.rs`
(R-16), and the JS side (`engine-preview-read.js`, `engine-restart-budget.js`,
`engine-sync-replay.js`, `preload/engine-api-scope.js`, R-4/R-12/R-24/R-25).
They are listed here only so the reader knows a response is already
underway; they will be reviewed when committed.

<sub>Review file maintained incrementally; earlier passes are not retracted,
only extended. Last full pass: `36672a33` — 2026-11-12.</sub>
