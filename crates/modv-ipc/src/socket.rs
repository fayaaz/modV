//! Non-blocking Unix domain socket plumbing for the control channel.
//!
//! spec: docs/rust-hybrid.md — “IPC and frame transport” + “Protocol
//! specification — control channel”.
//!
//! Engine integration pattern: the engine main loop calls [`SocketServer::poll`]
//! once per frame (or per iteration of its event loop), dispatches the returned
//! [`Inframe`]s to the method handlers, writes responses with
//! [`SocketServer::send_response`] and broadcasts events with
//! [`SocketServer::broadcast_event`]. Preview pixel data does **not** travel
//! over this socket — it travels over the shared-memory ring
//! ([`crate::preview`]); this socket carries the `preview/ready` handshake and
//! the `preview/frame` notifications as ordinary [`crate::message::Event`]s.
//!
//! Ambiguity resolved: the spec fixes the API shape but not exact semantics
//! for partial writes. `send` appends to a per-client output buffer and then
//! tries to flush immediately; on `WouldBlock` the remainder stays buffered and
//! is flushed by the next [`SocketServer::poll`]. A client whose buffer would
//! exceed [`MAX_PENDING_OUT_BYTES`] (16 MiB) — i.e. a peer that never reads —
//! is marked dead instead of growing memory without bound (spec: “never queue
//! unbounded frames”). A dead client is skipped by sends and dropped, with its
//! id reported in [`PollResult::disconnected`], at the next `poll`.
//!
//! spec note: kind 3 (`KIND_BINARY`) frames pass through this layer untouched;
//! interpreting kinds happens in [`crate::message::from_frame`].

use std::cell::{Cell, RefCell};
use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

use crate::frame::{self, FrameReader, KIND_EVENT, KIND_RESPONSE};
use crate::message::{Event, Response};

/// Default maximum number of simultaneous clients. spec: default 8.
pub const DEFAULT_MAX_CLIENTS: usize = 8;

/// Maximum bytes buffered per client for writes that would block. A client
/// that exceeds this is disconnected on the spot. spec: bounded queues only.
pub const MAX_PENDING_OUT_BYTES: usize = 16 * 1024 * 1024;

const READ_SCRATCH_BYTES: usize = 64 * 1024;

/// One complete frame read from a client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inframe {
    /// Stable id of the client that sent the frame.
    pub client: usize,
    /// Frame kind byte (0 = request, 1 = response, 2 = event, 3 = binary reserved).
    pub kind: u8,
    /// Frame payload (e.g. the JSON bytes of a `Request`).
    pub payload: Vec<u8>,
}

/// Outcome of one [`SocketServer::poll`].
#[derive(Debug, Default)]
pub struct PollResult {
    /// All complete frames received since the last poll, in read order.
    pub frames: Vec<Inframe>,
    /// Ids assigned to clients accepted since the last poll.
    pub accepted: Vec<usize>,
    /// Ids of clients that were dropped during this poll (EOF, hard I/O error,
    /// or a framing error on their stream). Disconnected ids are stable and
    /// are never reused.
    pub disconnected: Vec<usize>,
}

struct SocketClientConn {
    id: usize,
    stream: UnixStream,
    reader: FrameReader,
    /// Bytes accepted by the peer but not yet written (partial-write carry-over).
    out: RefCell<Vec<u8>>,
    /// Set when a hard error occurred; skipped by sends, reaped by the next poll.
    dead: Cell<bool>,
}

impl SocketClientConn {
    /// Write as much of the pending buffer as the kernel accepts right now.
    ///
    /// `WouldBlock` keeps the remainder buffered and reports success — the
    /// peer is simply slow, and `poll` flushes again next time. Any other
    /// error clears the buffer, marks nothing itself (the caller marks the
    /// client dead) and propagates.
    fn flush_out(&self) -> io::Result<()> {
        let mut out = self.out.borrow_mut();
        while !out.is_empty() {
            match (&self.stream).write(&out) {
                Ok(0) => return Err(write_failed()),
                Ok(n) => {
                    out.drain(..n);
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => {
                    out.clear();
                    return Err(e);
                }
            }
        }
        Ok(())
    }
}

fn write_failed() -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, "write to socket client failed")
}

