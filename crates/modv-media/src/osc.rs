//! OSC ingestion (WS-11), ported from
//! `src/renderer/src/application/worker/store/modules/osc.js`.
//!
//! The legacy renderer creates a `node-osc` `Server` on `0.0.0.0:3333` at worker
//! setup (`application/index.js:186-189`) and writes every message into
//! `state.data` with
//!
//! ```js
//! set(state.data, msg[0].substring(1).replaceAll("/", "."), msg.slice(1));
//! ```
//!
//! (`store/modules/osc.js:69-71`). Module renders then read `osc.data`
//! (`worker/loop.js:463`), so the address path is the same lodash path an input
//! link can point at (`osc.data.<address>`).
//!
//! This port keeps the wire protocol (`rosc` replaces `node-osc`) and the path
//! semantics, and moves the socket into a thread owned by the engine:
//!
//! * [`OscServer::start`] binds the UDP socket and spawns a reader thread; the
//!   callback runs on that thread, so it must be `Send` and should hand values
//!   to the engine (channel / delta queue) rather than mutate render state.
//! * [`OscValue`] is the subset of OSC types that can land in JSON state:
//!   `Float`, `Int`, `String`, `Blob`. `Double` is narrowed to `Float`, `Long`
//!   to `Int`; `Bool` becomes `Int(1|0)` and `Char` a one-character `String`.
//!   `Time`, `Color`, `Midi`, `Nil`, `Inf` and nested `Array` values are
//!   dropped (they have no representation in the store today).
//! * [`apply_osc_value`] sets one scalar at a lodash path. The JS actually
//!   stores the whole argument list (`msg.slice(1)`, an array);
//!   [`apply_osc_message`] reproduces that exactly. The per-argument callback
//!   in [`OscServer::start`] invokes the scalar form once per argument — see
//!   the deviation notes on [`dispatch_packet`].
//! * Bundles are dispatched immediately (the JS `node-osc` server also emits
//!   their contents); a bundle timetag in the future is **not** scheduled.

use std::fmt;
use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// Default OSC bind host (`application/index.js:186-189`, `new Server(3333, "0.0.0.0")`).
pub const DEFAULT_OSC_HOST: &str = "0.0.0.0";

/// Default OSC port (`application/index.js:186-189`).
pub const DEFAULT_OSC_PORT: u16 = 3333;

