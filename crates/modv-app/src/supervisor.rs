//! WS-14 / M1–M2 — the engine side of the control channel and the preview ring.
//!
//! One [`EngineSupervisor`] owns:
//! * the non-blocking `SocketServer` bound to `--socket`;
//! * the `VMP1` preview ring in `/dev/shm` (published to clients over the
//!   socket as `preview/ready` + `preview/frame` events);
//! * the state snapshot written to `--state` (crash-safe: tmp file + rename),
//!   reloaded at startup so a supervisor restart restores the last state;
//! * the heartbeat the Electron supervisor watchdog keys off
//!   (`engine/heartbeat` every second).
//!
//! ## M2 changes relative to M1
//!
//! * The state itself lives in [`Store`](crate::store_ops::Store); the
//!   supervisor no longer fakes dispatch results or holds a `Snapshot` struct.
//!   `state/snapshot` returns the six-key preset document plus the engine
//!   counters, `state/commit`/`state/dispatch` call `Store::dispatch`, and
//!   `engine/restore` reloads the state (from the request or the state file).
//! * Every successful mutation is broadcast as one `state/delta` event with
//!   the `{ type, payload }` record shape the renderer's mirror replays
//!   (`worker/index.worker.js:136-144`).
//! * `state/commit` and `state/dispatch` are the same code path: M2 has no
//!   separate action layer (inputs/expressions add one in WS-1b/WS-12), so a
//!   commit is dispatched and its return value is only sent to the caller.
//! * The preview no longer publishes a flat clear colour: the window loop asks
//!   the compositor for a real frame (`Compositor::read_preview`) and hands the
//!   written [`PreviewFrame`] back to [`EngineSupervisor::publish_preview`].
//!
//! `engine/shutdown` is still the only request with side effects beyond the
//! store: persist the snapshot, then signal the event loop to exit.

use std::collections::{HashSet, VecDeque};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use modv_ipc::frame::{KIND_EVENT, KIND_REQUEST};
use modv_ipc::message::{Event, Incoming, Request, Response, from_frame};
use modv_ipc::preview::{PreviewFrame, PreviewRing};
use modv_ipc::socket::SocketServer;

use crate::compositor::{PREVIEW_HEIGHT, PREVIEW_WIDTH};
use crate::module_preview::{MAX_PREVIEW_DIMENSION, PreviewError, base64_encode};
use crate::scene_preview::MAX_SCENE_PREVIEW_DIMENSION;
use crate::store_ops::{Store, frames_presented_in};

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(1);
const PREVIEW_INTERVAL: Duration = Duration::from_millis(33); // ≤ 30 fps
const SNAPSHOT_SAVE_EVERY_FRAMES: u64 = 120;

/// Most preview requests (`preview/module` + `preview/scene`) allowed to wait
/// for the window loop.
///
/// Gallery tiles poll at ~10 fps each and a Scene hover at ~30 fps; a handful
/// of surfaces never reaches this. A request that would overflow the queue is
/// answered with `engine_busy` immediately instead of growing memory (spec:
/// bounded queues only).
pub const MAX_PENDING_PREVIEWS: usize = 512;

/// Which preview a queued request renders.
///
/// Both kinds share one bounded queue and the same per-frame cap, so a hover of
/// a Gallery tile and a Scene cell competes fairly for the window loop's time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreviewKind {
    /// A Gallery thumbnail for a registered module (`preview/module`).
    Module {
        /// Registry name of the module to render.
        module_name: String,
    },
    /// A Scene grid-cell preview (`preview/scene`).
    Scene {
        /// Id of the Scene in the store.
        scene_id: String,
        /// Grid row (the cell) to render.
        row_index: usize,
    },
}

/// A queued preview request.
///
/// The supervisor cannot render (it owns no GPU device), so
/// [`EngineSupervisor::handle_request`] validates the payload and parks it
/// here; the window loop drains the queue after `render_frame` and answers it
/// through [`EngineSupervisor::complete_preview`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreviewRequest {
    /// Request id the response must carry.
    pub id: u64,
    /// Client that sent it, for [`SocketServer::send_response`].
    pub client: usize,
    /// Requested (already clamped) width.
    pub width: u32,
    /// Requested (already clamped) height.
    pub height: u32,
    /// What to render.
    pub kind: PreviewKind,
}

/// How [`EngineSupervisor::handle_request`] answered a request.
pub enum Reply {
    /// A complete response; `poll` sends it immediately.
    Now(Response),
    /// The response is produced later (a queued `preview/module` render); `poll`
    /// sends nothing now.
    Deferred,
}

/// The engine-side supervision state. Created before the window exists and
/// polled from the frame loop.
pub struct EngineSupervisor {
    server: SocketServer,
    state_path: Option<PathBuf>,
    frames_presented: u64,
    frames_since_save: u64,
    restored_frames: u64,
    started: Instant,
    last_heartbeat: Instant,
    last_preview: Option<Instant>,
    preview: Option<PreviewRing>,
    /// Set when the ring could not be created; the preview channel is optional
    /// (the UI falls back to nothing), so retrying every frame would only spam
    /// the log.
    preview_unavailable: bool,
    shutdown_requested: bool,
    /// Set by `engine/restore`; the window loop drops the compositor's GPU
    /// state (stale group textures and module slots) when it sees this.
    reset_requested: bool,
    /// `preview/module` and `preview/scene` requests waiting for the window
    /// loop's compositor.
    pending_previews: VecDeque<PreviewRequest>,
    unknown_dispatches_warned: HashSet<String>,
}

impl EngineSupervisor {
    /// Bind the control socket and read the restored frame count, if any.
    pub fn new(socket_path: PathBuf, state_path: Option<PathBuf>) -> io::Result<Self> {
        let server = SocketServer::bind(&socket_path)?;
        let restored_frames = state_path
            .as_ref()
            .and_then(|path| load_snapshot_frames(path))
            .unwrap_or(0);

        if restored_frames > 0 {
            eprintln!("engine: INFO restored state snapshot (frames_presented={restored_frames})");
        }

        Ok(Self {
            server,
            state_path,
            frames_presented: restored_frames,
            frames_since_save: 0,
            restored_frames,
            started: Instant::now(),
            last_heartbeat: Instant::now(),
            last_preview: None,
            preview: None,
            preview_unavailable: false,
            shutdown_requested: false,
            reset_requested: false,
            pending_previews: VecDeque::new(),
            unknown_dispatches_warned: HashSet::new(),
        })
    }

