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
  (latest pass: **`270a0003`** — see the [Round log](#round-log)). This is
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
and made this review productive. The author's response to this review (see
“Fix-wave verification — Round 5” below) was fast, broad and mostly
high-quality — R-1..R-30 landed, with residuals.

**Status:** as of `270a0003`, engine mode remains opt-in and should stay so
until the open items close. R-31, R-34, R-36 and the `.deb` PipeWire
dependency have landed and verified; the corpus validates 351/351; Ball
Of Fire resolves in the default Scenes. What blocks a default flip now:

1. **Expression evaluation has no wall-clock/allocation budget** (new, see
   Round 8) — a hostile shared playlist wedges the engine's event-loop
   thread (`evaluateEveryFrame` re-runs per frame); the watchdog/restart
   path turns it into a crash loop before the app goes `fatal`.
2. **R-2 residual** — mid-stream PipeWire death pins the show to silence
   with no log and no renderer-pump fallback (the module doc claims the
   opposite; engine-owned capture is a new user-facing path).
3. **R-35 (partial)** — the shader-transition pipeline-failure downgrade is
   correct on the prepare path but unreachable there (no wgpu error scopes);
   the apply-time failure path still shows one unblended frame with a late,
   misattributed WARN.
4. **Plugin-host P2s** — plugins can shadow core mutation types (mirror-only
   writes), async frame-hook rejections spray the unhandledrejection log
   at rAF cadence, and unsupported plugins get a dead silent checkbox.
5. **WS-12 docs contradict the shipped Boa evaluator** — progress doc /
   `rust-hybrid.md` still say expressions are not evaluated by the engine;
   they are (and carry a sandbox caveat that is only documented in code).
6. **WS-10 media end-to-end still broken today** — two P1s (stored media
   paths root at `/` so engine file sources never resolve on POSIX; the
   file-source lifetime is tied to the TextureControl component, killing
   playback on panel changes) plus `.deb` declares neither the GStreamer
   libs/plugins nor does the AppImage bundle them; MediaHub restarts
   non-looping files forever and blocks the render/heartbeat thread. The
   Webcam camera-index divergence was fixed in `28d90158`.
7. **Point10 volume re-port** — real fluid sim but it (a) deleted Rolling
   Fire and Geometric Tunnel render coverage, (b) loosened the SmokeRoom/
   DvdRoom transparency contracts from min-alpha bounds to a 10 % "some
   opaque" check, (c) always simulates the full 6.75 M-voxel grid (~432 MB
   `Rgba16Float`, no override), and (d) left my R-33/R-34 findings open.

The remaining conformance gap is not correctness but honesty and hardening:
the app is usable in engine mode, it just cannot fail closed.

## P1 — fix before anything else

### R-1 `crates/modv-renderers/src/isf/runtime.rs:838` ✔ — FIXED in `3754b249`
Trailing-implicit final passes were blitted over, breaking real ISF shaders

```rust
let last_target = passes.iter().rev().find_map(|pass| pass.target);
```

takes the last **targeted** pass. The JS sets `lastTarget = null` on every
implicit pass and only blits when the *final* pass has a target — an implicit
final pass is the destination. **30 corpus shaders have the affected shape**
(multi-pass with a trailing implicit final pass — measured from the vendored
corpus in Round 5; an earlier count of 33 in this file wrongly included three
single-implicit-pass shaders that were never broken). Affected: Glow, Soft
Blur, Lens Flare, Long Exposure, Auto Levels, Echo Trace, Ghosting,
VVMotionBlur 3.0, … For them, after the final (implicit) pass renders into
the destination, the code performed an extra nearest-filtered blit of the
pre-final buffer over the result (`runtime.rs:1095`), discarding the final
pass. The same wrong contract was baked into the docs (`runtime.rs:19-21`,
`overrides/README.md:77-78`, `mod.rs`), the 344/351 “success” narrative and
the multi-pass unit test.
**Fix (landed):** `passes.last().and_then(|pass| pass.target)`; magenta
regression test `trailing_implicit_final_pass_is_not_overwritten_by_a_blit`;
docs updated. Verified against the JS bundle's `lastTarget=null` per implicit
pass by the fix verifier.

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

### R-27 ISF corpus claim hygiene (`docs/isf-corpus-report.md`, `docs/rust-hybrid-progress.md`) ◐ — landed in `3754b249`

The corpus gate is `cargo test`, which (a) asserts only `translated > 0`, and
never asserts the 98.0 % bar, and (b) **rewrites the committed report on every
run** (`translate.rs:36,1002`), so the number in docs is machine/naga-version
dependent and silent regressions are invisible. Counts of `PERSISTENT` (43 vs
42, notes stale) and `FLOAT` (16 vs 15) in `docs/isf-translation-notes.md` are
stale. Also 7 override shaders are announced but the engine ships none
(`cargo build` does not include `crates/modv-renderers/src/isf/overrides`; the
`electron-builder.yml` `isf` resource ships only `resources/isf`), so in a
packaged build those shaders get **no overrides at all**.
**Fix (landed in `3754b249`, verified in Round 5):** checked-in
`isf-corpus-baseline.json` (344/0/7, 507 passes, 46 multi-pass, 15 FLOAT),
corpus test asserts ≥ 95 % + per-shader diff vs the baseline, report markdown
is written only to `OUT_DIR` (tracked file never rewritten by tests),
overrides are embedded (`EMBEDDED_OVERRIDES` via build.rs `include_str!`)
with `FNV-1a` fingerprint gating against `isf-overrides.json` (empty today),
`MODV_ISF_REPORT_PATH` affects only the manual report writer, and user
shaders named like corpus files cannot inherit overrides. **Still missing:**
the 7 failing corpus shaders have no override files and render nothing —
see Round 5 notes and the working list below.

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

- **Engine-mode registry is missing two legacy wrappers** ◐ — `Point10 Earth`, `Point10 Sun` exist in the legacy adapter (31 Point10 wrapper modules in `renderers/2d/modules/`) but have no Rust scene and no `Point10SceneKind`/registry entry (26 registered now). `Point10 Ball Of Fire` **is** registered as the 26th scene since `270a0003` — the default-Scenes parity gap ("Solar Flare"/"Terminal Bloom") is closed. Earth/Sun are referenced by no shipped default Scene, so this is inventory, not a regression. Note: `"Point10 Crt"` and `"Point10 Old Movie"` — also referenced by shipped default Scenes — **are** registered in the engine (`point10_effects.rs:447,720`).
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

- No code was compiled, tested, linted, packaged or run — this is a reading
  review with JS cross-checks. The author's e2e (144/144), benchmark and
  "1630 tests pass" claims were **not** re-run or reproduced.
- Line numbers drift with each commit; `✔` findings were re-verified at the
  reviewed tip (not every fix was re-verified by execution), `◐` findings are
  single-pass reads (verify before fixing).
- GPU-dependent behaviour (mip upload, clip-z, bloom alpha, clip masks)
  verified by reading wgpu/three.js semantics, not by running.
- `crates/modv-engine/data/default-scenes.json` content was validated
  structurally by a reviewing agent, not by re-serialising against the JS
  source.
- The review covers committed code only; the working tree at the snapshot was
  never touched. Where the author's counts disagree with themselves (e.g.
  1629 vs 1630 Rust subtotals, 947 vs 1005 Vitest), both were recorded
  verbatim rather than arbitrated — running the suites was out of scope.