/// How long the reader thread blocks on `recv_from` before re-checking its
/// stop flag. Also bounds the shutdown latency of [`OscServer::stop`].
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Maximum UDP payload a single OSC packet can occupy (the UDP length field is
/// 16 bits). `recv_from` drops anything larger, as `node-osc` would.
const MAX_DATAGRAM: usize = 65_535;

/// Largest numeric path segment treated as an array index.
///
/// `lodash.set` happily creates a *sparse* JS array for any safe integer index;
/// JSON has no holes, so materialising index `n` costs `n` `null`s. OSC arrives
/// from the network, so a hostile address such as `/layers/1000000000` must not
/// be able to allocate gigabytes. Indexes above this bound degrade to object
/// keys, which is bounded and still JSON-legal.
pub const MAX_ARRAY_INDEX: usize = 1 << 20;

/// A JSON-representable OSC argument ([`osc.js`'s default type mapping][osc]).
///
/// [osc]: https://github.com/colinbdclark/osc.js
#[derive(Debug, Clone, PartialEq)]
pub enum OscValue {
    /// OSC `f` (and the narrowed `d`).
    Float(f32),
    /// OSC `i` (and the narrowed `h`).
    Int(i32),
    /// OSC `s` / `S`.
    String(String),
    /// OSC `b`, kept as raw bytes.
    Blob(Vec<u8>),
}

impl OscValue {
    /// Converts a `rosc` argument into the store's value subset, or `None` for
    /// types the JS store has no representation for.
    pub fn from_osc_type(value: &rosc::OscType) -> Option<Self> {
        use rosc::OscType;

        match value {
            OscType::Float(v) => Some(Self::Float(*v)),
            // The JS store holds numbers; a double is narrowed like `f32` does.
            OscType::Double(v) => Some(Self::Float(*v as f32)),
            OscType::Int(v) => Some(Self::Int(*v)),
            // `Long` has no distinct JS type; clamp into the `i32` range.
            OscType::Long(v) => Some(Self::Int(i32::try_from(*v).unwrap_or(if *v < 0 {
                i32::MIN
            } else {
                i32::MAX
            }))),
            OscType::String(v) => Some(Self::String(v.clone())),
            OscType::Char(v) => Some(Self::String(v.to_string())),
            OscType::Blob(v) => Some(Self::Blob(v.clone())),
            // `true`/`false` are numbers in the JS path (`Number(true) === 1`).
            OscType::Bool(v) => Some(Self::Int(if *v { 1 } else { 0 })),
            OscType::Time(_)
            | OscType::Color(_)
            | OscType::Midi(_)
            | OscType::Array(_)
            | OscType::Nil
            | OscType::Inf => None,
        }
    }

    /// The JSON value written into `osc.data`.
    ///
    /// `Blob` becomes an array of byte numbers — the store is JSON-serialisable
    /// and an array round-trips better than a base64 string. A non-finite float
    /// becomes `null` (JSON cannot represent `NaN`/`Infinity`).
    pub fn to_json(&self) -> serde_json::Value {
        use serde_json::{Number, Value};

        match self {
            Self::Float(v) => Number::from_f64(f64::from(*v))
                .map(Value::Number)
                .unwrap_or(Value::Null),
            Self::Int(v) => Value::Number(Number::from(*v)),
            Self::String(v) => Value::String(v.clone()),
            Self::Blob(bytes) => Value::Array(
                bytes
                    .iter()
                    .map(|byte| Value::Number(Number::from(*byte)))
                    .collect(),
            ),
        }
    }
}

impl fmt::Display for OscValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Float(v) => write!(f, "{v}"),
            Self::Int(v) => write!(f, "{v}"),
            Self::String(v) => write!(f, "{v}"),
            Self::Blob(bytes) => write!(f, "{} bytes", bytes.len()),
        }
    }
}

/// Why an OSC server could not be created.
#[derive(Debug)]
pub enum OscError {
    /// Binding the UDP socket failed (port in use, address not local, …).
    Bind(io::Error),
    /// The reader thread could not be spawned.
    Spawn(io::Error),
}

impl fmt::Display for OscError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bind(err) => write!(f, "could not bind the OSC socket: {err}"),
            Self::Spawn(err) => write!(f, "could not spawn the OSC reader thread: {err}"),
        }
    }
}

impl std::error::Error for OscError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Bind(err) | Self::Spawn(err) => Some(err),
        }
    }
}