/// Non-blocking Unix domain socket server.
///
/// Binds the socket, accepts and de-frames clients, and writes/broadcasts
/// frames. Dropping the server unlinks the socket file it created (and only
/// that one).
pub struct SocketServer {
    listener: UnixListener,
    clients: Vec<SocketClientConn>,
    max_clients: usize,
    next_client_id: usize,
    path: PathBuf,
    /// Device/inode of the socket node this server bound, used by `Drop` to
    /// avoid unlinking a socket another engine has since created at the path.
    identity: Option<(u64, u64)>,
}

/// `(device, inode)` of the socket node at `path`, or `None` when the path is
/// missing, a symlink or not a socket node.
fn socket_identity(path: &Path) -> Option<(u64, u64)> {
    let metadata = fs::symlink_metadata(path).ok()?;

    if !metadata.file_type().is_socket() {
        return None;
    }

    Some((metadata.dev(), metadata.ino()))
}

/// Whether `path` is a socket node nobody is listening on.
///
/// A live socket accepts the probe connection; a regular file (or a symlink)
/// refuses with `ECONNREFUSED` but is not a socket node; anything else (a
/// permission error, a full listen backlog, …) is treated as "not ours to
/// replace".
fn is_stale_socket(path: &Path) -> bool {
    match UnixStream::connect(path) {
        Ok(_) => false,
        Err(error) => {
            error.kind() == io::ErrorKind::ConnectionRefused && socket_identity(path).is_some()
        }
    }
}