    /// Whether `engine/shutdown` has been received.
    pub fn shutdown_requested(&self) -> bool {
        self.shutdown_requested
    }

    /// Take the "GPU state must be reset" flag set by `engine/restore`.
    pub fn take_reset_request(&mut self) -> bool {
        std::mem::take(&mut self.reset_requested)
    }

    /// Record a presented frame; checkpoints the snapshot periodically.
    pub fn note_frame_presented(&mut self, store: &Store) {
        self.frames_presented += 1;
        self.frames_since_save += 1;

        if self.frames_since_save >= SNAPSHOT_SAVE_EVERY_FRAMES {
            if let Err(error) = self.save_snapshot(store) {
                eprintln!("engine: WARN could not save snapshot: {error}");
            }
        }
    }

    /// Persist the snapshot atomically (tmp file + rename).
    pub fn save_snapshot(&mut self, store: &Store) -> io::Result<()> {
        self.frames_since_save = 0;
        let Some(path) = &self.state_path else {
            return Ok(());
        };
        store.save_snapshot(path, self.frames_presented)
    }

    /// Poll the socket and publish scheduled heartbeat events.
    ///
    /// Requests are applied to `store`; every mutation they queue is
    /// broadcast as a `state/delta` event before the next request is handled,
    /// so the mirror sees the records in issue order.
    pub fn poll(&mut self, store: &mut Store) {
        let poll = match self.server.poll() {
            Ok(poll) => poll,
            Err(error) => {
                eprintln!("engine: WARN control socket poll failed: {error}");
                return;
            }
        };

        // Announce the preview channel to freshly connected clients, then
        // hand them the full state: the engine seeds Scenes and applies
        // documents before the UI connects, so the delta stream alone would
        // leave the mirror stale.
        for client in poll.accepted {
            let response = self.preview_ready_event();
            if let Ok(event) = response {
                let payload = serde_json::to_vec(&event).expect("preview/ready serialises");
                let _ = self.server.send(client, KIND_EVENT, &payload);
            }

            let sync = Event::new("state/sync", store.sync_state());
            match serde_json::to_vec(&sync) {
                Ok(payload) => {
                    if let Err(error) = self.server.send(client, KIND_EVENT, &payload) {
                        eprintln!("engine: WARN could not send state/sync: {error}");
                    }
                }
                Err(error) => eprintln!("engine: WARN could not serialise state/sync: {error}"),
            }
        }

        for frame in poll.frames {
            if frame.kind != KIND_REQUEST {
                continue;
            }
            match from_frame(frame.kind, &frame.payload) {
                Ok(Incoming::Request(request)) => {
                    let reply = self.handle_request(request, store, frame.client);
                    // Deltas first: the caller then observes every mutation
                    // this request produced before its own acknowledgement, so
                    // the mirror never applies an ack it cannot explain.
                    self.broadcast_deltas(store);
                    // A deferred request is answered by the window loop after
                    // it has rendered (it owns the compositor).
                    if let Reply::Now(response) = reply {
                        if let Err(error) = self.server.send_response(frame.client, &response) {
                            eprintln!("engine: WARN could not send response: {error}");
                        }
                    }
                }
                Ok(_) | Err(_) => {
                    eprintln!("engine: WARN ignoring malformed request frame");
                }
            }
        }

        self.maybe_heartbeat();
    }