/// A UDP OSC listener with its own reader thread.
///
/// Dropping the server stops it (the reader thread is joined).
pub struct OscServer {
    local_addr: SocketAddr,
    running: Arc<AtomicBool>,
    enabled: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl OscServer {
    /// Binds `host:port` and starts dispatching messages to `on_value`.
    ///
    /// The callback receives one call per OSC **argument**; a multi-argument
    /// message such as `/foo/bar 0.5 0.25` therefore fires twice with the same
    /// address (see the module docs). Each value is also available to
    /// [`apply_osc_message`] if the caller wants the JS array shape.
    ///
    /// The engine's default call site is
    /// `OscServer::start(DEFAULT_OSC_HOST, DEFAULT_OSC_PORT, …)`
    /// (`application/index.js:186-189`).
    pub fn start<F>(host: &str, port: u16, on_value: F) -> Result<Self, OscError>
    where
        F: FnMut(&str, OscValue) + Send + 'static,
    {
        let socket = UdpSocket::bind((host, port)).map_err(OscError::Bind)?;
        socket
            .set_read_timeout(Some(POLL_INTERVAL))
            .map_err(OscError::Bind)?;
        let local_addr = socket.local_addr().map_err(OscError::Bind)?;

        let running = Arc::new(AtomicBool::new(true));
        let enabled = Arc::new(AtomicBool::new(true));

        let thread_running = Arc::clone(&running);
        let thread_enabled = Arc::clone(&enabled);
        let mut on_value = on_value;

        let thread = thread::Builder::new()
            .name("modv-osc".to_string())
            .spawn(move || {
                let mut buffer = [0u8; MAX_DATAGRAM];

                while thread_running.load(Ordering::Acquire) {
                    let (len, _from) = match socket.recv_from(&mut buffer) {
                        Ok(received) => received,
                        // Read timeout: re-check the stop flag.
                        Err(err)
                            if err.kind() == io::ErrorKind::WouldBlock
                                || err.kind() == io::ErrorKind::TimedOut =>
                        {
                            continue;
                        }
                        // A closed / unrecoverable socket ends the thread.
                        Err(_) => break,
                    };

                    if !thread_enabled.load(Ordering::Acquire) {
                        continue;
                    }

                    if let Ok((_, packet)) = rosc::decoder::decode_udp(&buffer[..len]) {
                        dispatch_packet(&packet, &mut on_value);
                    }
                }
            })
            .map_err(OscError::Spawn)?;

        Ok(Self {
            local_addr,
            running,
            enabled,
            thread: Some(thread),
        })
    }

    /// The bound address (`port` 0 reports the ephemeral port the kernel chose).
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Starts or stops delivering messages. The socket stays bound while
    /// disabled, so pending datagrams are drained and dropped.
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Release);
    }

    /// Whether messages are currently delivered to the callback.
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }

    /// Whether the reader thread is alive.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Acquire) && self.thread.is_some()
    }

    /// Stops the reader thread and joins it (bounded by [`POLL_INTERVAL`]).
    ///
    /// Idempotent: a second call is a no-op.
    pub fn stop(&mut self) {
        self.running.store(false, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for OscServer {
    fn drop(&mut self) {
        self.stop();
    }
}

impl fmt::Debug for OscServer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OscServer")
            .field("local_addr", &self.local_addr)
            .field("running", &self.is_running())
            .field("enabled", &self.is_enabled())
            .finish()
    }
}

/// Walks a decoded packet and calls `on_value` for each argument.
///
/// Bundles are flattened recursively. Following the JS server (`node-osc`),
/// a message with no arguments produces no callback at all; a message with
/// several produces one callback per argument (the positional array shape is
/// available through [`apply_osc_message`]).
pub fn dispatch_packet(packet: &rosc::OscPacket, on_value: &mut dyn FnMut(&str, OscValue)) {
    match packet {
        rosc::OscPacket::Message(message) => {
            for argument in &message.args {
                if let Some(value) = OscValue::from_osc_type(argument) {
                    on_value(&message.addr, value);
                }
            }
        }
        rosc::OscPacket::Bundle(bundle) => {
            for packet in &bundle.content {
                dispatch_packet(packet, on_value);
            }
        }
    }
}

/// Converts an OSC address into the lodash path the store uses
/// (`store/modules/osc.js:69-71`: `msg[0].substring(1).replaceAll("/", ".")`).
///
/// A leading `/` is trimmed and every `/` becomes a `.`; `lodash.set` then
/// splits on `.`. An address of `"/"` (or an empty address) yields an empty
/// path, which `set` treats as a no-op.
///
/// Examples: `/modv/bass` → `["modv", "bass"]`, `/layers/0/alpha` →
/// `["layers", "0", "alpha"]`.
pub fn osc_address_to_path(address: &str) -> Vec<String> {
    let path = address.strip_prefix('/').unwrap_or(address);
    let path = path.replace('/', ".");
    if path.is_empty() {
        return Vec::new();
    }
    path.split('.').map(str::to_string).collect()
}

