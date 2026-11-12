//! WS-14 / M1 — the engine side of the control channel and the preview ring.
//!
//! One [`EngineSupervisor`] owns:
//! * the non-blocking `SocketServer` bound to `--socket`;
//! * the `VMP1` preview ring in `/dev/shm` (published to clients over the
//!   socket as `preview/ready` + `preview/frame` events);
//! * the state snapshot written to `--state` (crash-safe: tmp file + rename),
//!   reloaded at startup so a supervisor restart restores the last frame
//!   count instead of starting from zero;
//! * the heartbeat the Electron supervisor watchdog keys off
//!   (`engine/heartbeat` every second).
//!
//! M1 scope note: the engine has no store yet (that is WS-1, M2), so
//! `state/commit` / `state/dispatch` are accepted, acknowledged and logged —
//! unknown dispatches return `ok: true, result: null` rather than an error,
//! because the renderer legitimately fires boot-time actions (seeding,
//! hydration) the engine does not implement yet. `engine/shutdown` is the
//! only request with behaviour: persist the snapshot, then signal the event
//! loop to exit.
//!
//! The preview currently publishes the flat clear colour on a CPU-side
//! 320×180 buffer. WS-2/M2 replaces it with the compositor's output frame;
//! the transport (ring + events + seqlock) is already the final shape.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use modv_ipc::frame::{KIND_EVENT, KIND_REQUEST};
use modv_ipc::message::{Event, Incoming, Request, Response, from_frame};
use modv_ipc::preview::PreviewRing;
use modv_ipc::socket::SocketServer;

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(1);
const PREVIEW_INTERVAL: Duration = Duration::from_millis(33); // ≤ 30 fps
const SNAPSHOT_SAVE_EVERY_FRAMES: u64 = 120;
const PREVIEW_WIDTH: u32 = 320;
const PREVIEW_HEIGHT: u32 = 180;
// Matches window.rs's clear colour. The preview publishes this until WS-2
// wires the real compositor output through.
const CLEAR_RGBA: [u8; 4] = [11, 11, 15, 255];

/// On-disk state snapshot (the M1 stand-in for the full engine state).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Snapshot {
    version: u32,
    saved_at_epoch_ms: u64,
    frames_presented: u64,
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
    preview_pixels: Vec<u8>,
    shutdown_requested: bool,
    unknown_dispatches_warned: HashSet<String>,
}