impl SocketServer {
    /// Bind `path`, replacing a stale socket file first (spec: "removes a stale
    /// socket file"). The listener is set non-blocking and the node is
    /// restricted to its owner (`0600`).
    ///
    /// Only a socket node that nobody is listening on is replaced. A live
    /// socket — another engine already owns the path — is never deleted: `bind`
    /// fails with [`io::ErrorKind::AddrInUse`] instead, as does a regular file
    /// or symlink at the path. The engine supervisor treats `AddrInUse` as "a
    /// live control socket already exists".
    pub fn bind(path: &Path) -> io::Result<Self> {
        match fs::symlink_metadata(path) {
            Ok(_) => {
                if !is_stale_socket(path) {
                    return Err(io::Error::new(
                        io::ErrorKind::AddrInUse,
                        format!(
                            "{} exists and is not a stale control socket",
                            path.display()
                        ),
                    ));
                }
                fs::remove_file(path)?;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }

        let listener = UnixListener::bind(path)?;
        listener.set_nonblocking(true)?;
        // The socket carries `engine/shutdown` and full control of the engine;
        // do not depend on the caller's umask.
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        let identity = socket_identity(path);

        Ok(Self {
            listener,
            clients: Vec::new(),
            max_clients: DEFAULT_MAX_CLIENTS,
            next_client_id: 0,
            path: path.to_path_buf(),
            identity,
        })
    }

    /// Override the maximum number of simultaneous clients (default 8).
    pub fn with_max_clients(mut self, max_clients: usize) -> Self {
        self.max_clients = max_clients;
        self
    }

    /// Accept pending connections, flush buffered writes, read available
    /// bytes from every client and collect complete frames.
    ///
    /// Clients that hit EOF, a hard I/O error, or a framing error on their
    /// stream are dropped and their ids appear in [`PollResult::disconnected`].
    pub fn poll(&mut self) -> io::Result<PollResult> {
        let mut result = PollResult::default();

        // 1. Accept pending connections; reject excess clients by dropping them.
        loop {
            match self.listener.accept() {
                Ok((stream, _addr)) => {
                    stream.set_nonblocking(true)?;
                    let alive = self.clients.iter().filter(|c| !c.dead.get()).count();
                    if alive >= self.max_clients {
                        // spec: bounded clients; excess peers are dropped immediately.
                        continue;
                    }
                    let id = self.next_client_id;
                    self.next_client_id += 1;
                    self.clients.push(SocketClientConn {
                        id,
                        stream,
                        reader: FrameReader::new(),
                        out: RefCell::new(Vec::new()),
                        dead: Cell::new(false),
                    });
                    result.accepted.push(id);
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }

        // 2. Flush carry-over writes for live clients.
        for client in self.clients.iter_mut().filter(|c| !c.dead.get()) {
            if client.out.borrow().is_empty() {
                continue;
            }
            if client.flush_out().is_err() {
                client.dead.set(true);
            }
        }

        // 3. Read available bytes from live clients.
        let mut scratch = [0u8; READ_SCRATCH_BYTES];
        for client in self.clients.iter_mut().filter(|c| !c.dead.get()) {
            loop {
                match (&client.stream).read(&mut scratch) {
                    Ok(0) => {
                        // EOF: the peer closed the connection.
                        client.dead.set(true);
                        break;
                    }
                    Ok(n) => {
                        for frame in client.reader.push(&scratch[..n]) {
                            match frame {
                                Ok((kind, payload)) => result.frames.push(Inframe {
                                    client: client.id,
                                    kind,
                                    payload,
                                }),
                                Err(_frame_error) => {
                                    // Framer cannot resynchronise; drop the client.
                                    client.dead.set(true);
                                    break;
                                }
                            }
                        }
                        if client.dead.get() {
                            break;
                        }
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => {
                        client.dead.set(true);
                        break;
                    }
                }
            }
        }

        // 4. Reap dead clients and report their ids.
        let mut disconnected = Vec::new();
        self.clients.retain(|client| {
            if client.dead.get() {
                disconnected.push(client.id);
                false
            } else {
                true
            }
        });
        result.disconnected = disconnected;

        Ok(result)
    }

    /// Send one frame to a specific client.
    ///
    /// Partial writes are buffered internally and flushed on the next
    /// [`SocketServer::poll`]; a client that died is reported with an I/O
    /// error and reaped at the next poll. Unknown or dead client ids fail
    /// with `ErrorKind::NotFound` / `ErrorKind::BrokenPipe`.
    pub fn send(&self, client: usize, kind: u8, payload: &[u8]) -> io::Result<()> {
        let frame = frame::encode_frame(kind, payload)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;

        let conn = self
            .clients
            .iter()
            .find(|c| c.id == client)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("unknown client id {client}"),
                )
            })?;
        if conn.dead.get() {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                format!("client {client} is disconnected"),
            ));
        }