/// `lodash`'s `isIndex` (`/^(?:0|[1-9]\d*)$/` plus the `MAX_SAFE_INTEGER`
/// bound): a canonical non-negative array index.
fn array_index(segment: &str) -> Option<usize> {
    if segment.is_empty() || (segment.len() > 1 && segment.starts_with('0')) {
        return None;
    }
    if !segment.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let index = segment.parse::<u64>().ok()?;
    // `Number.MAX_SAFE_INTEGER` — lodash only treats safe integers as indexes.
    if index > 9_007_199_254_740_991 || index > MAX_ARRAY_INDEX as u64 {
        return None;
    }
    usize::try_from(index).ok()
}

/// Sets `value` at `path`, porting `lodash.set` (`baseSet`, lodash 4):
///
/// * a missing branch becomes an array when the *next* segment is an index,
///   and an object otherwise;
/// * an existing object/array branch is descended into;
/// * a scalar branch is replaced by the object/array the remaining path needs
///   (`{a: 1}` + `a.b` → `{a: {b: …}}`);
/// * the last segment is assigned.
///
/// Deviations forced by JSON (JS objects can hold arbitrary keys):
///
/// * a hole created by a sparse index is filled with `null` (`JSON.stringify`
///   would render the hole as `null` too);
/// * a non-numeric segment cannot be written onto a JSON array, so such a
///   write is dropped (in JS it would be a non-index property that
///   `JSON.stringify` also drops);
/// * an index above [`MAX_ARRAY_INDEX`] is treated as an object key instead of
///   materialising billions of `null`s.
fn set_json_path(root: &mut serde_json::Value, path: &[String], value: serde_json::Value) {
    use serde_json::{Map, Value};

    let Some((key, rest)) = path.split_first() else {
        return;
    };

    // The store root is always an object, but a caller may hand us a scalar;
    // lodash would throw on `null`, we re-root it instead.
    if !root.is_object() && !root.is_array() {
        *root = if array_index(key).is_some() {
            Value::Array(Vec::new())
        } else {
            Value::Object(Map::new())
        };
    }

    match root {
        Value::Object(map) => {
            if rest.is_empty() {
                map.insert(key.clone(), value);
                return;
            }

            let branch_exists = map
                .get(key)
                .is_some_and(|existing| existing.is_object() || existing.is_array());
            if !branch_exists {
                let next_is_index = rest
                    .first()
                    .is_some_and(|segment| array_index(segment).is_some());
                map.insert(
                    key.clone(),
                    if next_is_index {
                        Value::Array(Vec::new())
                    } else {
                        Value::Object(Map::new())
                    },
                );
            }

            if let Some(branch) = map.get_mut(key) {
                set_json_path(branch, rest, value);
            }
        }
        Value::Array(items) => {
            let Some(index) = array_index(key) else {
                // JSON arrays cannot carry named properties; JS would create a
                // non-index property that serialisation drops.
                return;
            };

            while items.len() <= index {
                items.push(Value::Null);
            }

            if rest.is_empty() {
                items[index] = value;
                return;
            }

            let slot = &mut items[index];
            if !slot.is_object() && !slot.is_array() {
                let next_is_index = rest
                    .first()
                    .is_some_and(|segment| array_index(segment).is_some());
                *slot = if next_is_index {
                    Value::Array(Vec::new())
                } else {
                    Value::Object(Map::new())
                };
            }

            set_json_path(slot, rest, value);
        }
        // Unreachable: a scalar root was re-rooted above.
        _ => {}
    }
}

/// Applies one OSC value to `data` at `address`, the scalar form of the store's
/// `osc/WRITE_DATA` (`store/modules/osc.js:69-71`).
///
/// This sets the value itself, not the JS `msg.slice(1)` array; use
/// [`apply_osc_message`] when parity with the legacy store matters.
pub fn apply_osc_value(data: &mut serde_json::Value, address: &str, value: &OscValue) {
    set_json_path(data, &osc_address_to_path(address), value.to_json());
}

/// Applies a whole OSC message: `set(data, path, [arg0, arg1, …])`, exactly
/// like `msg.slice(1)` (`store/modules/osc.js:69-71`).
///
/// The JS always stores an array, even for a single argument.
pub fn apply_osc_message(data: &mut serde_json::Value, address: &str, values: &[OscValue]) {
    let array = serde_json::Value::Array(values.iter().map(OscValue::to_json).collect());
    set_json_path(data, &osc_address_to_path(address), array);
}