- Review rounds are appended below; see [Round log](#round-log) for exactly
  what was covered and when. Rounds 8–12 covered the expressions/plugin-host
  work, GStreamer capture, engine media wiring and the Point10 fluid re-port
  (`9afa21dc..270a0003`). As of the final pass, no commits newer than
  `270a0003` had landed; the other thread had not stopped.

## Fix-wave verification — Round 5 (`36672a33..916586f5`)

The author addressed nearly every finding in dedicated commits
(`011e432a` engine/ipc · `5732bdf2` media · `21402b4f` render/2D/Point10 ·
`3754b249` ISF · `54e86322` app · `f1f3e615`/`916586f5` docs). Every fix was
re-verified by reading the new code; GPU behaviour verified by reading only.
The author also prepended a status table to this file
(“Status after the fix wave”) — it is accurate for R-1..R-30 except as noted
in this round's errata.

**Verified fixed**

- R-1 ISF trailing implicit pass — VERIFIED (`runtime.rs:907,1169`; blit now
  `passes.last()`; magenta regression test; corpus measurement reconciled at
  30 affected shaders).
- R-2/R-3 PipeWire negotiation — VERIFIED for negotiation (F32LE/S16LE only,
  8–192 kHz, 1–8 ch, `process` parks until accepted, readiness after accept,
  negotiated rate reported, latched errors via non-consuming `ChunkQueue`,
  analyser built from the negotiated rate). **Residual:** a mid-stream server
  death still surfaces nothing — no `state_changed`/`error` listener, so
  `next_chunk` returns `Ok(None)` forever: silence, no log, no renderer-pump
  fallback (the module doc even claims otherwise). Also a mid-stream
  rate/channel renegotiation leaves the startup-built analyser in place —
  silent mis-analysis until restart.
- R-4 cold-start sync — VERIFIED with design change: the engine now serves
  `engine/get-sync` (sender-validated, main-window-only, returns *current*
  state); the preload exposes `getSync()`; `application/index.js` awaits
  `_pullEngineSync()` during `_appReady` before boot dispatches are replayed;
  the accept-time `state/sync` path also feeds the same idempotent replay
  (`engine-sync-replay.js`: removals→adds of groups/modules/inputs/links/
  scenes/tweens/expressions; `registered` append-only). Re-entrant safe.
  **Gaps:** no e2e kills the engine between spawn and renderer-ready; the
  accept-time event can still fire before the tap registers (the pull is the
  load-bearing path — fine); engine-client re-subscribers still drop the old
  unsubscribe (P3, untouched); mirror tween descriptors carry no `frames`.
- R-5/R-6/R-7 BPM/FPS tween rebuilds — VERIFIED (`bpm > 0.0` in both
  collectors, infallible `TweenDescriptor::to_json`, per-entry serialisation,
  `.max(1.0)` dropped, no-loop branch restores the seeked frame; JS `|| 120`
  fallback for bpm≈0).
- R-8/R-9 tween reload — VERIFIED (`restore_scene_inputs` ends with
  `prune_tweens`, not full reload; per-entry decode with skip-and-warn;
  failed ids keep the previous runtime). **Caveat:** `remove_input` on Scene
  apply still never prunes stale tween descriptors from `state.tweens` —
  the accumulator grows across a long performance; tracked as a residual.
- R-10 scene-preview thrash — VERIFIED (retained instances for cached cells;
  scoped `dispose_instances` of the evicted slot only; `materialisations()`
  stays 1 across 20 warm renders; live path unchanged).
- R-11 sync replay drops tweens/expressions — VERIFIED (replay helper;
  unit-tested against the real worker store; idempotent; stale collection
  replaced).
- R-12 preview IPC — VERIFIED (magic + odd/even seq drop + post-pixel re-read
  retry ×3, exact slot bound, `slotBytes-32` and 64 MiB caps, `readSync`
  byte-count-checked, no `allocUnsafe`).
- R-13 removeScene/stopPerformance transitions — VERIFIED (both call
  `complete_all_row_transitions` + `complete_whole_scene_transition` and
  re-publish `SET_ACTIVE_ROWS`).
- R-14 migration duplicate removal — VERIFIED (first same-named match only,
  seeded gate, drops all seeded entries for the name, idempotent re-run).
  Residual: migration records carry names not ids (documented limitation).
- R-15 scene-capture rows — VERIFIED (`MAX_SCENE_ROWS = 1024` shared with
  `scene_rows`, BTreeMap, out-of-range rows dropped).
- R-16 frame-graph COPY_DST — VERIFIED (`check_usage` + negative test).
- R-17 2D self-sampling/clip/clear — VERIFIED (snapshot-routed
  GridStretch/Pixelate/Buffer; `clip()` intersects incl. circle∧rect;
  full-viewport `clearRect` is an ordered transparent-black `Replace` draw;
  zero-init on create/resize).
- R-18/R-19 OSC/tempo bounds — VERIFIED as library hardening (1<<16 array
  index, arg/bundle/store caps, bundle depth ≤ 8; tempo rate 1–1000, ≤ 60 s
  history, ≤ 60 000 frames). Note: OSC is unwired (R-23), so this protects
  WS-10 later, not a live path today.
- R-20/R-21 socket/ring — VERIFIED (`is_stale_socket`: ECONNREFUSED + socket
  node only; mode 0600 after bind; `Drop` unlinks only the bound dev+inode;
  ring created `0600`, slots init sequence=1/magic=0, poison-on-failure,
  publish = 4-byte seq rewrite with full-header reader re-check, slot bounds
  vs file length). Residual: bind/unlink TOCTOU vs a racing engine persists
  (unlink-at-bind is not atomic); the “windowless supervisor parks forever”
  P3 remains.
- R-22 RT allocations — PARTIAL (scratch owned by the stream closures;
  residual: `ChunkAssembler::push` still allocates/splits one `Vec` per
  emitted block on the cpal RT thread).
- R-23 engine-mode black-holing — PARTIAL (user-visible hints for Output
  Window/MIDI/OSC; boot warning; `windows/createWindow` never dispatched).
  Remaining: plugins list shows empty with no hint; fonts panel silently
  shows the 4 system fonts; the BPM source dropdown still offers "midi" and
  commits `SET_BPM_SOURCE midi` into the catch-all; `setupMidi()` still
  commits `midi/ADD_DEVICE`/`WRITE_DATA` into the void with no feedback.
- R-24 supervisor spawn/restart — VERIFIED (error→restart routing on
  “starting” state, stale-child gating, SIGTERM→bounded-await→SIGKILL with
  overlap log, `restartTimestamps` reset, budget logic extracted + unit-tested,
  window-pruned, fake-engine integration tests added).
- R-25 preload scope — VERIFIED (`engineApiScope` = main-only; `window.engine`
  undefined elsewhere incl. output window; sender validation on every engine
  handler; `ENGINE_IPC_CHANNELS` deleted). Loose end: `engine/get-state`
  skips the sender check (trivial); `engine-client.js` re-subscriber leak
  untouched.
- R-26 ISF edges — VERIFIED except overrides (size clamp to device limit;
  keyword-safe renames; `texture2DRect` → honest rejection (not a `textureLoad`
  mapping, but loud > wrong); matrix/array location accounting; FRAMEINDEX doc
  clarified; `$` scalars in size formulas incl. `$PASSINDEX`; `!!` truthiness;
  `has_fragment_main` real check; override WGSL contract-validated end to end
  with fingerprint (`FNV-1a`) manifest — user shaders named like corpus files
  cannot inherit overrides). `has_default()` still cannot distinguish absent
  vs `DEFAULT: null` (docs claim it can; zero corpus occurrences).
- R-27 corpus hygiene — MOSTLY VERIFIED (checked-in `isf-corpus-baseline.json`
  (344/0/7, 507 passes, 46 multi-pass, 15 FLOAT), corpus test asserts ≥ 95 %
  + per-shader diff vs baseline, report markdown now written to `OUT_DIR`
  only, tracked report never rewritten by tests). **Still missing:** the 7
  failing corpus shaders have no override files — they render nothing until
  someone writes the overrides (`Pixelize`, four Point10 Swirl/Wave variants,
  `Shape Morph Feedback Mask/Wrap`). Both the manifest
  (`isf-overrides.json` = `{"overrides": {}}`) and the README say so — honest,
  but the WS-4 “98 %” figure hides those 7 as silent skips.
- R-12-adjacent ring publish protocol — VERIFIED (see R-21).
- `remove_scene_action`/`stop_performance` — VERIFIED (see R-13).
- Ghost ordering + `restore()` republish — VERIFIED (ghost inserted before
  created Groups; stable row sort; restore clears rows/transitions and
  re-emits `SET_ACTIVE_ROWS`). Residual: the per-row interleave of incoming
  Groups vs the JS's unnamed-row block is an in-code comment only, undocumented
  in progress docs; `restore()` still leaves `engine_meter.pending` stale.

**Still open after the fix wave (all re-confirmed at `916586f5`)**

- **R-31 (P1) GeometricTunnel frame hoops ~16–24× too deep** — unchanged.
- **R-32 (P2) SmokeRoom pane/sphere alpha order inverted** — unchanged.
- **R-33 (P2) SmokeRoom lava sphere ignores `boxScale`** — unchanged.
- **R-34 (P2) RollingFire no-audio path freezes the sim** — unchanged.
- **R-35 (P2) compositron failure shows one unblended frame before the
  Fade fallback** — unchanged.
- **R-36 (P2) `fall_back_scene_transition_to_fade` snaps to frame-0 alphas**
  — unchanged.
- R-28 sub-items deliberately kept by the author (documented as deliberate):
  `ItIsComingForYou` keeps `preserve_bloom_alpha` (JS does not call it — alpha
  boost lost); `AroundPlus` keeps the GL-shaped matrix (justified for its
  screen-space water maths).
- `texture_pool.rs:229` — zero-size pool key still produces a zero
  `bytes_per_row` write (wgpu panic). Unfixed.
- `stroke_text` mask re-upload (~13 R8 textures per frame) — unfixed.
- Engine-client unsubscribe leak; `engine-ipc` `engine/get-state` sender
  validation; `MIDI` BPM-source dropdown reaches the catch-all;
  `engine_meter.pending` not drained by `restore()`.
- Docs arithmetic everywhere in the Wave docs: `rust-hybrid-progress.md` and
  the new “Verification snapshot” still say **424** registered modules
  ("37 shader + 15 2D + 21 Point10 + 351 ISF") — reality is **428**
  (25 Point10); `store_ops.rs` test constant still `37 + 15 + 18`;
  `mod.rs` says 287 image-input shaders (measured **288**);
  `docs/isf-translation-notes.md` still says “63 multi-pass / 43 PERSISTENT /
  16 FLOAT” (measured **46 / 46 / 15** — the progress doc itself was updated,
  the notes/plan were not); progress doc claims 1630 Rust tests summing to
  **1629**, and “947 Vitest” in one place vs “**1005** Vitest” in CHANGES.md;
  the fix-wave author added a status table to this file claiming R-1..R-30
  fixed — accurate except the counts above and the silent omission of
  R-31..R-36 (they were simply not in the table).
- Review-file hygiene: the committed copy of this document now leads with the
  author's status table; the “In flight at the time of writing … they will be
  reviewed when committed” paragraph was stale (now updated), and “Last full
  pass” lagged behind (updated by this round).

## Round 6 (`916586f5..73729fdb`)

- `electron-builder.yml` — Point10 scene textures (`point10/textures`) now
  ship as `extraResources: textures`; `engine-supervisor.js` computes
  `assetsDir` (packaged resources path in production, source-tree path in
  dev) and exports `MODV_ASSETS_DIR` into the engine. Clean, minimal fix for
  the packaged-build texture fallbacks (reported as fixed in the progress
  doc: zero asset-fallback warnings, 107 fps, 144/144 e2e).
- Doc claims re-checked: “the `.deb` depends on the distro's package set …
  `MODV_ENGINE=0` (the default) never launches the engine” — the default-flag
  half is true; the `.deb` claim is **not**: `electron-builder.yml` `deb:` has
  no `depends:`, and the electron-builder default set (GTK/notify/nss…) does
  not include `libpipewire-0.3-0`, while `modv-media`/`modv-app` link it
  **unconditionally** (linux target-dep) — so on a minimal host without it,
  simply spawning the engine dies at load time regardless of
  `--audio-source`, and the supervisor burns its restart budget. Fix: build
  `modv-engine` with dlopen for PipeWire (like NDI), or declare
  `deb.depends: ["default", "libpipewire-0.3-0"]`.

## Round 7 — `73729fdb..9afa21dc` (ISF 351/351, gallery effect previews)

### R-1/R-27 follow-up — the last 7 corpus shaders — VERIFIED, with one honest caveat
Commit `77fcb048` adds four general GLSL→WGSL passes to `source.rs` (declared
hoisting of non-constant global initialisers; per-call-site cloning of
sampler-parameter functions; deterministic default returns for non-void
fall-through functions; column splitting of matrix varyings) and bumps the
baseline to 351/351, 507 passes, no overrides. Verified by reading only:

- All four passes preserve declaration order and only fire on unreachable
  paths or shapes naga previously rejected outright; the previously-344
  "translated" corpus keeps identical semantics by construction, and the
  corpus gate (exact name-set equality + ≥ 95 % assertion, `OUT_DIR`-only
  markdown) now pins it. No previously-fine shader can reach the sampler
  cloning path; matrix varying splits only the previously-failing sidecar.
- Pass-specific cares: const globals never hoisted; `#if`-guarded
  declarations never move; recursion refused (loud failure); clone tuples
  cache per distinct sampler-pair not per call (no exponential blowup;
  unbounded for pathological user shaders — no compile-time guard); a call
  *before* a sampler-function's definition is never inlined (documented
  false negative); `apply_edits` silently drops out-of-order edits (footgun
  for future passes, P3).