        {
            let mut out = conn.out.borrow_mut();
            if out.len() + frame.len() > MAX_PENDING_OUT_BYTES {
                conn.dead.set(true);
                return Err(io::Error::other(format!(
                    "client {client} exceeded {MAX_PENDING_OUT_BYTES} pending output bytes"
                )));
            }
            out.extend_from_slice(&frame);
        }
        // Best effort immediate flush; leftover bytes are flushed by the next poll.
        match conn.flush_out() {
            Ok(()) => Ok(()),
            Err(e) => {
                conn.dead.set(true);
                Err(e)
            }
        }
    }

    /// Convenience: serialize `response` and send it as a kind 1 frame.
    pub fn send_response(&self, client: usize, response: &Response) -> io::Result<()> {
        let payload = serde_json::to_vec(response)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        self.send(client, KIND_RESPONSE, &payload)
    }

    /// Broadcast an event to every live client.
    ///
    /// Best effort: every live client is written to even if some fail; the
    /// first failure is returned (failed clients are marked dead and dropped at
    /// the next poll).
    pub fn broadcast_event(&self, event: &Event) -> io::Result<()> {
        let payload = serde_json::to_vec(event)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        let mut first_error = None;
        for client in self.clients.iter().filter(|c| !c.dead.get()) {
            if let Err(e) = self.send(client.id, KIND_EVENT, &payload) {
                first_error.get_or_insert(e);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    /// Number of live (not-yet-reaped) clients.
    pub fn client_count(&self) -> usize {
        self.clients.iter().filter(|c| !c.dead.get()).count()
    }

    /// The socket path — for error reporting and the engine's `--socket` argument.
    pub fn paths(&self) -> &Path {
        &self.path
    }
}

impl Drop for SocketServer {
    fn drop(&mut self) {
        // Best effort: do not leave a stale socket file behind — but only
        // unlink the node this server bound. Another engine may have replaced
        // the path after ours was removed, and deleting its live socket would
        // make it unreachable without stopping it.
        if self.identity.is_some() && socket_identity(&self.path) == self.identity {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{Request, from_frame};
    use serde_json::json;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    fn temp_socket_path(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!(
            "modv-ipc-test-{tag}-{}-{nanos}.sock",
            std::process::id()
        ))
    }

    fn read_frame(stream: &mut UnixStream) -> (u8, Vec<u8>) {
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("set read timeout");
        let mut header = [0u8; 5];
        stream.read_exact(&mut header).expect("read header");
        let len = u32::from_le_bytes(header[..4].try_into().expect("slice")) as usize;
        let mut payload = vec![0u8; len];
        stream.read_exact(&mut payload).expect("read payload");
        (header[4], payload)
    }

    #[test]
    fn bind_replaces_a_stale_socket_file() {
        let path = temp_socket_path("stale");
        // `UnixListener::bind` leaves the socket node behind when the listener
        // is dropped, which is exactly the stale-file case.
        let stale = UnixListener::bind(&path).expect("create stale socket node");
        drop(stale);
        assert!(path.exists());

        let mut server = SocketServer::bind(&path).expect("bind over a stale socket");
        assert_eq!(server.paths(), path.as_path());
        assert_eq!(server.client_count(), 0);

        let _client = UnixStream::connect(&path).expect("connect to the new server");
        assert_eq!(server.poll().expect("poll").accepted, vec![0]);
    }

    #[test]
    fn bind_refuses_to_replace_a_live_socket() {
        let path = temp_socket_path("live");
        let listener = UnixListener::bind(&path).expect("bind live listener");

        let result = SocketServer::bind(&path);
        let err = match result {
            Ok(_) => panic!("must not steal a live socket"),
            Err(error) => error,
        };
        assert_eq!(err.kind(), io::ErrorKind::AddrInUse);

        // The path and the live listener survive: the probe connection that
        // `bind` made is queued for it.
        assert!(path.exists());
        listener
            .set_nonblocking(true)
            .expect("non-blocking listener");
        listener
            .accept()
            .expect("probe connection is queued on the live listener");

        drop(listener);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn bind_refuses_to_remove_a_regular_file() {
        let path = temp_socket_path("regular");
        fs::write(&path, b"not a socket").expect("seed stale file");

        let result = SocketServer::bind(&path);
        let err = match result {
            Ok(_) => panic!("must not delete a plain file"),
            Err(error) => error,
        };
        assert_eq!(err.kind(), io::ErrorKind::AddrInUse);
        assert_eq!(fs::read(&path).expect("the file survives"), b"not a socket");

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn bind_creates_a_private_socket() {
        let path = temp_socket_path("mode");
        let server = SocketServer::bind(&path).expect("bind");
        let mode = fs::metadata(&path)
            .expect("socket metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(
            mode, 0o600,
            "the control socket must not be world-accessible"
        );
        drop(server);
    }

    #[test]
    fn drop_keeps_a_socket_replaced_by_another_engine() {
        let path = temp_socket_path("replaced");
        let server = SocketServer::bind(&path).expect("bind");

        // A second engine takes the path over while ours is still alive.
        fs::remove_file(&path).expect("unlink our socket");
        let replacement = UnixListener::bind(&path).expect("bind replacement");

        drop(server);
        assert!(
            path.exists(),
            "drop must not unlink another process's socket"
        );

        drop(replacement);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn request_roundtrip_over_socket() {
        let path = temp_socket_path("roundtrip");
        let mut server = SocketServer::bind(&path).expect("bind");

        let request = Request {
            id: 42,
            method: "scenes/applyScene".to_string(),
            params: json!({ "sceneId": "scene-1" }),
        };
        let mut client = UnixStream::connect(&path).expect("connect");
        client
            .write_all(&request.clone().into_frame())
            .expect("write request");

        let result = server.poll().expect("poll");
        assert_eq!(result.frames.len(), 1);
        assert!(result.disconnected.is_empty());
        let Inframe {
            client: id,
            kind,
            payload,
        } = &result.frames[0];
        assert_eq!(*kind, crate::frame::KIND_REQUEST);
        match from_frame(*kind, payload).expect("decode") {
            crate::message::Incoming::Request(decoded) => {
                assert_eq!(decoded.id, 42);
                assert_eq!(decoded.method, "scenes/applyScene");
                assert_eq!(decoded.params, json!({ "sceneId": "scene-1" }));
            }
            other => panic!("expected request, got {other:?}"),
        }
        assert_eq!(*id, 0, "first client gets id 0");
        assert_eq!(server.client_count(), 1);
        // Keep the connection alive; server cleans up on drop.
        drop(client);
    }

    #[test]
    fn client_disconnect_is_reported() {
        let path = temp_socket_path("disconnect");
        let mut server = SocketServer::bind(&path).expect("bind");

        let client = UnixStream::connect(&path).expect("connect");
        server.poll().expect("poll to accept");
        assert_eq!(server.client_count(), 1);

        drop(client); // close the peer
        let result = server.poll().expect("poll after disconnect");
        assert_eq!(result.disconnected, vec![0]);
        assert_eq!(server.client_count(), 0);
    }

    #[test]
    fn send_response_roundtrip() {
        let path = temp_socket_path("response");
        let mut server = SocketServer::bind(&path).expect("bind");

        let mut client = UnixStream::connect(&path).expect("connect");
        server.poll().expect("poll to accept");

        let response = Response::ok(42, json!({ "applied": true }));
        server.send_response(0, &response).expect("send response");

        let (kind, payload) = read_frame(&mut client);
        assert_eq!(kind, crate::frame::KIND_RESPONSE);
        match from_frame(kind, &payload).expect("decode") {
            crate::message::Incoming::Response(decoded) => {
                assert_eq!(decoded.id, 42);
                assert!(decoded.ok);
                assert_eq!(decoded.result, Some(json!({ "applied": true })));
            }
            other => panic!("expected response, got {other:?}"),
        }
    }

    #[test]
    fn broadcast_event_reaches_all_clients() {
        let path = temp_socket_path("broadcast");
        let mut server = SocketServer::bind(&path).expect("bind");

        let mut client_a = UnixStream::connect(&path).expect("connect a");
        let mut client_b = UnixStream::connect(&path).expect("connect b");
        server.poll().expect("poll to accept both");
        assert_eq!(server.client_count(), 2);

        let event = Event::new(
            "state/delta",
            json!({ "type": "groups/UPDATE_GROUP", "payload": { "id": "g1" } }),
        );
        server.broadcast_event(&event).expect("broadcast");

        for client in [&mut client_a, &mut client_b] {
            let (kind, payload) = read_frame(client);
            assert_eq!(kind, crate::frame::KIND_EVENT);
            match from_frame(kind, &payload).expect("decode") {
                crate::message::Incoming::Event(decoded) => {
                    assert_eq!(decoded.event, "state/delta");
                    assert_eq!(decoded.data["type"], "groups/UPDATE_GROUP");
                }
                other => panic!("expected event, got {other:?}"),
            }
        }
    }

    #[test]
    fn send_to_unknown_client_fails() {
        let path = temp_socket_path("unknown");
        let server = SocketServer::bind(&path).expect("bind");
        let err = server
            .send(1234, crate::frame::KIND_EVENT, b"{}")
            .expect_err("unknown client must fail");
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }
}