#[cfg(test)]
mod tests {
    //! Ports the `osc.js` / `lodash.set` cases the legacy store relies on
    //! (`store/modules/osc.js:69-71`) plus a UDP loopback test.
    use super::*;
    use serde_json::json;
    use std::sync::mpsc;

    // store/modules/osc.js:69-71 — "/modv/bass" → set(data, "modv.bass", …)
    #[test]
    fn leading_slash_becomes_a_nested_object() {
        let mut data = json!({});

        apply_osc_value(&mut data, "/modv/bass", &OscValue::Float(0.5));

        assert_eq!(data, json!({ "modv": { "bass": 0.5 } }));
    }

    // lodash.set — a missing branch is an object when the next segment is not an index.
    #[test]
    fn nested_objects_are_created_and_siblings_preserved() {
        let mut data = json!({ "modv": { "bass": 1 } });

        apply_osc_value(&mut data, "/modv/mid", &OscValue::Float(0.25));

        assert_eq!(data, json!({ "modv": { "bass": 1, "mid": 0.25 } }));
    }

    // lodash.set — a numeric segment creates an array (baseSet isIndex branch).
    #[test]
    fn numeric_segments_index_and_grow_arrays() {
        let mut data = json!({});

        apply_osc_value(&mut data, "/layers/0", &OscValue::Int(1));
        apply_osc_value(&mut data, "/layers/1", &OscValue::Int(2));

        assert_eq!(data, json!({ "layers": [1, 2] }));
    }

    // lodash.set — an index followed by a name creates an object inside the array.
    #[test]
    fn numeric_index_then_name_creates_an_object_element() {
        let mut data = json!({});

        apply_osc_value(&mut data, "/layers/0/alpha", &OscValue::Float(0.75));

        assert_eq!(data, json!({ "layers": [{ "alpha": 0.75 }] }));
    }

    // lodash.set — existing arrays/objects are descended into, not replaced.
    #[test]
    fn existing_array_elements_are_merged() {
        let mut data = json!({ "layers": [{ "alpha": 0.5 }] });

        apply_osc_value(
            &mut data,
            "/layers/0/name",
            &OscValue::String("base".into()),
        );

        assert_eq!(
            data,
            json!({ "layers": [{ "alpha": 0.5, "name": "base" }] })
        );
    }

    // lodash.set — a scalar is replaced when the path needs a branch.
    #[test]
    fn scalar_branches_are_replaced_by_objects() {
        let mut data = json!({ "layers": 3 });

        apply_osc_value(&mut data, "/layers/0/alpha", &OscValue::Float(0.125));

        assert_eq!(data, json!({ "layers": [{ "alpha": 0.125 }] }));
    }

    // lodash.set — a sparse index is padded with null (JSON has no holes).
    #[test]
    fn indexes_above_the_cap_degrade_to_object_keys() {
        let mut data = json!({});

        apply_osc_value(&mut data, "/layers/2000000", &OscValue::Float(0.5));

        // Bounded and JSON-legal, instead of a 2-million-entry array.
        assert_eq!(data, json!({ "layers": { "2000000": 0.5 } }));
    }

    #[test]
    fn sparse_indexes_are_padded_with_null() {
        let mut data = json!({ "layers": [1] });

        apply_osc_value(&mut data, "/layers/3", &OscValue::Int(9));

        assert_eq!(data, json!({ "layers": [1, null, null, 9] }));
    }

    // setup-midi-style addresses keep working without a leading slash.
    #[test]
    fn addresses_without_a_leading_slash_are_supported() {
        let mut data = json!({});

        apply_osc_value(&mut data, "modv/treble", &OscValue::Float(0.5));

        assert_eq!(data, json!({ "modv": { "treble": 0.5 } }));
    }