- **Caveat (docs):** the "100 % corpus" claim is materially misleading for
  **v002 Light Leak**: it now naga-validates, but its sidecar's `mat2`
  rotation is rebuilt from zero-initialised private globals — the leak's
  rotation is a silent no-op. Runtime behaviour is identical to before the
  commit (the effect was already dead), so it is *not* a regression; but
  "351/351 translation passes naga" must not be read as "all 351 shaders
  render fully". The `PERSISTENT`/`FLOAT` counts (46/15) remain accurate;
  the tracked report is consistent with the new baseline.
- All seven previously-failing shaders individually verified against their
  specific failure modes (Pixelize hoist; Swirl/Wave/Banded sampler clones;
  Shape Morph default returns; Light Leak column split).

### `9afa21dc` — gallery effect previews (test card) — VERIFIED, clean
Effects were rendered over a transparent/blank input so filter thumbnails
were blank. `preview/module` now renders over a synthetic UV-gradient +
checker card (white border), padded to the 256-byte row alignment, with a
synthetic `audio`/`audioFFT` texture pair for shaders declaring audio inputs
("animates even when the live frame is silent"). Textures are per-slot,
dropped with the slot (bounded by the 32-slot cache), `create_output_texture`
includes `COPY_DST` (no write-texture validation risk), image inputs are
bound to the same test card. Compositor tests now assert the effect frame is
non-uniform and opaque. No live-path impact; thumbnails now legible.

### `8df313c9` — corpus docs — fine modulo the caveat above
Claims "351/351 fragments, 36/36 vertex sidecars, no overrides" — the
*counts* reproduce exactly from the corpus, but see the v002 Light Leak note
and `mod.rs`/`isf-translation-notes.md`'s still-stale 63/43/16 figures
(carried from Round 5; unfixed in this delta).

### In flight (uncommitted, not yet reviewed)
At the time of this round, the working tree carries a substantially larger
scope: an **expressions engine** (`crates/modv-engine/src/expressions.rs`
and `crates/modv-app/src/expressions.rs`, new), a **JS plugin host**
(`src/renderer/src/application/plugin-host.js`, `install-plugin.js`,
`plugins/grab-canvas.js`, `use.js`, `tests/unit/plugin-host.test.js`), video
capture (`crates/modv-media/src/video/`, new), Point10 volume scenes
(`scenes/dvd_logo.rs`, `point10/volume/`, new), ISF `translate.rs`/`source.rs`
further changes with baseline bumps, `docs/rust-full-rewrite.md` (new), and
`Dockerfile`/Cargo.toml updates. Not reviewed until committed. The
full-rewrite doc suggests the project may be pivoting beyond the hybrid
plan — watch for its landing.




## Round 8 — `9afa21dc..89f8acc9` (R-31/R-35/R-36 fixes, Boa expressions, JS plugin host)

Three commits: `c5c5fd23` (fixes my Round 1–4 findings), `0ae8943c` (WS-12
expressions via Boa 0.22 embedded JS), `89f8acc9`
(renderer-side JS plugin host in engine mode).

### `c5c5fd23` — my findings' fixes

- **R-31 — VERIFIED (real fix).** The frame-hoop instance now carries
  `scale(RADIUS, RADIUS, TUNNEL_HOOP_DEPTH)`; regression test asserts the
  basis (column) lengths of `instances[0].model` ≈ JS contract; JS parity
  bit-for-bit (half-depth ≈ 0.021–0.031, radius 4.2 × pose.scale). No stale
  asserts.
- **R-36 — VERIFIED.** `fall_back_scene_transition_to_fade` now computes
  `eased = smoothstep(progress)` and lerps `to.alpha = alpha*eased` /
  `from.alpha = alpha*(1-eased)` — line-by-line JS parity (`scenes.js`
  `fallbackToFade`); sets values one frame earlier than the JS, with a new
  50 % midpoint test. Resize-mid-transition path reaches the same fixed
  fallback. One stale doc comment on the function (claims incoming resets to
  0) survived; the resize glue has no test.
- **R-35 — PARTIAL.** Pipelines are pre-built before the first transition
  frame (`prepare_scene_transition`), and the *prepare-failure* branch
  downgrades in-frame with an honest WARN at the failure site (no unblended
  frame). But that branch is unreachable in this wgpu (`failed[]` has no
  writer; native validation errors panic; no error scopes) — and the
  apply-time failure path (`render_frame_retaining`) **still shows one
  unblended frame** (drops `transition_from`, no WARN at the failure site,
  downgrade next frame via the "lost the captured outgoing frame" path with a
  misattributed WARN). Keep open until the failure site logs/downgrades or
  error scopes make the pre-build verdict observable; also add the
  window-glue test.
- **deb — VERIFIED.** `electron-builder.yml` now declares the nine
  electron-builder v26 defaults plus `libpipewire-0.3-0`. Minor: the frozen
  default list won't track upstream default changes (acceptable; correct
  today). AppImage keeps host-provided `libpipewire-0.3` (documented, fine).

### `0ae8943c` — expressions via embedded Boa (WS-12) — new surface, new defects