    /// Broadcast every queued `{ type, payload }` record as `state/delta`.
    pub(crate) fn broadcast_deltas(&mut self, store: &mut Store) {
        for mutation in store.drain_deltas() {
            let event = Event::new(
                "state/delta",
                json!({ "type": mutation.r#type, "payload": mutation.payload }),
            );
            if let Err(error) = self.server.broadcast_event(&event) {
                eprintln!("engine: WARN state/delta broadcast failed: {error}");
            }
        }
    }

    /// Dispatch one request.
    ///
    /// `preview/module` and `preview/scene` cannot be answered here — the
    /// supervisor has no compositor — so they are validated and queued
    /// ([`Reply::Deferred`]); every other method answers immediately
    /// ([`Reply::Now`]).
    fn handle_request(&mut self, request: Request, store: &mut Store, client: usize) -> Reply {
        match request.method.as_str() {
            "preview/module" => self.queue_module_preview(request, client),
            "preview/scene" => self.queue_scene_preview(request, client),
            _ => Reply::Now(self.handle_request_now(request, store)),
        }
    }

    /// Validate and queue a `preview/module` request for the window loop.
    ///
    /// The payload is `{ moduleName, width, height }`; widths/heights are
    /// clamped to `1..=512` (the main process already clamps, but the engine
    /// must not trust a peer). An unknown module is *not* rejected here: the
    /// compositor reports `module_not_found`, so the renderer gets the same
    /// error shape whether the registry or the renderer is at fault.
    fn queue_module_preview(&mut self, request: Request, client: usize) -> Reply {
        if self.shutdown_requested {
            return Reply::Now(Response::err(
                request.id,
                "engine_shutting_down",
                "the engine is shutting down",
            ));
        }

        let module_name = match request.params.get("moduleName").and_then(Value::as_str) {
            Some(name) if !name.is_empty() => name.to_string(),
            _ => {
                return Reply::Now(Response::err(
                    request.id,
                    "invalid_params",
                    "preview/module needs a non-empty string \"moduleName\"",
                ));
            }
        };
        let (Some(width), Some(height)) = (
            preview_dimension(request.params.get("width"), MAX_PREVIEW_DIMENSION),
            preview_dimension(request.params.get("height"), MAX_PREVIEW_DIMENSION),
        ) else {
            return Reply::Now(Response::err(
                request.id,
                "invalid_params",
                "preview/module needs numeric \"width\" and \"height\"",
            ));
        };

        if self.pending_previews.len() >= MAX_PENDING_PREVIEWS {
            return Reply::Now(Response::err(
                request.id,
                "engine_busy",
                "too many module preview requests are queued",
            ));
        }

        self.pending_previews.push_back(PreviewRequest {
            id: request.id,
            client,
            width,
            height,
            kind: PreviewKind::Module { module_name },
        });
        Reply::Deferred
    }

    /// Validate and queue a `preview/scene` request for the window loop.
    ///
    /// The payload is `{ sceneId, rowIndex, width, height }`; sizes are
    /// clamped to `1..=480` (the UI caps a cell at 480 × 270) and `rowIndex`
    /// must be a non-negative integer. An unknown Scene or an empty row is
    /// *not* rejected here: the store lookup/render reports `scene_not_found`
    /// or `scene_row_empty`, which is the same error shape the renderer sees
    /// for every other preview failure.
    fn queue_scene_preview(&mut self, request: Request, client: usize) -> Reply {
        if self.shutdown_requested {
            return Reply::Now(Response::err(
                request.id,
                "engine_shutting_down",
                "the engine is shutting down",
            ));
        }

        let scene_id = match request.params.get("sceneId").and_then(Value::as_str) {
            Some(id) if !id.is_empty() => id.to_string(),
            _ => {
                return Reply::Now(Response::err(
                    request.id,
                    "invalid_params",
                    "preview/scene needs a non-empty string \"sceneId\"",
                ));
            }
        };
        let Some(row_index) = request
            .params
            .get("rowIndex")
            .and_then(Value::as_u64)
            .and_then(|row| usize::try_from(row).ok())
        else {
            return Reply::Now(Response::err(
                request.id,
                "invalid_params",
                "preview/scene needs a non-negative integer \"rowIndex\"",
            ));
        };
        let (Some(width), Some(height)) = (
            preview_dimension(request.params.get("width"), MAX_SCENE_PREVIEW_DIMENSION),
            preview_dimension(request.params.get("height"), MAX_SCENE_PREVIEW_DIMENSION),
        ) else {
            return Reply::Now(Response::err(
                request.id,
                "invalid_params",
                "preview/scene needs numeric \"width\" and \"height\"",
            ));
        };

        if self.pending_previews.len() >= MAX_PENDING_PREVIEWS {
            return Reply::Now(Response::err(
                request.id,
                "engine_busy",
                "too many preview requests are queued",
            ));
        }

        self.pending_previews.push_back(PreviewRequest {
            id: request.id,
            client,
            width,
            height,
            kind: PreviewKind::Scene {
                scene_id,
                row_index,
            },
        });
        Reply::Deferred
    }

    /// Take every queued preview request (the window loop answers them).
    pub fn pending_previews(&mut self) -> Vec<PreviewRequest> {
        self.pending_previews.drain(..).collect()
    }

    /// Return unanswered requests to the front of the queue, in order.
    ///
    /// The window loop answers a bounded number per frame, so a burst of tile
    /// requests rolls over to the next frame instead of stalling present.
    pub fn requeue_previews(&mut self, requests: Vec<PreviewRequest>) {
        for request in requests.into_iter().rev() {
            self.pending_previews.push_front(request);
        }
    }

    /// Send the answer for a preview request the window loop rendered.
    ///
    /// Success is `{ width, height, pixels }` with `pixels` the base64 of
    /// `width × height × 4` tightly packed RGBA8 bytes — the same shape for
    /// `preview/module` and `preview/scene`. A failure is the renderer-facing
    /// error code (`module_not_found`, `scene_not_found`, `scene_row_empty`,
    /// `preview_failed`).
    pub fn complete_preview(
        &mut self,
        request: PreviewRequest,
        result: Result<Vec<u8>, PreviewError>,
    ) {
        let response = match result {
            Ok(pixels) => {
                let expected = request.width as usize * request.height as usize * 4;
                if pixels.len() == expected {
                    Response::ok(
                        request.id,
                        json!({
                            "width": request.width,
                            "height": request.height,
                            "pixels": base64_encode(&pixels),
                        }),
                    )
                } else {
                    Response::err(
                        request.id,
                        "preview_failed",
                        format!(
                            "preview returned {} bytes, expected {expected}",
                            pixels.len()
                        ),
                    )
                }
            }
            Err(error) => Response::err(request.id, error.code(), error.message()),
        };

        if let Err(error) = self.server.send_response(request.client, &response) {
            eprintln!("engine: WARN could not send preview response: {error}");
        }
    }

    /// Error every queued preview request (shutdown or a fatal render failure).
    pub fn reject_pending_previews(&mut self, code: &str, message: &str) {
        for request in std::mem::take(&mut self.pending_previews) {
            let response = Response::err(request.id, code, message);
            if let Err(error) = self.server.send_response(request.client, &response) {
                eprintln!("engine: WARN could not send preview error: {error}");
            }
        }
    }

    fn handle_request_now(&mut self, request: Request, store: &mut Store) -> Response {
        match request.method.as_str() {
            "engine/shutdown" => {
                eprintln!("engine: INFO shutdown requested; flushing state");
                if let Err(error) = self.save_snapshot(store) {
                    eprintln!("engine: WARN shutdown snapshot failed: {error}");
                }
                self.shutdown_requested = true;
                // Nothing will render another frame: answer the queued
                // thumbnails instead of leaving their callers to time out.
                self.reject_pending_previews("engine_shutting_down", "the engine is shutting down");
                Response::ok(request.id, json!({ "saved": true }))
            }

            "state/snapshot" => {
                let mut snapshot = store.snapshot();
                if let Value::Object(map) = &mut snapshot {
                    map.insert("framesPresented".to_string(), json!(self.frames_presented));
                    map.insert("restoredFrames".to_string(), json!(self.restored_frames));
                    map.insert(
                        "uptimeSeconds".to_string(),
                        json!(self.started.elapsed().as_secs_f64()),
                    );
                }
                Response::ok(request.id, snapshot)
            }

            // The `state/sync` event a client receives on accept is not queued
            // by Electron's IPC, so a renderer that subscribes late loses it
            // and its mirror stays stale (R-4). This is the idempotent pull:
            // the same payload the accept path sends, replayable at any time.
            "engine/get-sync" => Response::ok(request.id, store.sync_state()),

            // `state/commit` and `state/dispatch` are the same path in M2: the
            // renderer sends both over the same channels and there is no
            // action layer to distinguish them yet.
            "state/commit" | "state/dispatch" => {
                let action = match request.params.get("type").and_then(Value::as_str) {
                    Some(action) => action.to_string(),
                    None => {
                        self.warn_unknown("<missing type>");
                        return Response::err(
                            request.id,
                            "invalid_params",
                            "state/commit and state/dispatch need a string \"type\"",
                        );
                    }
                };
                let payload = request
                    .params
                    .get("payload")
                    .cloned()
                    .unwrap_or(Value::Null);

                match store.dispatch(&action, payload) {
                    Ok(result) => Response::ok(request.id, result),
                    Err(error) => {
                        eprintln!("engine: WARN dispatch {action} failed: {error}");
                        Response::err(request.id, "dispatch_failed", error.to_string())
                    }
                }
            }

            "engine/restore" => {
                let preset = request
                    .params
                    .get("preset")
                    .cloned()
                    .or_else(|| self.load_snapshot_file());

                match preset {
                    Some(preset) => match store.restore(&preset) {
                        Ok(()) => {
                            if let Some(frames) = frames_presented_in(&preset) {
                                self.restored_frames = frames;
                                self.frames_presented = frames;
                            }
                            // Group ids and module ids can change completely;
                            // the compositor must not keep the old textures or
                            // pipeline slots.
                            self.reset_requested = true;
                            Response::ok(request.id, json!({ "restored": true }))
                        }
                        Err(error) => {
                            eprintln!("engine: WARN engine/restore failed: {error}");
                            Response::err(request.id, "restore_failed", error.to_string())
                        }
                    },
                    None => Response::ok(request.id, json!({ "restored": false })),
                }
            }

            other => Response::err(
                request.id,
                "unknown_method",
                format!("no such method: {other}"),
            ),
        }
    }

    fn load_snapshot_file(&self) -> Option<Value> {
        let path = self.state_path.as_ref()?;
        let bytes = fs::read(path).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    fn warn_unknown(&mut self, action: &str) {
        if self.unknown_dispatches_warned.insert(action.to_string()) {
            eprintln!("engine: WARN dispatch {action} missing its type (M2)");
        }
    }

    fn maybe_heartbeat(&mut self) {
        if self.last_heartbeat.elapsed() < HEARTBEAT_INTERVAL {
            return;
        }
        self.last_heartbeat = Instant::now();

        let event = Event::new(
            "engine/heartbeat",
            json!({
                "uptimeSeconds": self.started.elapsed().as_secs_f64(),
                "framesPresented": self.frames_presented,
                "version": env!("CARGO_PKG_VERSION"),
            }),
        );
        if let Err(error) = self.server.broadcast_event(&event) {
            eprintln!("engine: WARN heartbeat broadcast failed: {error}");
        }
    }

    fn ensure_preview(&mut self) -> Option<&PreviewRing> {
        if self.preview.is_none() {
            if self.preview_unavailable {
                return None;
            }
            match PreviewRing::create(None, 3, PREVIEW_WIDTH, PREVIEW_HEIGHT) {
                Ok(ring) => {
                    eprintln!(
                        "engine: INFO preview ring ready at {} ({}x{}, 3 slots)",
                        ring.path().display(),
                        PREVIEW_WIDTH,
                        PREVIEW_HEIGHT
                    );
                    self.preview = Some(ring);
                }
                Err(error) => {
                    eprintln!("engine: WARN could not create preview ring: {error}");
                    self.preview_unavailable = true;
                    return None;
                }
            }
        }

        self.preview.as_ref()
    }

    fn preview_ready_event(&mut self) -> io::Result<Event> {
        let ring = self
            .ensure_preview()
            .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "preview ring unavailable"))?;
        Ok(Event::new("preview/ready", ring.handshake()))
    }

    /// Whether a preview frame is due (≤ 30 fps cadence).
    pub fn preview_due(&self) -> bool {
        match self.last_preview {
            Some(last) => last.elapsed() >= PREVIEW_INTERVAL,
            None => true,
        }
    }

    /// The preview ring to write the next frame into, creating it on demand.
    pub fn preview_ring_mut(&mut self) -> Option<&mut PreviewRing> {
        self.ensure_preview();
        self.preview.as_mut()
    }

    /// Publish the frame the compositor just wrote: broadcast `preview/frame`
    /// and start the next cadence window.
    pub fn publish_preview(&mut self, frame: PreviewFrame) {
        self.last_preview = Some(Instant::now());
        let event = Event::new("preview/frame", frame.to_event());
        if let Err(error) = self.server.broadcast_event(&event) {
            eprintln!("engine: WARN preview broadcast failed: {error}");
        }
    }
}

/// Read one requested preview edge: finite and positive, rounded, clamped to
/// `1..=max`.
///
/// `None` for a missing or non-numeric value (a malformed request), so the
/// caller can answer `invalid_params` rather than render an arbitrary size.
fn preview_dimension(value: Option<&Value>, max: u32) -> Option<u32> {
    let number = value?.as_f64()?;
    if !number.is_finite() {
        return None;
    }
    Some(number.round().clamp(1.0, f64::from(max)) as u32)
}

/// `framesPresented` from the on-disk snapshot's `_engine` sidecar.
fn load_snapshot_frames(path: &Path) -> Option<u64> {
    let bytes = fs::read(path).ok()?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    frames_presented_in(&value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;
    use std::time::{SystemTime, UNIX_EPOCH};

    use modv_ipc::frame::{FrameReader, KIND_RESPONSE};

    /// Poll the supervisor until a KIND_RESPONSE frame arrives from the
    /// stream; returns its payload bytes and any events seen on the way.
    fn read_response(
        supervisor: &mut EngineSupervisor,
        store: &mut Store,
        stream: &mut UnixStream,
    ) -> (Vec<u8>, Vec<Event>) {
        let mut reader = FrameReader::new();
        let mut events = Vec::new();
        for _ in 0..100 {
            supervisor.poll(store);
            let mut scratch = [0u8; 8192];
            match stream.read(&mut scratch) {
                Ok(n) if n > 0 => {
                    for frame in reader.push(&scratch[..n]) {
                        let (kind, payload) = frame.expect("frame decodes");
                        if kind == KIND_RESPONSE {
                            return (payload, events);
                        }
                        if kind == KIND_EVENT {
                            if let Ok(event) = serde_json::from_slice::<Event>(&payload) {
                                events.push(event);
                            }
                        }
                    }
                }
                Ok(_) => {}
                Err(e)
                    if e.kind() == io::ErrorKind::WouldBlock
                        || e.kind() == io::ErrorKind::TimedOut =>
                {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(e) => panic!("read failed: {e}"),
            }
        }

        panic!("timed out waiting for a response frame");
    }

    fn request(
        supervisor: &mut EngineSupervisor,
        store: &mut Store,
        stream: &mut UnixStream,
        id: u64,
        method: &str,
        params: Value,
    ) -> (Response, Vec<Event>) {
        stream
            .write_all(
                &Request {
                    id,
                    method: method.into(),
                    params,
                }
                .into_frame(),
            )
            .expect("write request");
        let (payload, events) = read_response(supervisor, store, stream);
        (
            serde_json::from_slice(&payload).expect("response parses"),
            events,
        )
    }

    fn temp_paths(label: &str) -> (PathBuf, PathBuf) {
        let unique = format!(
            "modv-app-test-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let socket = std::env::temp_dir().join(format!("{unique}.sock"));
        let state = std::env::temp_dir().join(format!("{unique}.json"));
        (socket, state)
    }

    fn connect(socket_path: &Path) -> UnixStream {
        let stream = UnixStream::connect(socket_path).expect("connect");
        stream
            .set_read_timeout(Some(Duration::from_millis(500)))
            .unwrap();
        stream
    }

    #[test]
    fn snapshot_roundtrip_restores_frame_count() {
        let (socket_path, state_path) = temp_paths("roundtrip");
        let mut store = Store::new();

        {
            let mut supervisor =
                EngineSupervisor::new(socket_path.clone(), Some(state_path.clone()))
                    .expect("supervisor binds");
            for _ in 0..7 {
                supervisor.note_frame_presented(&store);
            }
            supervisor.save_snapshot(&store).expect("snapshot saved");
            assert_eq!(supervisor.frames_presented, 7);
        }

        let restored = EngineSupervisor::new(socket_path, Some(state_path.clone()))
            .expect("supervisor rebinds");
        assert_eq!(restored.frames_presented, 7);

        // The snapshot file also loads into a fresh store.
        store
            .restore_from_file(&state_path)
            .expect("snapshot loads into the store");
        assert_eq!(store.snapshot()["version"], json!(1));

        let _ = fs::remove_file(&state_path);
    }

    #[test]
    fn snapshot_request_returns_state_and_frame_count() {
        let (socket_path, state_path) = temp_paths("snapshot-ipc");
        let mut store = Store::new();
        let mut supervisor =
            EngineSupervisor::new(socket_path.clone(), Some(state_path)).expect("supervisor binds");
        for _ in 0..3 {
            supervisor.note_frame_presented(&store);
        }
        store
            .dispatch("groups/createGroup", json!({"id": "g1", "name": "Effects"}))
            .expect("createGroup dispatches");
        store.drain_deltas();

        let mut stream = connect(&socket_path);
        let (response, _) = request(
            &mut supervisor,
            &mut store,
            &mut stream,
            42,
            "state/snapshot",
            Value::Null,
        );

        assert_eq!(response.id, 42);
        assert!(response.ok);
        let result = response.result.expect("result present");
        assert_eq!(result["framesPresented"], json!(3));
        assert_eq!(result["version"], json!(1));
        assert_eq!(result["groups"][0]["id"], json!("g1"));
    }

    #[test]
    fn get_sync_returns_the_mirror_relevant_state_idempotently() {
        let (socket_path, state_path) = temp_paths("get-sync");
        let mut store = Store::new();
        let mut supervisor =
            EngineSupervisor::new(socket_path.clone(), Some(state_path)).expect("supervisor binds");
        store
            .dispatch("groups/createGroup", json!({"id": "g1", "name": "Effects"}))
            .expect("createGroup dispatches");
        store.drain_deltas();

        let mut stream = connect(&socket_path);
        let (first, _) = request(
            &mut supervisor,
            &mut store,
            &mut stream,
            61,
            "engine/get-sync",
            Value::Null,
        );
        assert_eq!(first.id, 61);
        assert!(first.ok);
        let sync = first.result.expect("sync payload present");

        // The mirror rebuilds `registered`/`scenes` from this payload and
        // replays groups/modules/inputs; a missing key silently blanks a
        // surface (R-4).
        assert!(sync.get("groups").is_some_and(Value::is_array));
        assert!(sync.get("modules").is_some_and(Value::is_object));
        assert!(sync.get("registered").is_some_and(Value::is_object));
        assert!(sync.get("scenes").is_some_and(Value::is_array));
        assert!(sync.get("inputs").is_some_and(Value::is_object));
        assert!(sync.get("tweens").is_some());
        assert!(sync.get("expressions").is_some());
        assert_eq!(sync["groups"][0]["id"], json!("g1"));
        assert!(
            sync["registered"]
                .as_object()
                .is_some_and(|registered| !registered.is_empty()),
            "the registered module map ships with every sync"
        );

        // The pull is idempotent: a renderer that already replayed the accept
        // event can call it again without changing the payload.
        let (second, _) = request(
            &mut supervisor,
            &mut store,
            &mut stream,
            62,
            "engine/get-sync",
            Value::Null,
        );
        assert!(second.ok);
        let again = second.result.expect("second sync payload present");
        assert_eq!(again, sync);
    }

    #[test]
    fn create_group_dispatch_creates_a_group_and_emits_a_delta() {
        let (socket_path, state_path) = temp_paths("create-group");
        let mut store = Store::new();
        let mut supervisor =
            EngineSupervisor::new(socket_path.clone(), Some(state_path)).expect("supervisor binds");

        let mut stream = connect(&socket_path);
        let (response, events) = request(
            &mut supervisor,
            &mut store,
            &mut stream,
            11,
            "state/dispatch",
            json!({ "type": "groups/createGroup", "payload": { "name": "Gallery" } }),
        );

        assert!(response.ok);
        let result = response.result.expect("result present");
        assert_eq!(result["name"], json!("Gallery"));
        assert_eq!(result["modules"], json!([]));
        assert_eq!(store.state().groups.len(), 1);

        let delta = events
            .iter()
            .find(|event| event.event == "state/delta")
            .expect("state/delta is broadcast");
        assert_eq!(delta.data["type"], json!("groups/ADD_GROUP"));
        assert_eq!(delta.data["payload"]["group"]["name"], json!("Gallery"));
    }

    #[test]
    fn state_commit_is_dispatched_and_emits_a_delta() {
        let (socket_path, state_path) = temp_paths("commit");
        let mut store = Store::new();
        let mut supervisor =
            EngineSupervisor::new(socket_path.clone(), Some(state_path)).expect("supervisor binds");
        store
            .dispatch("groups/createGroup", json!({"id": "g1", "name": "Effects"}))
            .expect("createGroup dispatches");
        store.drain_deltas();

        let mut stream = connect(&socket_path);
        let (response, events) = request(
            &mut supervisor,
            &mut store,
            &mut stream,
            12,
            "state/commit",
            json!({
                "type": "groups/UPDATE_GROUP_BY_KEY",
                "payload": {"groupId": "g1", "key": "alpha", "value": 0.5}
            }),
        );

        assert!(response.ok);
        assert_eq!(store.state().groups[0].alpha, 0.5);
        let delta = events
            .iter()
            .find(|event| event.event == "state/delta")
            .expect("state/delta is broadcast");
        assert_eq!(delta.data["type"], json!("groups/UPDATE_GROUP_BY_KEY"));
        assert_eq!(delta.data["payload"]["data"], json!(0.5));
    }

    #[test]
    fn engine_restore_loads_the_snapshot_file_and_requests_a_gpu_reset() {
        let (socket_path, state_path) = temp_paths("restore");
        let mut store = Store::new();
        let mut supervisor = EngineSupervisor::new(socket_path.clone(), Some(state_path.clone()))
            .expect("supervisor binds");

        // Persist a state with one group, then replace the live state.
        store
            .dispatch("groups/createGroup", json!({"id": "g1", "name": "Saved"}))
            .expect("createGroup dispatches");
        supervisor.save_snapshot(&store).expect("snapshot saved");
        store
            .restore(&json!({"groups": []}))
            .expect("state cleared");

        let mut stream = connect(&socket_path);
        let (response, _) = request(
            &mut supervisor,
            &mut store,
            &mut stream,
            13,
            "engine/restore",
            Value::Null,
        );
        assert!(response.ok);
        assert_eq!(response.result.unwrap()["restored"], json!(true));
        assert_eq!(store.state().groups.len(), 1);
        assert_eq!(store.state().groups[0].name, "Saved");
        assert!(supervisor.take_reset_request());

        let _ = fs::remove_file(&state_path);
    }

    #[test]
    fn engine_shutdown_flushes_the_snapshot_and_requests_exit() {
        let (socket_path, state_path) = temp_paths("shutdown");
        let mut store = Store::new();
        let mut supervisor = EngineSupervisor::new(socket_path.clone(), Some(state_path.clone()))
            .expect("supervisor binds");

        let mut stream = connect(&socket_path);
        let (response, _) = request(
            &mut supervisor,
            &mut store,
            &mut stream,
            14,
            "engine/shutdown",
            Value::Null,
        );
        assert!(response.ok);
        assert!(supervisor.shutdown_requested());

        let saved: Value = serde_json::from_slice(&fs::read(&state_path).expect("state file"))
            .expect("state parses");
        assert_eq!(saved["version"], json!(1));
        assert_eq!(saved["_engine"]["framesPresented"], json!(0));

        let _ = fs::remove_file(&state_path);
    }

    #[test]
    fn unknown_method_gets_unknown_method_error() {
        let (socket_path, state_path) = temp_paths("unknown-method");
        let mut store = Store::new();
        let mut supervisor =
            EngineSupervisor::new(socket_path.clone(), Some(state_path)).expect("supervisor binds");

        let mut stream = connect(&socket_path);
        let (response, _) = request(
            &mut supervisor,
            &mut store,
            &mut stream,
            9,
            "definitely/not-a-method",
            Value::Null,
        );

        assert!(!response.ok);
        assert_eq!(response.error.unwrap().code, "unknown_method");
    }

    #[test]
    fn preview_cadence_starts_due_and_waits_after_publishing() {
        let (socket_path, state_path) = temp_paths("preview");
        let mut supervisor =
            EngineSupervisor::new(socket_path, Some(state_path)).expect("supervisor binds");

        assert!(supervisor.preview_due());
        let ring = supervisor.preview_ring_mut().expect("ring is created");
        let pixels = vec![0u8; (PREVIEW_WIDTH * PREVIEW_HEIGHT * 4) as usize];
        let frame = ring.write_frame(&pixels).expect("frame writes");
        supervisor.publish_preview(frame);
        assert!(
            !supervisor.preview_due(),
            "publishing starts the cadence window"
        );
    }

    // ————————————————— preview/module (WS-14) —————————————————

    /// Send one request and poll until the supervisor queued it (the socket
    /// is non-blocking, so the frame may need a couple of polls).
    fn queue_request(
        supervisor: &mut EngineSupervisor,
        store: &mut Store,
        stream: &mut UnixStream,
        id: u64,
        method: &str,
        params: Value,
    ) -> PreviewRequest {
        stream
            .write_all(
                &Request {
                    id,
                    method: method.into(),
                    params,
                }
                .into_frame(),
            )
            .expect("write preview request");

        for _ in 0..100 {
            supervisor.poll(store);
            let queued = supervisor.pending_previews();
            if let Some(request) = queued.first().cloned() {
                return request;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("timed out waiting for the preview to be queued");
    }

    /// [`queue_request`] for a `preview/module` request.
    fn queue_preview(
        supervisor: &mut EngineSupervisor,
        store: &mut Store,
        stream: &mut UnixStream,
        id: u64,
        params: Value,
    ) -> PreviewRequest {
        queue_request(supervisor, store, stream, id, "preview/module", params)
    }

    /// Read `expected` KIND_RESPONSE frames without polling the supervisor:
    /// completed previews are already buffered on the socket. Events seen on
    /// the way are skipped.
    fn read_buffered_responses(stream: &mut UnixStream, expected: usize) -> Vec<Response> {
        let mut reader = FrameReader::new();
        let mut responses = Vec::new();
        let mut scratch = [0u8; 8192];
        for _ in 0..200 {
            match stream.read(&mut scratch) {
                Ok(n) if n > 0 => {
                    for frame in reader.push(&scratch[..n]) {
                        let (kind, payload) = frame.expect("frame decodes");
                        if kind == KIND_RESPONSE {
                            responses.push(serde_json::from_slice(&payload).expect("response"));
                        }
                    }
                    if responses.len() >= expected {
                        return responses;
                    }
                }
                Ok(_) => {}
                Err(error)
                    if error.kind() == io::ErrorKind::WouldBlock
                        || error.kind() == io::ErrorKind::TimedOut =>
                {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("read failed: {error}"),
            }
        }
        panic!(
            "timed out waiting for {expected} buffered response frame(s); got {}",
            responses.len()
        );
    }

    #[test]
    fn preview_request_is_deferred_and_answered_with_base64_pixels() {
        let (socket_path, state_path) = temp_paths("preview-module");
        let mut store = Store::new();
        let mut supervisor =
            EngineSupervisor::new(socket_path.clone(), Some(state_path)).expect("supervisor binds");
        let mut stream = connect(&socket_path);

        let request = queue_preview(
            &mut supervisor,
            &mut store,
            &mut stream,
            31,
            json!({ "moduleName": "Point10 Invert", "width": 4.0, "height": 2.0 }),
        );

        assert_eq!(request.id, 31);
        assert_eq!(
            request.kind,
            PreviewKind::Module {
                module_name: "Point10 Invert".to_string()
            }
        );
        assert_eq!((request.width, request.height), (4, 2));

        let pixels: Vec<u8> = (0..(4 * 2 * 4)).map(|value| value as u8).collect();
        supervisor.complete_preview(request, Ok(pixels.clone()));

        let responses = read_buffered_responses(&mut stream, 1);
        let response = &responses[0];
        assert_eq!(response.id, 31);
        assert!(response.ok);
        let result = response.result.as_ref().expect("result present");
        assert_eq!(result["width"], json!(4));
        assert_eq!(result["height"], json!(2));
        assert_eq!(result["pixels"], json!(base64_encode(&pixels)));
    }

    #[test]
    fn preview_module_errors_surface_their_codes() {
        let (socket_path, state_path) = temp_paths("preview-errors");
        let mut store = Store::new();
        let mut supervisor =
            EngineSupervisor::new(socket_path.clone(), Some(state_path)).expect("supervisor binds");
        let mut stream = connect(&socket_path);

        // A payload without a module name is rejected immediately rather than
        // queued for a render that cannot resolve it.
        let (response, _) = request(
            &mut supervisor,
            &mut store,
            &mut stream,
            41,
            "preview/module",
            json!({ "width": 8, "height": 8 }),
        );
        assert!(!response.ok);
        assert_eq!(response.error.unwrap().code, "invalid_params");

        // A queued request for an unknown module is answered with
        // `module_not_found` (the renderer backs off on that code).
        let request = queue_preview(
            &mut supervisor,
            &mut store,
            &mut stream,
            42,
            json!({ "moduleName": "definitely not registered", "width": 8, "height": 8 }),
        );
        supervisor.complete_preview(
            request,
            Err(PreviewError::ModuleNotFound(
                "definitely not registered".to_string(),
            )),
        );

        let response = read_buffered_responses(&mut stream, 1);
        let response = &response[0];
        assert_eq!(response.id, 42);
        assert!(!response.ok);
        assert_eq!(
            response.error.as_ref().expect("error present").code,
            "module_not_found"
        );
    }

    #[test]
    fn requeued_previews_keep_their_order() {
        let (socket_path, state_path) = temp_paths("preview-queue");
        let mut store = Store::new();
        let mut supervisor =
            EngineSupervisor::new(socket_path.clone(), Some(state_path)).expect("supervisor binds");
        let mut stream = connect(&socket_path);

        let first = queue_preview(
            &mut supervisor,
            &mut store,
            &mut stream,
            51,
            json!({ "moduleName": "Ball", "width": 16, "height": 16 }),
        );
        // Put it back as the window loop would after answering a bounded
        // number of previews this frame.
        supervisor.requeue_previews(vec![first.clone()]);
        assert_eq!(supervisor.pending_previews(), vec![first]);
    }

    #[test]
    fn scene_preview_request_is_deferred_and_answered_with_base64_pixels() {
        let (socket_path, state_path) = temp_paths("preview-scene");
        let mut store = Store::new();
        let mut supervisor =
            EngineSupervisor::new(socket_path.clone(), Some(state_path)).expect("supervisor binds");
        let mut stream = connect(&socket_path);

        let request = queue_request(
            &mut supervisor,
            &mut store,
            &mut stream,
            81,
            "preview/scene",
            json!({ "sceneId": "scene-1", "rowIndex": 2, "width": 4, "height": 2 }),
        );

        assert_eq!(request.id, 81);
        assert_eq!((request.width, request.height), (4, 2));
        assert_eq!(
            request.kind,
            PreviewKind::Scene {
                scene_id: "scene-1".to_string(),
                row_index: 2,
            }
        );

        let pixels: Vec<u8> = (0..(4 * 2 * 4)).map(|value| value as u8).collect();
        supervisor.complete_preview(request, Ok(pixels.clone()));

        let responses = read_buffered_responses(&mut stream, 1);
        let response = &responses[0];
        assert_eq!(response.id, 81);
        assert!(response.ok);
        let result = response.result.as_ref().expect("result present");
        assert_eq!(result["width"], json!(4));
        assert_eq!(result["height"], json!(2));
        assert_eq!(result["pixels"], json!(base64_encode(&pixels)));
    }

    #[test]
    fn scene_preview_clamps_sizes_and_validates_params() {
        let (socket_path, state_path) = temp_paths("preview-scene-params");
        let mut store = Store::new();
        let mut supervisor =
            EngineSupervisor::new(socket_path.clone(), Some(state_path)).expect("supervisor binds");
        let mut stream = connect(&socket_path);

        // Missing scene id and a negative row are rejected immediately rather
        // than queued for a lookup that cannot succeed.
        let (response, _) = request(
            &mut supervisor,
            &mut store,
            &mut stream,
            82,
            "preview/scene",
            json!({ "rowIndex": 0, "width": 8, "height": 8 }),
        );
        assert!(!response.ok);
        assert_eq!(response.error.unwrap().code, "invalid_params");

        let (response, _) = request(
            &mut supervisor,
            &mut store,
            &mut stream,
            83,
            "preview/scene",
            json!({ "sceneId": "scene-1", "rowIndex": -1, "width": 8, "height": 8 }),
        );
        assert!(!response.ok);
        assert_eq!(response.error.unwrap().code, "invalid_params");

        // Sizes clamp to the Scene cap (480), not the module thumbnail cap.
        let request = queue_request(
            &mut supervisor,
            &mut store,
            &mut stream,
            84,
            "preview/scene",
            json!({ "sceneId": "scene-1", "rowIndex": 0, "width": 1000, "height": 0 }),
        );
        assert_eq!((request.width, request.height), (480, 1));

        supervisor.complete_preview(
            request,
            Err(PreviewError::SceneRowEmpty {
                scene_id: "scene-1".to_string(),
                row: 0,
            }),
        );
        let responses = read_buffered_responses(&mut stream, 1);
        assert_eq!(
            responses[0].error.as_ref().expect("error present").code,
            "scene_row_empty"
        );

        // An unknown Scene surfaces as `scene_not_found` (the UI retries on
        // its 500 ms backoff rather than treating it as a fatal error).
        let request = queue_request(
            &mut supervisor,
            &mut store,
            &mut stream,
            85,
            "preview/scene",
            json!({ "sceneId": "missing-scene", "rowIndex": 0, "width": 8, "height": 8 }),
        );
        supervisor.complete_preview(
            request,
            Err(PreviewError::SceneNotFound("missing-scene".to_string())),
        );
        let responses = read_buffered_responses(&mut stream, 1);
        assert_eq!(
            responses[0].error.as_ref().expect("error present").code,
            "scene_not_found"
        );
    }

    #[test]
    fn preview_queue_is_bounded() {
        let (socket_path, state_path) = temp_paths("preview-bound");
        let mut store = Store::new();
        let mut supervisor =
            EngineSupervisor::new(socket_path.clone(), Some(state_path)).expect("supervisor binds");
        for id in 0..MAX_PENDING_PREVIEWS {
            supervisor.pending_previews.push_back(PreviewRequest {
                id: id as u64,
                client: 0,
                width: 16,
                height: 16,
                kind: PreviewKind::Module {
                    module_name: "Ball".to_string(),
                },
            });
        }

        let mut stream = connect(&socket_path);
        let (response, _) = request(
            &mut supervisor,
            &mut store,
            &mut stream,
            71,
            "preview/module",
            json!({ "moduleName": "Ball", "width": 16, "height": 16 }),
        );
        assert!(!response.ok);
        assert_eq!(response.error.unwrap().code, "engine_busy");
        assert_eq!(
            supervisor.pending_previews().len(),
            MAX_PENDING_PREVIEWS,
            "the overflow request is not queued"
        );
    }

    #[test]
    fn shutdown_rejects_queued_previews() {
        let (socket_path, state_path) = temp_paths("preview-shutdown");
        let mut store = Store::new();
        let mut supervisor =
            EngineSupervisor::new(socket_path.clone(), Some(state_path)).expect("supervisor binds");
        let mut stream = connect(&socket_path);

        let queued = queue_preview(
            &mut supervisor,
            &mut store,
            &mut stream,
            61,
            json!({ "moduleName": "Ball", "width": 16, "height": 16 }),
        );
        // `queue_preview` drains the queue like the window loop; put it back
        // so the shutdown path has something to reject.
        supervisor.requeue_previews(vec![queued]);

        stream
            .write_all(
                &Request {
                    id: 62,
                    method: "engine/shutdown".into(),
                    params: Value::Null,
                }
                .into_frame(),
            )
            .expect("write shutdown request");
        for _ in 0..100 {
            supervisor.poll(&mut store);
            if supervisor.shutdown_requested() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(supervisor.shutdown_requested());

        // The queued preview was answered with an error before the shutdown
        // acknowledgement.
        let responses = read_buffered_responses(&mut stream, 2);
        assert_eq!(responses[0].id, 61);
        assert!(!responses[0].ok);
        assert_eq!(
            responses[0].error.as_ref().expect("error present").code,
            "engine_shutting_down"
        );
        assert_eq!(responses[1].id, 62);
        assert!(responses[1].ok);
    }
}