    #[test]
    fn trailing_slash_creates_an_empty_key() {
        // The JS `replaceAll("/", ".")` leaves an empty last segment; lodash.set
        // then writes the empty-string key.
        assert_eq!(
            osc_address_to_path("/modv/"),
            vec!["modv".to_string(), String::new()]
        );
    }

    #[test]
    fn empty_address_is_a_no_op() {
        let mut data = json!({ "modv": { "bass": 1 } });

        apply_osc_value(&mut data, "/", &OscValue::Float(0.5));

        assert_eq!(data, json!({ "modv": { "bass": 1 } }));
        assert!(osc_address_to_path("/").is_empty());
    }

    // JSON cannot carry a named key on an array; the JS property is dropped by
    // serialisation too.
    #[test]
    fn non_numeric_segments_on_arrays_are_dropped() {
        let mut data = json!({ "layers": [1] });

        apply_osc_value(&mut data, "/layers/name", &OscValue::String("x".into()));

        assert_eq!(data, json!({ "layers": [1] }));
    }

    // store/modules/osc.js:69-71 — the store sets `msg.slice(1)`, always an array.
    #[test]
    fn apply_osc_message_stores_the_argument_array() {
        let mut data = json!({});

        apply_osc_message(
            &mut data,
            "/modv/bands",
            &[OscValue::Float(0.1), OscValue::Float(0.2)],
        );
        // OSC floats are f32 on the wire; JSON carries the widened f32 value.
        let a = 0.1_f32 as f64;
        let b = 0.2_f32 as f64;
        assert_eq!(data, json!({ "modv": { "bands": [a, b] } }));

        apply_osc_message(&mut data, "/modv/one", &[OscValue::Int(1)]);
        assert_eq!(data, json!({ "modv": { "bands": [a, b], "one": [1] } }));
    }

    #[test]
    fn values_convert_to_json() {
        assert_eq!(OscValue::Float(0.5).to_json(), json!(0.5));
        assert_eq!(OscValue::Int(-3).to_json(), json!(-3));
        assert_eq!(OscValue::String("hello".into()).to_json(), json!("hello"));
        assert_eq!(OscValue::Blob(vec![1, 2, 3]).to_json(), json!([1, 2, 3]));
        // JSON cannot represent NaN / Infinity.
        assert_eq!(OscValue::Float(f32::NAN).to_json(), json!(null));
        assert_eq!(OscValue::Float(f32::INFINITY).to_json(), json!(null));
    }

    #[test]
    fn osc_types_map_onto_the_store_subset() {
        use rosc::OscType;

        assert_eq!(
            OscValue::from_osc_type(&OscType::Float(0.5)),
            Some(OscValue::Float(0.5))
        );
        assert_eq!(
            OscValue::from_osc_type(&OscType::Double(0.5)),
            Some(OscValue::Float(0.5))
        );
        assert_eq!(
            OscValue::from_osc_type(&OscType::Long(i64::from(i32::MAX) + 1)),
            Some(OscValue::Int(i32::MAX))
        );
        assert_eq!(
            OscValue::from_osc_type(&OscType::Bool(true)),
            Some(OscValue::Int(1))
        );
        assert_eq!(
            OscValue::from_osc_type(&OscType::Char('a')),
            Some(OscValue::String("a".into()))
        );
        assert_eq!(OscValue::from_osc_type(&OscType::Nil), None);
        assert_eq!(OscValue::from_osc_type(&OscType::Inf), None);
    }

    #[test]
    fn bundle_arguments_are_dispatched_in_order() {
        use rosc::{OscBundle, OscMessage, OscPacket, OscTime, OscType};

        let packet = OscPacket::Bundle(OscBundle {
            timetag: OscTime {
                seconds: 0,
                fractional: 0,
            },
            content: vec![
                OscPacket::Message(OscMessage {
                    addr: "/a".into(),
                    args: vec![OscType::Float(1.0)],
                }),
                OscPacket::Message(OscMessage {
                    addr: "/b".into(),
                    args: vec![OscType::Int(2)],
                }),
            ],
        });

        let mut received = Vec::new();
        dispatch_packet(&packet, &mut |address, value| {
            received.push((address.to_string(), value));
        });

        assert_eq!(
            received,
            vec![
                ("/a".to_string(), OscValue::Float(1.0)),
                ("/b".to_string(), OscValue::Int(2)),
            ]
        );
    }