- **[P1] `crates/modv-engine/src/expressions.rs:138`** — no wall-clock /
  allocation budget on evaluation; the 100k loop bound and 64 recursion cap
  only bound VM-counted loops. Native builtins with their own loops
  (`Array.from({length: 1e9}, …)`, `.map/.forEach/.reduce` on huge arrays,
  `"x".repeat(1e9)`, `new Array(1e9).fill(0)`) run with no budget at all.
  Expressions run synchronously on the engine's event-loop thread (so do
  `evaluateEveryFrame` ones at every frame). A document shared as a scenes
  playlist → user "Import Playlist" → `restore_scene_expressions` compiles
  the source. A crafted assignment stalls rendering/audio for minutes or
  until OOM; repeated via `evaluateEveryFrame`, forever. (The Electron
  heartbeat watchdog would eventually kill the engine, and the restart
  budget would then re-apply the same Scene → crash loop → `fatal` — the
  damage is bounded but self-inflicted on a shared file.) Mitigation starts
  with `RuntimeLimitError`-style heap/timeout or an allocation ceiling + a
  per-expression execution cap (and *documented* trust stance for playlist
  import). Legacy Chromium had no budget either — but "engine embeds a JS
  engine in a security boundary" is now a shipped claim worth holding.
- **[P2] `__modvExpressionFunctions` is discoverable and WRITABLE/CONFIGURABLE.**
  Any expression can read/overwrite/`delete` any assignment's handle by
  predictable `e0, e1, …` keys (silent permanent freeze of that input) or
  dump every other expression in the document. Register NON_ENUMERABLE and
  freeze the map around rebuilds (or use unguessable keys).
- **[P2] every-frame evaluation order is nondeterministic** — `by_input` is a
  `HashMap`; two `evaluateEveryFrame` assignments targeting the same
  prop/module race in random order each process start (JS first-wins). Iterate
  insertion order (`IndexMap`/sorted).
- **[P2] lodash path parity** — `props.nested.0.x` (numeric string key vs
  array index) and `"a..b"` resolve wrong (serde_json `Value::get(&str)` vs
  arrays; silent `null`). No crash; silently wrong scope for valid paths.
- **[P2] `restore()` replaces `state.expressions` wholesale with no deltas** —
  mirror stale after restore (heals only via sync replay; matters once
  WS-13 wires restore to the UI).
- **[P2] the Boa realm is shared and long-lived** — prototype-pollution
  assignments (`Array.prototype.push = …`) poison every later expression in
  the session, including across restores. No realm refresh path exists.