impl EngineSupervisor {
    /// Bind the control socket and load the snapshot, if any.
    pub fn new(socket_path: PathBuf, state_path: Option<PathBuf>) -> io::Result<Self> {
        let server = SocketServer::bind(&socket_path)?;
        let restored_frames = state_path
            .as_ref()
            .and_then(|path| load_snapshot(path))
            .map(|s| s.frames_presented)
            .unwrap_or(0);

        if restored_frames > 0 {
            eprintln!("engine: INFO restored state snapshot (frames_presented={restored_frames})");
        }

        let preview_pixels = CLEAR_RGBA
            .iter()
            .copied()
            .cycle()
            .take((PREVIEW_WIDTH * PREVIEW_HEIGHT) as usize * 4)
            .collect();

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
            preview_pixels,
            shutdown_requested: false,
            unknown_dispatches_warned: HashSet::new(),
        })
    }

    // Read by tests and part of the snapshot-facing API; not used on the hot path yet.
    #[allow(dead_code)]
    pub fn frames_presented(&self) -> u64 {
        self.frames_presented
    }

    pub fn shutdown_requested(&self) -> bool {
        self.shutdown_requested
    }

    /// Record a presented frame; checkpoints the snapshot periodically.
    pub fn note_frame_presented(&mut self) {
        self.frames_presented += 1;
        self.frames_since_save += 1;

        if self.frames_since_save >= SNAPSHOT_SAVE_EVERY_FRAMES {
            if let Err(error) = self.save_snapshot() {
                eprintln!("engine: WARN could not save snapshot: {error}");
            }
        }
    }

    /// Persist the snapshot atomically (tmp file + rename).
    pub fn save_snapshot(&mut self) -> io::Result<()> {
        self.frames_since_save = 0;
        let Some(path) = &self.state_path else {
            return Ok(());
        };

        let snapshot = Snapshot {
            version: 1,
            saved_at_epoch_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
            frames_presented: self.frames_presented,
        };
        let payload = serde_json::to_vec_pretty(&snapshot).map_err(io::Error::other)?;
        let tmp = PathBuf::from(format!("{}.tmp", path.display()));
        fs::write(&tmp, payload)?;
        fs::rename(&tmp, path)
    }

    /// Poll the socket and publish scheduled heartbeat/preview events.
    pub fn poll(&mut self) {
        let poll = match self.server.poll() {
            Ok(poll) => poll,
            Err(error) => {
                eprintln!("engine: WARN control socket poll failed: {error}");
                return;
            }
        };

        // Announce the preview channel to freshly connected clients.
        for client in poll.accepted {
            let response = self.preview_ready_event();
            if let Ok(event) = response {
                let payload = serde_json::to_vec(&event).expect("preview/ready serialises");
                let _ = self.server.send(client, KIND_EVENT, &payload);
            }
        }

        for frame in poll.frames {
            if frame.kind != KIND_REQUEST {
                continue;
            }
            match from_frame(frame.kind, &frame.payload) {
                Ok(Incoming::Request(request)) => {
                    let response = self.handle_request(request);
                    if let Err(error) = self.server.send_response(frame.client, &response) {
                        eprintln!("engine: WARN could not send response: {error}");
                    }
                }
                Ok(_) | Err(_) => {
                    eprintln!("engine: WARN ignoring malformed request frame");
                }
            }
        }

        self.maybe_heartbeat();
        self.maybe_publish_preview();
    }

    fn handle_request(&mut self, request: Request) -> Response {
        match request.method.as_str() {
            "engine/shutdown" => {
                eprintln!("engine: INFO shutdown requested; flushing state");
                if let Err(error) = self.save_snapshot() {
                    eprintln!("engine: WARN shutdown snapshot failed: {error}");
                }
                self.shutdown_requested = true;
                Response::ok(request.id, json!({ "saved": true }))
            }

            "state/snapshot" => Response::ok(
                request.id,
                json!({
                    "version": 1,
                    "framesPresented": self.frames_presented,
                    "restoredFrames": self.restored_frames,
                    "uptimeSeconds": self.started.elapsed().as_secs_f64(),
                }),
            ),

            "state/commit" => {
                // M1: no store yet. Log the intent; the payload is ignored.
                if let Some(commit_type) = request.params.get("type").and_then(Value::as_str) {
                    eprintln!(
                        "engine: INFO commit {commit_type} (accepted; engine has no store yet)"
                    );
                }
                Response::ok(request.id, Value::Null)
            }

            "state/dispatch" => {
                let dispatch_type = request
                    .params
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("<missing>")
                    .to_string();
                match dispatch_type.as_str() {
                    // M1: the engine has no store yet, but the caller's
                    // result contract is "the created Group" — return a
                    // plausible synthetic one until WS-1 lands.
                    "groups/createGroup" => {
                        let name = request
                            .params
                            .get("payload")
                            .and_then(|p| p.get("name"))
                            .and_then(Value::as_str)
                            .unwrap_or("New Group")
                            .to_string();
                        eprintln!(
                            "engine: INFO dispatch groups/createGroup (synthetic result for M1)"
                        );
                        return Response::ok(
                            request.id,
                            json!({
                                "id": uuid_v4(),
                                "name": name,
                                "modules": [],
                                "enabled": 1,
                                "alpha": 1,
                                "hidden": false,
                            }),
                        );
                    }
                    "scenes/hydrateScenes"
                    | "scenes/ensureDefaultScenes"
                    | "osc/createServer"
                    | "metrics/SET_FPS_MEASURE" => {
                        eprintln!("engine: INFO dispatch {dispatch_type} (accepted; no-op in M1)");
                    }
                    other => {
                        if self.unknown_dispatches_warned.insert(other.to_string()) {
                            eprintln!("engine: WARN dispatch {other} not implemented yet (M2+)");
                        }
                    }
                }
                Response::ok(request.id, Value::Null)
            }

            "engine/restore" => {
                let restored = self
                    .state_path
                    .as_ref()
                    .and_then(|path| load_snapshot(path))
                    .map(|s| {
                        self.restored_frames = s.frames_presented;
                        self.frames_presented = s.frames_presented;
                        true
                    })
                    .unwrap_or(false);
                Response::ok(request.id, json!({ "restored": restored }))
            }

            other => Response::err(
                request.id,
                "unknown_method",
                format!("no such method: {other}"),
            ),
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

    fn maybe_publish_preview(&mut self) {
        let now = Instant::now();
        if let Some(last) = self.last_preview {
            if now.duration_since(last) < PREVIEW_INTERVAL {
                return;
            }
        }

        if self.ensure_preview().is_none() {
            return;
        }

        // Move the pixel buffer out to sidestep the borrow conflict with the
        // ring; it is moved back in before returning.
        let pixels = std::mem::take(&mut self.preview_pixels);
        let frame = match self.preview.as_mut() {
            Some(ring) => match ring.write_frame(&pixels) {
                Ok(frame) => Some(frame),
                Err(error) => {
                    eprintln!("engine: WARN preview write failed: {error}");
                    None
                }
            },
            None => None,
        };
        self.preview_pixels = pixels;

        let Some(frame) = frame else {
            return;
        };
        self.last_preview = Some(now);

        let event = Event::new("preview/frame", frame.to_event());
        if let Err(error) = self.server.broadcast_event(&event) {
            eprintln!("engine: WARN preview broadcast failed: {error}");
        }
    }
}

fn load_snapshot(path: &Path) -> Option<Snapshot> {
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Minimal UUID v4 from /dev/urandom (the M1 engine has no uuid dependency).
fn uuid_v4() -> String {
    let mut bytes = [0u8; 16];
    match fs::File::open("/dev/urandom").and_then(|mut f| {
        use std::io::Read;
        f.read_exact(&mut bytes)
    }) {
        Ok(()) => {
            bytes[6] = (bytes[6] & 0x0f) | 0x40;
            bytes[8] = (bytes[8] & 0x3f) | 0x80;
            format!(
                "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
                bytes[0],
                bytes[1],
                bytes[2],
                bytes[3],
                bytes[4],
                bytes[5],
                bytes[6],
                bytes[7],
                bytes[8],
                bytes[9],
                bytes[10],
                bytes[11],
                bytes[12],
                bytes[13],
                bytes[14],
                bytes[15]
            )
        }
        Err(_) => {
            // Fallback: time + pid entropy, hex-only. Fine for M1.
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            format!(
                "{:08x}-0000-4000-8000-{:012x}",
                std::process::id(),
                nanos & 0xffffffffffff
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;

    use modv_ipc::frame::{FrameReader, KIND_RESPONSE};

    /// Poll the supervisor until a KIND_RESPONSE frame arrives from the
    /// stream; returns its payload bytes.
    fn read_response(supervisor: &mut EngineSupervisor, stream: &mut UnixStream) -> Vec<u8> {
        let mut reader = FrameReader::new();
        for _ in 0..100 {
            supervisor.poll();
            let mut scratch = [0u8; 8192];
            match stream.read(&mut scratch) {
                Ok(n) if n > 0 => {
                    for frame in reader.push(&scratch[..n]) {
                        let (kind, payload) = frame.expect("frame decodes");
                        if kind == KIND_RESPONSE {
                            return payload;
                        }
                        // preview/ready and heartbeat events are expected too;
                        // skip them.
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

    #[test]
    fn snapshot_roundtrip_restores_frame_count() {
        let (socket_path, state_path) = temp_paths("roundtrip");

        {
            let mut supervisor =
                EngineSupervisor::new(socket_path.clone(), Some(state_path.clone()))
                    .expect("supervisor binds");
            for _ in 0..7 {
                supervisor.note_frame_presented();
            }
            supervisor.save_snapshot().expect("snapshot saved");
            assert_eq!(supervisor.frames_presented(), 7);
        }

        let restored = EngineSupervisor::new(socket_path, Some(state_path.clone()))
            .expect("supervisor rebinds");
        assert_eq!(restored.frames_presented(), 7);

        let _ = fs::remove_file(&state_path);
    }

    #[test]
    fn snapshot_request_roundtrip() {
        let (socket_path, state_path) = temp_paths("roundtrip-ipc");
        let mut supervisor =
            EngineSupervisor::new(socket_path.clone(), Some(state_path)).expect("supervisor binds");
        for _ in 0..3 {
            supervisor.note_frame_presented();
        }

        let mut stream = UnixStream::connect(&socket_path).expect("connect");
        stream
            .set_read_timeout(Some(Duration::from_millis(500)))
            .unwrap();
        let request = Request {
            id: 42,
            method: "state/snapshot".into(),
            params: Value::Null,
        };
        stream
            .write_all(&request.into_frame())
            .expect("write request");

        let payload = read_response(&mut supervisor, &mut stream);
        let response: Response = serde_json::from_slice(&payload).expect("response parses");
        assert_eq!(response.id, 42);
        assert!(response.ok);
        assert_eq!(response.result.unwrap()["framesPresented"], 3);
    }

    #[test]
    fn create_group_dispatch_returns_synthetic_group() {
        let (socket_path, state_path) = temp_paths("create-group");
        let mut supervisor =
            EngineSupervisor::new(socket_path.clone(), Some(state_path)).expect("supervisor binds");

        let mut stream = UnixStream::connect(&socket_path).expect("connect");
        stream
            .set_read_timeout(Some(Duration::from_millis(500)))
            .unwrap();
        let request = Request {
            id: 11,
            method: "state/dispatch".into(),
            params: json!({ "type": "groups/createGroup", "payload": { "name": "Gallery" } }),
        };
        stream
            .write_all(&request.into_frame())
            .expect("write request");

        let payload = read_response(&mut supervisor, &mut stream);
        let response: Response = serde_json::from_slice(&payload).expect("response parses");
        assert!(response.ok);
        let result = response.result.expect("result present");
        assert_eq!(result["name"], "Gallery");
        assert!(result["id"].as_str().map(|s| s.len()).unwrap_or(0) >= 32);
        assert_eq!(result["modules"], json!([]));
    }

    #[test]
    fn unknown_method_gets_unknown_method_error() {
        let (socket_path, state_path) = temp_paths("unknown-method");
        let mut supervisor =
            EngineSupervisor::new(socket_path.clone(), Some(state_path)).expect("supervisor binds");

        let mut stream = UnixStream::connect(&socket_path).expect("connect");
        stream
            .set_read_timeout(Some(Duration::from_millis(500)))
            .unwrap();
        let request = Request {
            id: 9,
            method: "definitely/not-a-method".into(),
            params: Value::Null,
        };
        stream
            .write_all(&request.into_frame())
            .expect("write request");

        let payload = read_response(&mut supervisor, &mut stream);
        let response: Response = serde_json::from_slice(&payload).expect("response parses");
        assert!(!response.ok);
        assert_eq!(response.error.unwrap().code, "unknown_method");
    }
}