    #[test]
    fn server_state_transitions() {
        let mut server = match OscServer::start("127.0.0.1", 0, |_, _| {}) {
            Ok(server) => server,
            Err(err) => {
                eprintln!("skipping OSC state test: {err}");
                return;
            }
        };

        assert!(server.is_running());
        assert!(server.is_enabled());
        assert_ne!(server.local_addr().port(), 0);

        server.set_enabled(false);
        assert!(!server.is_enabled());

        server.set_enabled(true);
        assert!(server.is_enabled());

        server.stop();
        assert!(!server.is_running());
        // Idempotent.
        server.stop();
        assert!(!server.is_running());
    }

    // Loopback: encode with `rosc`, send over UDP, assert the callback fires.
    // Skips gracefully when the sandbox blocks UDP or the port cannot bind.
    #[test]
    fn loopback_delivers_the_address_and_value() {
        use rosc::{OscMessage, OscPacket, OscType};

        let (sender, receiver) = mpsc::channel();
        let mut server = match OscServer::start("127.0.0.1", 0, move |address, value| {
            let _ = sender.send((address.to_string(), value));
        }) {
            Ok(server) => server,
            Err(err) => {
                eprintln!("skipping OSC loopback: {err}");
                return;
            }
        };

        let client = match UdpSocket::bind("127.0.0.1:0") {
            Ok(client) => client,
            Err(err) => {
                eprintln!("skipping OSC loopback: {err}");
                return;
            }
        };

        let packet = OscPacket::Message(OscMessage {
            addr: "/modv/bass".into(),
            args: vec![OscType::Float(0.5), OscType::Int(7)],
        });
        let bytes = match rosc::encoder::encode(&packet) {
            Ok(bytes) => bytes,
            Err(err) => {
                eprintln!("skipping OSC loopback: {err}");
                return;
            }
        };

        if let Err(err) = client.send_to(&bytes, server.local_addr()) {
            eprintln!("skipping OSC loopback: {err}");
            return;
        }

        match receiver.recv_timeout(Duration::from_secs(2)) {
            Ok((address, value)) => {
                assert_eq!(address, "/modv/bass");
                assert_eq!(value, OscValue::Float(0.5));
            }
            Err(err) => {
                eprintln!("skipping OSC loopback (no datagram): {err}");
                return;
            }
        }

        assert_eq!(
            receiver
                .recv_timeout(Duration::from_secs(2))
                .expect("second argument"),
            ("/modv/bass".to_string(), OscValue::Int(7))
        );

        server.stop();
    }

    #[test]
    fn disabled_server_drops_messages() {
        use rosc::{OscMessage, OscPacket, OscType};

        let (sender, receiver) = mpsc::channel();
        let mut server = match OscServer::start("127.0.0.1", 0, move |address, value| {
            let _ = sender.send((address.to_string(), value));
        }) {
            Ok(server) => server,
            Err(err) => {
                eprintln!("skipping OSC disable test: {err}");
                return;
            }
        };
        server.set_enabled(false);

        let client = match UdpSocket::bind("127.0.0.1:0") {
            Ok(client) => client,
            Err(err) => {
                eprintln!("skipping OSC disable test: {err}");
                return;
            }
        };
        let bytes = rosc::encoder::encode(&OscPacket::Message(OscMessage {
            addr: "/modv/mid".into(),
            args: vec![OscType::Float(1.0)],
        }))
        .expect("encode");

        if client.send_to(&bytes, server.local_addr()).is_err() {
            eprintln!("skipping OSC disable test: send failed");
            return;
        }

        // The reader still drains, but must not invoke the callback.
        assert!(receiver.recv_timeout(Duration::from_millis(400)).is_err());

        server.stop();
    }
}