- **[P3]** no compile cache (every restore re-parses all sources — typing
  in the Expression textarea re-compiles the set); `writeToSwap` silently
  ignored on all four actions (deltas carry `false` — mirror matches, but
  the caller gets no signal); per-frame `Vec<String>` clone per frame pass;
  `enabled: f64→i64` truncation drops fractional parts;
  null-returning every-frame props skip writes (intended) but
  null-returning `updateProp` writes `null` over the live value;
  **stale docs** — `rust-hybrid-progress.md` and `rust-hybrid.md` (WS-12
  “Recommended (a), not shipped") still claim expressions are unimplemented,
  while they are now a shipped, Boa-evaluated feature (and the empty-`osc`
  caveat exists only in code docs).
- Also note: claimed "260 modv-app tests" vs 261 counted; Boa
  `default-features = false`, no native bindings — the P1 is the only
  sandbox reality; scope (`value,time,inputValue,osc`) exactly matches JS;
  NaN guards hold; deltas and replay integration are correct;
  `evaluateEveryFrame` ordering (after link evaluation, before render)
  matches the JS; restore compile is skip-with-warning, never NaN.

### `89f8acc9` — renderer-side JS plugin host (engine mode) — mostly sound boundary, gaps

- **[P2] plugins can claim core mutation types** — non-namespaced plugin
  modules can register `groups/ADD_GROUP` (etc.) as their own mutation;
  the facade then applies the plugin's shadow handler to the mirror only —
  the engine never sees the write (permanent desync), and the same-type
  shadow handler fires again on engine→mirror replay. Require plugin
  namespacing or reject non-plugin types.
- **[P2] engine commits to unimplemented types vanish with zero signal** —
  fire-and-forget IPC; engine warn-once goes to stderr; the renderer never
  hears it.
- **[P2] async `preProcessFrame` rejections bypass the warn-once guard** —
  caught synchronously only; a rejecting hook logs via `unhandledrejection`
  every renderer frame, growing the daily log without bound.
- **[P2] unsupported plugins (grab-canvas) get a dead, silent toggle** —
  `supportsEngine: false` is enforced inside the host but the panel
  (`Plugins.vue`) renders an ordinary checkbox that never sticks; no badge,
  no reason shown.
- **[P3]** the unsupported guard is bypassable by a raw `plugins/*` commit;
  a throwing `init`/`shutdown` wedges the enabled state; `host.add` failure
  aborts the rest of installation (UI/install hook skipped for engine-mode
  installs where legacy always proceeded); camelized-name collision leaves a
  live-but-inert plugin (its mutations flow to the engine catch-all);
  `plugins/*` actions outside the three hosted ones fall through to the
  void; `preProcessFrame` reads a one-frame-stale payload (no
  read-your-writes between hook and delta replay); test suite lacks the
  async-hook/shutdownAll/collision coverage.
- **Checked and clean:** no new untrusted-code loader path — plugins enter
  via the built-in grab-canvas and `modV.use()` only; legacy media-dir
  `eval()` stays inert (engine catch-all) and untouched; no engine→mirror
  sync path touches `plugins/*` or plugin namespaces; the legacy worker
  path is byte-identical; grab-canvas cannot enable in engine mode; hook
  execution is fire-and-forget rAF with try/catch; frame ordering vs delta
  replay is deterministic (single thread); test file uses the real mirror
  store + stubbed engine.




## Round 9 — `89f8acc9..418c5cc8` (GStreamer video capture WS-10)

### `122389e9` — GStreamer video capture core — sound FFI, packaging/robustness gaps
Camera DeviceMonitor, `uridecodebin` file sources, `pipewiresrc` portal
nodes, test-pattern pipelines → newest-wins RGBA8 frames via a bounded
two-buffer pool. No GLib main loop (bus drained via `pop()`), no unsafe
code, gst objects owned by gst-rs values, `map_readable` folded during the
copy, no steady-state allocation, injection-safe launch strings
(`escape_launch_value` with a whitelist + escape path; verified),
`videoconvert`+`videorate` negotiation, file sync/loop semantics.

Findings:
- **[P2] packaged engine does not declare GStreamer** — deb deps weren't
  extended past PipeWire; the AppImage ships only the bare binary (plugins
  are dlopened); Dockerfile only adds plugin sets to the rust/toolchain
  images. `MODV_ENGINE=1` fails to load on stock Debian/Ubuntu; even a
  started engine dies on first camera open. Declare libgstreamer1.0-0,
  -plugins-base1.0-0, gstreamer1.0-plugins-base/good, gstreamer1.0-pipewire
  (or dlopen-style optional loading).
- **[P2] 1 s start timeout aborts slow file prerolls** — non-live
  `uridecodebin` must preroll; ASYNC-pending after the timeout →
  `CaptureError::PipelineRefused` misreported as “no error on the bus”; the
  module doc says “a timeout is not a failure” — the code is stricter.
- **[P2] stalled sources are never detected** — PipeWire node whose producer
  stops / V4L2 stops delivering: pipeline stays `Playing`, frames freeze
  forever; `sequence` stops advancing with no recovery or surfaced error.
- **[P2] Screen sources accept unbounded caps** — `VideoSourceKind::Screen`
  ignores requested dims and `info.width()/height()` is never clamped
  (`MAX_DIMENSION` applies only to *requested* sizes); a node advertising
  e.g. 65535×65535 drives `reserve(~17 GB)` → process abort. Cap negotiated
  size/total pixels. (Close to P1; kept at the reporter's P2 with this note.)
- **[P2] feature is dead code today with a new hard runtime dependency** —
  zero callers in `modv-app`/`engine-supervisor`/IPC; it compiles into the
  engine regardless → packaged engine gains DT_NEEDED on GStreamer for
  uncalled functionality until the IPC wiring lands. Document the gap or
  gate it.
- **[P2] live camera/screen tests can never fail** — both skip on error
  (only file/encode tests assert hard); the suite is 55/55 green with the
  camera path untested even in a misconfigured environment. Count is honest
  (2+10+4+18+10+11 = 55); the evidence isn't.
- **[P3]** `stop()` swallows `Null`/join failures (docs claim the callback
  cannot touch the frame afterwards, resting on unchecked results);
  non-RGBA/negative-stride negotiation returns `FlowError::Error` without a
  bus message, so the camera stays open in `Playing` until stop; camera
  enumeration is a point-in-time snapshot (no hotplug, stale v4l2src nodes
  listed); `screen_support()` treats a stale socket file as available;
  file-missing codec fails late with a generic message (no element pre-flight
  for h264/h265/vp9); `path_to_file_uri` lossy non-UTF8 bytes; 16k×1000 fps
  clamps instead of sane bounds; `test_support.rs` stop test under-skips
  required elements; `select_camera` empty-monitor fallback returns None only,
  no feedback channel for empty list vs busy device.
- **Clean:** launch-string escaping; frame pool swap (no torn frames, ≤4
  buffers); RGBA↔WGSL byte order; stride/neg-stride checks with
  `checked_mul`; FIFO caps parsing with sign checks and 16/structure cap;
  `sync=false` live / `sync=true` file; EOS→seek loop with latched failure;
  `PluginMissing` pre-flight; no unwraps on external input; 55 `#[test]`s,
  none hidden behind `#[ignore]`.

### `418c5cc8` — docs commit (this file's round-5/6 edits, committed by author)
No functional content beyond landing the working-tree state of this review;
the file in-tree now reflects rounds up to early Round 8.

### In flight (uncommitted, not yet reviewed)
`crates/modv-app/src/media.rs` (new — WS-10 wiring?), `src/main/engine-media-request.js`
(new), `src/renderer/src/application/engine-media.js` (new),
`tests/unit/engine-media{,-client,-request}.test.js` (new), `Dockerfile`
(GStreamer runtime in packaged images?), `compositor.rs`, `module_preview.rs`,
the R-32..R-34 Point10 fixes (`dvd_room.rs`, `rolling_fire.rs`,
`smoke_room.rs`, `dvd_logo.rs`/volume — new), `engine-ipc.js` /
`preload/engine-api-scope.js` / `engine-client.js` deltas (media channel?),
`docs/rust-full-rewrite.md`, `crates/modv-media/src/lib.rs` exports. Watch
for the media IPC commit — it turns WS-10 from groundwork into a real user
path, and it must fix the packaged dependency story (finding 1) to match.




## Round 10 — `418c5cc8..e88bbd07` (engine-owned media sources, WS-10 end-to-end)

Two commits: `9d5f2d87` (renderer/main: IPC, media-mirror, UI, engine-media.js
+ tests) and `e88bbd07` (engine: MediaHub, supervisor actions, compositor
texture pool, Webcam binding, window wiring).

### The two P1s (renderer-side file-source path)

- **[P1] `src/renderer/src/application/engine-media.js:336` — stored media
  paths start with a leading `/`, so engine video files can never open on
  POSIX.** The media manager stores `item.path = filePath.replace(mediaDirectoryPath, "")`
  → `"/video/clip.mp4"`. `isAbsolutePath()` matches any leading `/`, so the
  media-directory prefix is never prepended and the source id is built from a
  root-level path that doesn't exist. Unit tests only use relative
  `"default/video/a.mp4"` — pinned green while the feature is broken. Legacy
  works because `path.join` doesn't reset on a leading slash. Same function
  also keeps `..` unresolved. **The "video files play through Webcam" path
  does not work on a stock Linux/macOS install today.**
- **[P1] `src/renderer/src/components/Controls/TextureControl.vue:294` —
  engine file pipeline is killed by UI navigation, not the module.** The
  file-source lifetime is tied to the Control component: closing the
  inspector / switching modules or Scenes unmounts the control and calls
  `stopEngineVideoSource` — the video freezes on its last frame. Legacy
  kept the pipeline via the worker auxillary regardless of panel state.
  Preset/scene load never re-mounts the control, so a loaded video texture
  shows nothing until opened in the inspector.

### Engine/media findings

- **[P2] MediaHub:** (a) a `loop:false` file is replayed forever —
  `poll_health` restarts any `needs_restart()` source (Error and Ended
  lumped together), so the loop flag never stops replay; (b) a failed
  re-`start` of an existing sourceId silently removes it from `order` —
  all later sources re-index (a Webcam with `camera:2` silently becomes the
  previous source 3) and the failure is invisible to status/reconcile;
  (c) mid-list source stop is a silent Webcam feed switch — `camera:N`
  indexes the live source list, not a stable identity; no test covers the
  texture sampled after a mid-list stop; (d) `schedule_restart` runs inline
  on the render/heartbeat thread — `stop_source` joins up to 2 s, and a
  failed retry waits `wait_bus_error` a full 1 s; one unplugged camera ≈
  1 s frozen output per retry — bridges the 5 s Electron watchdog; (e) the
  restart bookkeeping is half-dead (`logged_failure` never set) and there is
  no permanent-failure latch — unplugged camera retries forever at cap.
- **[P2] Path containment absent** — `media/start-file` and `file()` keep any
  string with a scheme (`https://thing`, `rtsp://…`) and any pathverbatim;
  no canonicalisation/containment to the media root anywhere; renderer
  passes, main validates only presence. Not a privilege boundary (same UID,
  0600 socket, renderer already has nodeIntegration) but a contract the code
  claims to keep, plus a network-fetch vector from the engine process.
  (`cameraId`/`pattern` are launch-escaped, so no injection.)
- **[P2] Camera order divergence** — the engine indexes the Webcam `camera`
  prop across **all** running sources (cameras+files+screens, start order);
  the renderer computes `cameraIndexForDevice` over cameras only — any
  stop/re-start after a file source is running silently repoints every
  Webcam prop; the renderer never re-derives/re-asserts order. Observed as
  constant `reconcileCameraIndex` mismatch warnings.
- **[P2] Media not re-armed after engine restart** — no `onState running`
  hooks into `ensureCameras`; with the panel closed, selected cameras stay
  dead after a supervisor restart; `refreshVideoInputs` is only called at
  boot/devicechange/re-scan.
- **[P2] `media-mirror` cannot represent deletions** — `CLEAR_MEDIA_STATE` /
  `RESET_STATE` are dropped (only ADD is handled) — the library shows stale
  items whose sources can never start (no UI feedback); on top of that the
  mirror replay path throws on `media/addMedia` inside the same tick.
- **[P2] `media/list-cameras` failure ≈ empty list** — `ensureCameras` /
  `refreshVideoInputs` then stop every selected camera on a transient
  failure — one flaky enumerate makes live captures drop.
- **[P2] Re-scan starts with engine defaults, not saved `videoSettings`** —
  the boot-restore/pollStatus paths honour the saved per-device resolution/
  frame rate, `ensureCameras` does not.
- **[P2] File start is silently dropped or invisible** — started before the
  `media-manager-state` replay lands → `toEngineMediaPath` returns null →
  nothing runs, nothing logged; engine start failure only logs (no UI
  feedback); and it stops the *previous* pipeline first.
- **[P2] Unbounded retry loop** — fixed 5 s interval, no jitter, no max
  attempts/circuit breaker; permanently busy device = one start attempt
  every 5 s forever (each a full GStreamer nego.).
- **[P3]** Overlapping 1 Hz `media/status` polls (no in-flight guard; 4 s
  main-side timeout → up to 4 queued requests); `EngineClient` media methods
  + `withTimeout` are dead code; stop bookkeeping happens before the engine
  confirms; unguarded `state.media.media[project]?.video` computeds; screen
  path dead (`startScreen` passes ignored `name`, tracks nothing); tests
  pin relative paths only; media-mirror tests prove replay commits but
  never run real Vuex mutations; no test for the two P1 paths.
- **[P3] Engine:** non-looping file loop bug follow-throughs; mid-list stop
  details; `MediaTexture` pool never `tick()`ed → LRU collapses (~1 GiB
  re-create on size flip); `camera_index` rejects numeric strings the JS
  accepts; `media/start-test-pattern` registered unconditionally;
  `media/stop-source` swallows the boolean; `MAX_DIMENSION` 16384 → ~1 GiB
  texture alloc; no test for the mid-list `camera:N` behaviour nor for the
  frozen-texture backoff path.
- **Clean areas (verified):** all new channels sender-validated
  (`isMainWindowSender`); payload validation int-bound/positive-checked;
  error-code contract matches the engine exactly (`media_*`); engine-mode
  gating consistent everywhere (AudioVideo/ScreenCapture/
  ScreenPickerDialog/polling) — nothing inverted, legacy unchanged; status
  wire shape matches the JS normalisers (`{sources:[...]}`,
  `{cameras:[...]}`, start `restart` vocabulary does not fight the engine
  backoff); mirror store registered at boot with sane defaults; dual-commit
  parity with legacy; texture upload skips unchanged sequences, per-source
  pools, unaligned rows zero-padded (tested), preview/`Texture 2D` isolation
  holds; scene apply/restore genuinely keeps sources; stop_all synchronous
  before preview rejection and teardown; engine-mode interdependencies
  throughout are 0600 same-uid socket + 0600 ring — no cross-user access.
- **Docs to fix (this round):** `docs/rust-hybrid-progress.md` still says
  the deb depends "the distro's package set" — it declares only libpipewire;
  this commit adds GStreamer DT_NEEDED with no deb/AppImage
  acknowledgement; `TextureControl.vue` version claim; media progress
  entries.
- **Bottom line:** IPC/validation layer and the engine↔renderer contract are
  tight, and the renderer gating is clean — but the two P1s in the file-source
  path (path rooting; control-lifetime stop) plus the camera-order divergence
  mean this feature ships broken end-to-end on real installs today, in both
  user-facing contexts (files never open; file playback stops when you close
  the inspector).

## Round 11 — `e88bbd07..28d90158` (camera-index fix; Point10 SmokeRoom/DvdRoom fluid-sim re-port)

### `28d90158` — Webcam camera index follows camera-like sources — FIXED (engine half)
Engine now selects the Webcam input from the camera-like order
(`Camera`/`Screen`/`TestPattern`; files excluded) while still uploading
textures for every source — starting a file can no longer shift the Webcam
prop; renderer's cameras-only order matches. Residual: screen nodes count as
cameras engine-side but not renderer-side (the renderer gates screen capture
as unavailable, so the divergence only shows if a screen is started via the
diagnostic path — benign, but note it). The engine and renderer still need a
documented agreement here.

### `9e3ac2a5` — SmokeRoom/DvdRoom re-ported to a real GPU fluid sim — PARTIAL
~6 300 lines: stable-fluids sim (advect velocity/dye, curl + vorticity,
divergence, 2-iteration Jacobi, project), raymarched volume, bloom → ACES;
JS uses the same GPU fluid path — this is a faithful re-port, not an
"approximated to particles" regression.

**Prior findings against the rewrite:**
- **R-32 — VERIFIED FIXED** — panes (transparent, back-cull = JS FrontSide)
  render after the opaque sphere/logo; volume light combined additively
  after both, matching the JS chain.
- **R-33 — NOT FIXED** — `boxScale` still never reaches the sphere/logo
  mesh or the emission matrix (`smoke_room.rs:481/379/443`,
  `dvd_room.rs:419-428/599`); at `boxScale ≠ 1` the wind region and
  emission radius disagree with the visible mesh — same bug as before the
  rewrite.
- **R-34 — NOT FIXED** — `rolling_fire.rs:1332` returns before the puff
  loop when `!audio.is_available()`; the new volume scenes do not share the
  gate (internally inconsistent).
- **R-28 (DvdRoom lane pairing) — FIXED** — bloom/`dropHue` driven from
  5-bin `smoke_bass` (`dt*8`), logo/glass/camera from asymmetric
  `smoothed_bass` (`dt*16/dt*5`), glass opacity `0.34 + smBass*0.06` =
  `DvdRoom.js:445`.

**Regressions introduced by the rewrite**
- **[P2] GPU coverage for RollingFire and GeometricTunnel was silently
  deleted** — `rolling_fire_renders`, `geometric_tunnel_renders`,
  `geometric_tunnel_keeps_a_translucent_layer` all removed in the same
  commit; both scenes still ship. RollingFire has zero render coverage now.
- **[P2] transparency contracts weakened** — min-alpha ≤ 48/16 assertions
  replaced by ">10 % of pixels below alpha 128"; haze can now legally cover
  most of the frame.
- **[P2] production runs the full JS grid always** — 150×150×300 = 6.75 M
  voxels ⇒ 8 × 54 MB = ~432 MB `Rgba16Float` + ~40 M invocations/step, no
  production override (test-only `MODV_VOLUME_GRID`); weak iGPU /
  software-adapter users get no escape hatch and GPU tests don't run at JS
  size.
- **[P3]** `uSway` computed but never consumed (flame anchor missing the JS
  ±0.4 x/z term); attenuation correction missing the JS `max(0.01, d²)`
  clamp (dark pinhole at the flame core); `half_to_f32` decodes subnormals
  4× small (test-only); two `.expect("bloom pass always has a composite
  target")` panic paths on resize; translucent panes write depth
  (`depth_write_enabled: true`, JS uses `depthWrite: false`); `fire_temporal`
  evaluated and discarded whenever `steps > 0` (6 CPU Perlin evals/frame
  wasted); dead `params.z/w` writes; stale test-grid comment; heavy GPU
  tests (~minutes on lavapipe) dropped two unrelated scenes.
- **Clean:** advect/divergence/Jacobi/project signs and BC (clamp-to-edge =
  JS), dissipation < 1, emit-then-parity-flip order matches the JS shader
  chain, raymarch constants (Beer/multi-scatter/powder/dither), half-res HDR
  targets rebuilt on resize, no per-frame readback, the two transparency
  tests exist and are strict (`assert_alpha_is_max_rgb`), particle system
  fully gone, both volume scenes stay alpha-transparent end-to-end.

## Round 12 — `28d90158..270a0003` (RollingFire volume + BallOfFire; docs)

### `270a0003` — Rolling Fire becomes the JS GPU fluid sim; Ball Of Fire registered (26th scene)
The particle billboards are replaced by the shared 100×100×200 grid +
18-step raymarch + 2-step shadow march, hard key light, lava sphere/floor,
bloom/ACES; BallOfFire = RollingFire with the frozen ball (transparent
overlay, standalone) registered as the 26th scene — **my default-scenes
parity finding is closed** ("Solar Flare"/"Terminal Bloom" layers resolve
again). Docs bumped to 429 modules / 26 scenes / 1777 workspace tests.

- **R-34 — VERIFIED FIXED** @ rolling_fire.rs:1137-1148/1267-1277 — the sim
  step is submitted regardless of audio availability; silence arrives as a
  zero `freqMag` and the smoothers decay, the ball coasts; registered
  audio-reactive so live audio is picked up immediately. **But the
  regression test never exercises `render()` without audio** —
  `render_owned_chain_frames` only runs loud/quiet payloads, and the
  silent-path tests are CPU-only; a future gate on `is_available()` in
  silence would still pass everything. Add one GPU frame test (silent
  bridge, N frames, assert the dye field advanced or `pending_steps > 0`).
- **[P2] straight-alpha scene targets composited as premultiplied** —
  engine-wide (pre-existing, not introduced here): the frame graph assumes
  all-premultiplied, `blend.rs:415` multiplies `src.rgb` by alpha as if
  premultiplied, but Point10/ISF scene targets are **straight** alpha
  (`alpha = max(rgb_HDR)`, `rgb` 5–8× brighter than `alpha` for dim pixels).
  Over an opaque/bright backdrop the engine ADDs too much colour; over black
  it matches canvas. Solar Flare/Terminal Bloom sit on Background rows
  (black) today, so current default content is unaffected — but this is the
  one x-ray for every future Point10/ISF layer over opaque content, and it
  contradicts `present()` being documented as un-premultiplied. Needs an
  explicit decision (premultiply scene output, or un-premultiply in the
  composite path).
- **[P3] SmokeRoom/DvdRoom never wire `uSway`** — the flame point-light
  segment stays anchored at the sphere (`uSway` computed, never bound); JS
  sways the segment in both rooms; `VolumeUniforms::light2` doc still says
  "yzw unused".
- **[P3] GPU tests pass vacuously without an adapter** — every volume GPU
  test returns early on missing adapter; there is no lint/test CI; the
  lavapipe toolchain guards it locally but nothing gates it in GitHub.
  Both volume scenes resident ≈ 0.6 GB RAM/VRAM at once (RollingFire ≈
  128 MB, rooms ≈ 432 MB).
- **Docs**: 26/26 scenes, 429 modules now accurate; Vitest count still says
  **947** in the progress doc while CHANGES.md says **1005** (carried).
- **Clean:** BallOfFire registration end-to-end (kind, `Point10SceneKind`,
  all registry arms, meta vs JS wrapper, default-scenes resolution,
  `all_scenes` order, count tests, scene meta "2d"/"3d-scenes"/
  transparent-overlay); `_freezeBall` skip; the whole RollingFire audio
  mapping is JS-exact (bass dt*8, hats dt*10, snare dt*12, `1+hi*2.5+
  snare*8`, camera-yaw 0.15, hue shortest-arc ease); step order;
  rooms-pass-JS defaults (16 steps, key light) preserved after the
  parameterisation; `fire_colour.wgsl` extraction is behaviour-preserving.

## Outstanding items — the working list as of `270a0003`

Ordered by severity. Everything here is fix-wave-verified or carried forward
(marked ✔/◐ in the body). Items closed by later rounds are noted as such
rather than deleted.

**P1**
1. **Expressions: no wall-clock/allocation budget** (`expressions.rs`) —
   native builtin loops (`Array.from({length:1e9})`, `.map/.forEach/.reduce`,
   `"x".repeat(1e9)`) bypass the 100k loop bound; expressions run
   synchronously on the engine event-loop thread, and `evaluateEveryFrame`
   assignments run every frame. A shared scenes playlist with a hostile
   expression wedges the app (the heartbeat watchdog kills the engine;
   restart re-applies it; budget exhausts; engine goes `fatal`). Add a
   heap/time ceiling plus a per-evaluation cap, and document the trust
   posture for imported playlists.
2. **Media path rooting (engine-media.js)** — stored media paths are
   `/` prefixed (e.g. `/video/clip.mp4`), so the POSIX
   `media/start-file` target never resolves under the media directory;
   files play through Webcam in the default config only by accident of
   Windows semantics. Fix: canonicalise `item.path` into `mediaPath` (POSIX
   join, rejecting `..`), and add tests pinning a stored `/video/x.webm`.
3. **Media source lifetime bound to the TextureControl component** —
   closing the inspector/switching modules or Scenes stops the engine file
   source (`stopEngineVideoSource` at `beforeUnmount`), freezing video; a
   preset/scene load never restarts it (no `created()` when not mounted).
   Fix: own the source lifecycle by the texture-id/module prop and persist
   across scene restores (mirroring the legacy `videos` auxillary).

**Closed / re-verified**
- ~~R-32~~ — SmokeRoom pane/sphere order fixed with the volume re-port.
- ~~R-28~~ — DvdRoom 5-bin bloom lane vs `band(1,9)` logo/glass split matches
  the JS.
- ~~camera order divergence~~ — engine Webcam prop now indexes camera-like
  sources only (`Camera | Screen | TestPattern`); files never shift it.
  Residual: screen counts as camera engine-side, not renderer-side (benign
  while screen start is gated off).
- ~~R-34~~ — RollingFire no-audio path idles properly (freq_mag zeroed,
  sim runs; `270a0003`). **No GPU-level silent-audio regression test**
  (all silent-path tests are CPU-only; see Round 12).
- ~~BallOfFire registry gap~~ — registered as the 26th scene in `270a0003`;
  "Solar Flare"/"Terminal Bloom" layers resolve.
- ~~R-31, R-36, deb PipeWire, R-1, R-4, R-10…R-30 (round 5)~~ — see Rounds 5–6.
- Docs module/scene counts now accurate (429/26).

**P2**
1. **R-2 residual** — PipeWire mid-stream server death is silent: no
   `state_changed`/`error` listener → capture never reports, feed stops,
   silence, no fallback. Register the listener and route to the same
   fallback used for startup failures.
2. **Point10 volume re-port (`9e3ac2a5`/`270a0003`) follow-throughs** — (a) RollingFire
   and GeometricTunnel lost their GPU render tests; partially restored in
   `270a0003` (+289) but RollingFire's own render coverage is still thin
   (`render_owned_chain_frames` only exercises loud/quiet payloads);
   (b) transparency contracts weakened from min-alpha ≤ 48/16 to “>10 %
   of pixels < 128”;
   (c) production always simulates the JS-full grid (~432 MB `Rgba16Float`,
   no override) — weak iGPU/software-adapter hosts get no escape hatch;
   (d) R-33 (`boxScale` never scales sphere/logo/emission) **still open**
   after the rewrite;
   (e) straight-alpha scene targets are composited as premultiplied
   (pre-existing engine-wide): scene RGB is additively blended at full
   strength where the canvas would contribute `rgb × alpha` — bright layers
   over bright content run hot; black background rows are unaffected today.
   Needs an explicit premultiply/un-premultiply decision.
3. **R-35 (PARTIAL)** — pipeline pre-build before the transition frame +
   same-frame downgrade on *prepare* failure (good), but that failure path
   is unreachable in this wgpu (no error scopes; `failed[]` has no writer),
   and the **apply-time** failure path in `render_frame_retaining` still
   shows one unblended frame, drops `transition_from` silently and
   downgrades next frame with a misattributed WARN. Close it in-frame at
   the failure site, or wire wgpu error scopes.
4. **Expressions P2s** — (a) discoverable `__modvExpressionFunctions` with
   predictable keys (NON_ENUMERABLE + freeze the map, or unguessable keys);
   (b) every-frame writes in nondeterministic `HashMap` order vs JS
   first-wins semantics; (c) lodash path gaps (`props.nested.0` resolves to
   `null`; `"a..b"` collapses); (d) `restore()` drops expression deltas
   (mirror stale; heals only via sync); (e) shared long-lived realm —
   prototype pollution poisons all later expressions; no realm refresh path.
5. **Plugin host P2s** — (a) plugins can claim core mutation types and
   shadow them to the mirror only (desync + double-fire on replay) —
   require namespaced modules or reject non-plugin types; (b) engine
   commits to unimplemented types vanish with zero renderer-side signal;
   (c) async `preProcessFrame` rejections bypass the once-per-id warn and
   flood the unhandledrejection log at rAF cadence;
   (d) unsupported plugins get a dead, silent checkbox in the panel.
6. **R-33** — SmokeRoom/DvdRoom lava sphere + logo (and the emission seed
   matrix) are drawn at identity instead of `boxScale ×` the JS `_boxGroup
   .scale`; the wind falloff already follows `boxScale`, so wind and mesh
   disagree at non-1 scales.
7. **GStreamer packaging + MediaHub robustness (WS-10)** — add
   libgstreamer1.0-0, -plugins-base1.0-0, gstreamer1.0-plugins-base/good,
   gstreamer1.0-pipewire to the `.deb` `depends` (or dlopen-style
   loading); the Docker/toolchain images already install them. Also:
   **screen sources must clamp negotiated caps** (`MAX_DIMENSION` applies
   only to requested dims — a 65535×65535 node means a ~17 GB reserve →
   abort); the 1 s start timeout must not kill slow prerolls (module doc
   says "timeout is not a failure" — code is stricter); stalled PipeWire
   nodes/V4L2 devices never error out (no frame deadline); dead cameras
   stay open after `FlowError::Error` (no bus message → never moved to
   Null); live-camera tests cannot fail.
8. **Media reconciliation & restart** — `loop:false` files replay forever;
   a failed re-`start` drops the `order` slot and re-indexes every later
   source (Webcam prop moves silently); `schedule_restart` runs inline on
   the render/heartbeat thread (2 s joins, 1 s bus wait per retry, can
   starve the 5 s watchdog); no permanent-failure latch (retries forever at
   cap); media is not re-armed after an engine restart (panel closed → no
   restart); the `media/list-cameras` failure case stops every selected
   camera; the Re-scan path ignores saved videoSettings; file starts before
   the media-manager-state replay are silently dropped; `media-mirror` can't
   represent CLEAR/RESET; fixed 5 s restart interval with no jitter/cap.
9. **Media path security** — `media/start-file` accepts any path/URIs
    (`https://…` network fetch, `..` escapes) with no canonicalisation/
    containment, contradicting its own comment.

**Closed in the fix wave, re-verified**
- ~~R-31~~ — hoop z-scale fixed, regression test, JS parity verified.
- ~~R-36~~ — Fade fallback lerps from eased progress (JS parity), resize path
  covered by construction, 50 % midpoint test.
- ~~deb linkage~~ — `libpipewire-0.3-0` declared in `deb.depends`.
- ~~R-1~~ — ISF trailing implicit pass (30 shaders) fixed + regression test;
  corpus now validates 351/351 (with the Light Leak caveat, item 17).
- ~~R-4~~ — cold-start timeout closed via `engine/get-sync` + idempotent
  replay.
- ~~R-10, R-11, R-12, R-13, R-14, R-15, R-16, R-17, R-18, R-19, R-20, R-21,
  R-24, R-25~~ — see Round 5.
- ~~R-2/R-3~~ — negotiation fixed; only the mid-stream death residual above.

**P3 / hygiene**
8. `restore()` leaves `engine_meter.pending` stale (store a fresh one or
   drop it on restore).
9. `remove_input` doesn't purge Scene-restoration tween descriptors from
   `state.tweens` — unbounded growth over a long performance (R-8 residual);
   `state.tweens` also silently shrinks on a non-finite BPM write (bounded,
   warn-only).
10. PipeWire mid-stream rate renegotiation leaves the startup-rate analyser
    in place (silent mis-analysis — undocumented).
11. `texture_pool.rs:229` — zero-size pool key → zero `bytes_per_row`
    (wgpu panic).
12. `stroke_text` re-uploads the same mask ~13× per frame.
13. Engine-client `onDelta/onState/onStateSync` drop previous listeners
    instead of unsubscribing; `engine/get-state` skips the sender check.
14. BPM panel still offers a “midi” source (no-op commitment); `setupMidi()`
    commits device actions into the void; plugins/font panels silently render
    empty in engine mode (boot warning only); plugin-host async hook guard
    gap; plugin install failure aborts the rest of installation; unknown
    `plugins/*` actions fall through to the engine void; `preProcessFrame`
    gets stale features (one-frame-late vs worker); camelized namespace
    collision leaves inert-but-listed plugin.
15. Docs arithmetic: progress doc now says **429** registered modules
    (37+15+26+351 — accurate) but `store_ops` test constant still says
    `37+15+18`; `isf-translation-notes.md` still says 63/43/16 (it is
    46/46/15); progress doc claims 1630→**1777** Rust tests; "947" Vitest
    in the progress doc vs "**1005**" in CHANGES.md; `rust-hybrid.md` still
    carries stale figures; **WS-12/expressions docs contradict the shipped
    Boa evaluator** — progress doc and `rust-hybrid.md` still say expressions
    are not evaluated by the engine; they are, since `0ae8943c` (Round 8),
    with the budget/sandbox caveat.
16. Whole-scene Fade interleaves incoming **between** same-row outgoing
    groups; JS appends incoming after outgoing (R-29 deviation, in-code
    comment only).
17. ISF “351/351” caveat: the corpus validates to naga, but **v002 Light
    Leak** renders with its rotation silently dead (sidecar matrix varying is
    rebuilt from zeros) — naga-passing is not the same as effect-working;
    consider failing that shader loudly or shipping one override. Also ship
    no UI signal for "effect cannot fully render" in engine mode.
18. ISF translator residuals (P3): `apply_edits` silently drops
    out-of-order edits; clone of sampler-parameter functions has no
    compile-time guard; a call before a sampler-parameter function's
    definition is never inlined; the hoist doc over-claims about forward
    references.
19. Plugin-host residuals: raw `plugins/SET_PLUGIN_ENABLE` commit bypasses
    the unsupported guard; throwing `init`/`shutdown` wedges the enabled state;
    hosted `plugins/*` three actions only — the rest go to the engine;
    no guard against `plugins.*` namespace collisions on Vuex register;
    test suite lacks async-reject/shutdownAll/collision coverage.
20. R-36 leftovers: stale doc on `fall_back_scene_transition_to_fade`;
    resize→downgrade window glue has no test; R-35 apply-time failure path
    unreachable-in-wrapper claim now inaccurate — see item 3.

## Round log

| Round | Commit reviewed | What it added | Notes |
|-------|-----------------|---------------|-------|
| 1 | `e8d78f18` (worktree, as handed to the reviewer) | Entire Rust workspace, JS integration, tests, build files | Worktree deleted externally mid-review; findings re-verified against `3548101f` by reading; docs/build drift noted |
| 2 | `e8d78f18..3548101f` (delta, from `git archive`) | Tweens, Scene hover previews, engine sync wiring, Point10 batch 5a (21/25) | All P1s from round 1 re-verified still present; new tween and preview findings R-5..R-11 |
| 3 | `3548101f..c088335d` (delta) | Point10 batch 5b (25/25), `--audio-source` engine-owned capture wiring, installer plumbing | New P1 R-31 (tunnel hoop depth); R-32..R-34 parity findings; packaging gap (no libpipewire in deb depends); docs/tests coverage gaps |
| 4 | `c088335d..36672a33` (delta) | Shader whole-Scene transitions (crossfade/dissolve/pixelate/glitch/wipe/circle), progress docs, `M7 — in progress` status | No P1; new R-35..R-36 fallback-path defects; transition window glue untested; fallback-Fade alpha snap |
| 5 | `36672a33..916586f5` (fix wave, 7 commits) | Review-driven fixes across all crates + JS | R-1..R-30 re-verified; residuals listed in the Round 5 section; **R-31..R-36 NOT FIXED**; new caveat: PipeWire mid-stream death still silent; module-count docs still wrong (424→428 unfixed) |
| 6 | `916586f5..73729fdb` (delta) | Point10 textures shipped for packaged builds | Assets fix clean; `.deb` still undeclared `libpipewire-0.3-0` |
| 7 | `73729fdb..9afa21dc` (delta) | ISF “351/351” (four new translation passes, no overrides), gallery effect previews over a test card | Passes verified per-shader; caveat: v002 Light Leak validates with its rotation silently dead — “351/351” ≠ “351/351 render correctly”; gallery fix clean |
| 8 | `9afa21dc..89f8acc9` (delta) | R-31/R-35/R-36 fixes, `.deb` PipeWire dep, Boa expressions (WS-12), JS plugin host | R-31/R-36/deb verified fixed; R-35 partial; new P1 (expression evaluation budget); plugin-host P2s; WS-12 docs still contradict the shipped feature |
| 9 | `89f8acc9..418c5cc8` (delta) | GStreamer video capture core (WS-10), docs commit | No P1; GStreamer FFI/pool/escaping clean; packaged engine undeclared GStreamer deps; start timeout aborts slow prerolls; stalled sources silent; screen caps unbounded |
| 10 | `418c5cc8..e88bbd07` (delta) | Engine-owned media sources (renderer + engine), WS-10 end-to-end | **2 new P1s** (media path rooting breaks files on POSIX; TextureControl lifetime freezes file video); engine/renderer contract tight (sender validation, error codes, wire shapes all verified); MediaHub P2s (loop:false replay, order-drop on failed start, thread-blocking restart, no cap); camera-order divergence repointing Webcam props |
| 11 | `e88bbd07..28d90158` (delta) | Webcam camera index → camera-like sources (files excluded); Point10 SmokeRoom/DvdRoom re-ported to a GPU fluid sim + raymarch | Camera-index fix verified (engine side); R-28 fix verified; R-32 fix verified; **R-33/R-34 still open**; volume rewrite deleted RollingFire/GeometricTunnel GPU tests and loosened transparency contracts |
| 12 | `28d90158..270a0003` (delta) | RollingFire re-ported to the volume sim (R-34 fix); BallOfFire registered as 26th scene; docs counts | R-34 verified fixed (CPU-gated only — no GPU silent test); BallOfFire verified end-to-end; straight-alpha scene targets composite as premultiplied (pre-existing engine-wide); SmokeRoom/DvdRoom never bind `uSway`; GPU tests pass vacuously without an adapter |

<sub>Review file maintained incrementally; earlier passes are not retracted,
only extended. Last full pass: `270a0003` — 2026-11-12.</sub>

## Round 13 — `270a0003..HEAD` (validation fix wave)

- **Expressions P1 closed** — `expr` source is capped (`MAX_EXPRESSION_SOURCE`)
  with a denylist for native allocation/loop/context escapes (`Array.from`,
  `repeat/fill/map/forEach/reduce`, `Function/eval`, `globalThis`,
  `setTimeout`, `require`, …); the compiled-function map uses an unguessable,
  non-enumerable global key; `rebuild` disposes the JS realm (no prototype
  pollution across presets); every-frame writes follow document order instead
  of `HashMap` order. Hostile sources are rejected at compile time (tested);
  ordinary maths still evaluates (≈0.7 µs).
- **Media P1s closed** — media-library paths (`/video/clip.mp4`) are re-rooted
  under `media.path` (known folders only, `..` rejected) while true absolute
  paths pass through; engine video sources are owned by the texture/module and
  no longer stopped when the inspector unmounts; `media/start-file` rejects
  URI schemes, relative paths and `..`; non-looping files park at their end
  instead of replaying forever; frames larger than 8192² are refused before
  the texture pool reserves them.
- **PipeWire P2 closed** — the monitor stream now latches `state_changed`
  errors/disconnects into the existing error channel, so a mid-stream server
  restart falls back to the renderer pump instead of reporting silence.
- **Camera enumeration crash fixed** — `media/list-cameras` crashed the
  packaged engine with SIGSEGV: GStreamer's `DeviceMonitor` instantiates the
  libcamera provider on a GStreamer 1.28 host. Enumeration now queries V4L2
  directly (`v4l` + sysfs), filters non-capturable metadata nodes, and lists
  `/dev/video0` + `/dev/video2` with their full caps. Verified in the packaged
  AppImage (`MODV_ENGINE=1`), including a live Webcam module fed by
  `/dev/video0`.
- **R-33 closed for SmokeRoom** — the emitter mesh/seed matrix now scales with
  `boxScale` (the JS `_boxGroup.setScalar`), matching the wind-falloff radius
  (`u.origin.w`), with a regression test.
- **Packaging** — the `.deb` declares the GStreamer runtime packages
  (`libgstreamer1.0-0`, plugins base/good/libav) alongside
  `libpipewire-0.3-0`; the AppImage still needs them from the host (the legacy
  renderer does not), documented in AGENTS/progress.
- **Input Link mirror flush** — a value that settled inside the 100 ms delta
  throttle was cleared as "unchanged" before it reached the mirror, so the UI
  slider could stick at an intermediate value while the engine rendered the
  final one (reproduced by the engineParity bass link: mirror 1.37, engine
  0.63). Unchanged writes now flush the current value for module props, meta
  values and group controls, with a regression test.
- **Volumetric fidelity** — SmokeRoom, DvdRoom, RollingFire and BallOfFire now
  run the real GPU volume sim + raymarch; BallOfFire is registered (26
  scenes); side-by-side captures against the legacy AppImage look structurally
  right (turbulent warm fields, raymarch strata, legible DVD wordmark, frozen
  lava core). GeometricTunnel hoop scale (R-31) and same-frame/fade transition
  fallbacks (R-35/R-36) are fixed with tests.
- **Still open (documented):** straight-alpha Point10/ISF targets composited
  through a premultiplied graph (black backdrop rows unaffected; needs an
  explicit composite-time premultiply decision), engine-mode screen capture
  (Electron portal node id), R-35 apply-time failure path, plugin-host P2s,
  and the media-health nits (inline restart, permanent-failure latch).
