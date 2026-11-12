//! JSON messages carried by the control channel.
//!
//! spec: docs/rust-hybrid.md — “Protocol specification — control channel”.
//!
//! The wire JSON shapes are exactly the ones shown in the spec:
//!
//! ```json
//! { "id": 42, "method": "scenes/applyScene", "params": { "sceneId": "..." } }
//! { "id": 42, "ok": true, "result": null }
//! { "id": 42, "ok": false, "error": { "code": "unknown_scene", "message": "..." } }
//! { "event": "state/delta",
//!   "data": { "type": "groups/UPDATE_GROUP", "payload": { "...": "..." } } }
//! ```
//!
//! Ambiguity resolved: the spec's `{ "id": 42, "ok": true, "result": null }`
//! sample is accepted on the wire, but serde normalises an explicit JSON `null`
//! into `None` for `Option<serde_json::Value>`, so a re-serialised `Response`
//! with `result: None` omits the key entirely (see `skip_serializing_if`).
//! `Response::ok(id, serde_json::Value::Null)` serialises back to exactly
//! `"result": null`, matching the sample byte-for-byte.

use serde::{Deserialize, Serialize};
use std::fmt;

use crate::frame::{self, KIND_EVENT, KIND_REQUEST, KIND_RESPONSE};

/// A JSON request. spec: `{ "id", "method", "params" }`; `params` defaults to
/// `null` when omitted by the caller.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    /// Caller-chosen request id, echoed back in the matching [`Response`].
    pub id: u64,
    /// Method name, e.g. `"scenes/applyScene"` or `"engine/shutdown"`.
    pub method: String,
    /// Method parameters; defaults to JSON `null` when absent.
    #[serde(default)]
    pub params: serde_json::Value,
}

impl Request {
    /// Frame this request as a [`crate::frame::KIND_REQUEST`] frame.
    ///
    /// # Panics
    ///
    /// Panics only if the resulting frame exceeds [`frame::MAX_FRAME_BYTES`]
    /// (64 MiB), which a request never legitimately does.
    pub fn into_frame(self) -> Vec<u8> {
        let payload = serde_json::to_vec(&self).expect("Request is always serialisable");
        frame::encode_frame(KIND_REQUEST, &payload)
            .expect("request payload exceeds MAX_FRAME_BYTES (64 MiB)")
    }
}

/// Structured error returned by a failed [`Response`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResponseError {
    /// Machine-readable error code, e.g. `"unknown_scene"`.
    pub code: String,
    /// Human-readable detail.
    pub message: String,
}

/// A JSON response. spec: `{ "id", "ok", "result"? , "error"? }`; `result` and
/// `error` are omitted when absent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    /// Id of the request this response answers.
    pub id: u64,
    /// Whether the method executed successfully.
    pub ok: bool,
    /// Method return value; absent on failure, present (possibly JSON `null`) on success.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    /// Structured error; present only when `ok` is `false`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ResponseError>,
}

impl Response {
    /// Build a success response carrying `result` (use `serde_json::Value::Null`
    /// for the spec's `"result": null` shape).
    pub fn ok(id: u64, result: serde_json::Value) -> Self {
        Self {
            id,
            ok: true,
            result: Some(result),
            error: None,
        }
    }

    /// Build a failure response with a machine-readable `code` and a
    /// human-readable `message`.
    pub fn err(id: u64, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            id,
            ok: false,
            result: None,
            error: Some(ResponseError {
                code: code.into(),
                message: message.into(),
            }),
        }
    }

    /// Frame this response as a [`crate::frame::KIND_RESPONSE`] frame.
    ///
    /// # Panics
    ///
    /// Panics only if the resulting frame exceeds [`frame::MAX_FRAME_BYTES`]
    /// (64 MiB).
    pub fn into_frame(self) -> Vec<u8> {
        let payload = serde_json::to_vec(&self).expect("Response is always serialisable");
        frame::encode_frame(KIND_RESPONSE, &payload)
            .expect("response payload exceeds MAX_FRAME_BYTES (64 MiB)")
    }
}

/// A JSON event (server-initiated). spec: `{ "event", "data" }`; `data`
/// defaults to `null` when omitted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// Event name, e.g. `"state/delta"`, `"preview/ready"`, `"preview/frame"`.
    pub event: String,
    /// Event payload; defaults to JSON `null` when absent.
    #[serde(default)]
    pub data: serde_json::Value,
}

impl Event {
    /// Convenience constructor.
    pub fn new(event: impl Into<String>, data: serde_json::Value) -> Self {
        Self {
            event: event.into(),
            data,
        }
    }

    /// Frame this event as a [`crate::frame::KIND_EVENT`] frame.
    ///
    /// # Panics
    ///
    /// Panics only if the resulting frame exceeds [`frame::MAX_FRAME_BYTES`]
    /// (64 MiB).
    pub fn into_frame(self) -> Vec<u8> {
        let payload = serde_json::to_vec(&self).expect("Event is always serialisable");
        frame::encode_frame(KIND_EVENT, &payload)
            .expect("event payload exceeds MAX_FRAME_BYTES (64 MiB)")
    }
}

/// A decoded control-channel message.
///
/// spec: kinds 0/1/2 decode as `Request`/`Response`/`Event`. Kind 3
/// (`KIND_BINARY`) is reserved by the spec and decodes as
/// [`MessageError::InvalidKind`] for now — it is left as an explicit error so
/// call sites can be audited when the binary channel is defined.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    /// A JSON request frame (kind 0).
    Request(Request),
    /// A JSON response frame (kind 1).
    Response(Response),
    /// A JSON event frame (kind 2).
    Event(Event),
}

/// Errors produced when decoding a message payload.
#[derive(Debug)]
pub enum MessageError {
    /// The frame kind was not 0, 1 or 2. Kind 3 is reserved (see [`Incoming`]).
    InvalidKind(u8),
    /// The payload was not valid JSON or did not match the message shape.
    JsonError(String),
}

impl fmt::Display for MessageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MessageError::InvalidKind(kind) => {
                write!(
                    f,
                    "unsupported frame kind {kind} (0/1/2 expected; 3 is reserved)"
                )
            }
            MessageError::JsonError(detail) => write!(f, "invalid JSON message: {detail}"),
        }
    }
}

impl std::error::Error for MessageError {}

/// Decode a frame's kind + payload into a message.
///
/// Kind 3 (`KIND_BINARY`, reserved) and any other unknown kind produce
/// [`MessageError::InvalidKind`].
pub fn from_frame(kind: u8, payload: &[u8]) -> Result<Incoming, MessageError> {
    match kind {
        KIND_REQUEST => serde_json::from_slice(payload)
            .map(Incoming::Request)
            .map_err(|e| MessageError::JsonError(e.to_string())),
        KIND_RESPONSE => serde_json::from_slice(payload)
            .map(Incoming::Response)
            .map_err(|e| MessageError::JsonError(e.to_string())),
        KIND_EVENT => serde_json::from_slice(payload)
            .map(Incoming::Event)
            .map_err(|e| MessageError::JsonError(e.to_string())),
        other => Err(MessageError::InvalidKind(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn request_parses_spec_sample() {
        // spec: { "id": 42, "method": "scenes/applyScene", "params": { "sceneId": "..." } }
        let payload = br#"{"id":42,"method":"scenes/applyScene","params":{"sceneId":"..."}}"#;
        let incoming = from_frame(KIND_REQUEST, payload).expect("decode");
        match incoming {
            Incoming::Request(request) => {
                assert_eq!(request.id, 42);
                assert_eq!(request.method, "scenes/applyScene");
                assert_eq!(request.params, json!({ "sceneId": "..." }));
            }
            other => panic!("expected request, got {other:?}"),
        }
    }

    #[test]
    fn request_params_default_to_null() {
        let payload = br#"{"id":7,"method":"engine/shutdown"}"#;
        let incoming = from_frame(KIND_REQUEST, payload).expect("decode");
        match incoming {
            Incoming::Request(request) => {
                assert_eq!(request.id, 7);
                assert_eq!(request.params, serde_json::Value::Null);
            }
            other => panic!("expected request, got {other:?}"),
        }
    }

    #[test]
    fn response_ok_roundtrip() {
        // spec: { "id": 42, "ok": true, "result": null }
        let response = Response::ok(42, serde_json::Value::Null);
        let frame = response.clone().into_frame();
        let incoming = from_frame(KIND_RESPONSE, &frame[5..]).expect("decode");
        match incoming {
            Incoming::Response(decoded) => {
                // serde maps JSON null to Option::None, so the wire round
                // trip collapses Some(Null) — the spec's "result": null is
                // preserved on the wire, not in the struct.
                assert_eq!(decoded.id, 42);
                assert!(decoded.ok);
                assert_eq!(decoded.result, None);
                assert!(decoded.error.is_none());
            }
            other => panic!("expected response, got {other:?}"),
        }
        assert_eq!(&frame[5..], br#"{"id":42,"ok":true,"result":null}"#);
        // The exact spec bytes round-trip too.
        let wire = br#"{"id":42,"ok":true,"result":null}"#;
        let decoded: Response = serde_json::from_slice(wire).expect("decode wire");
        assert!(decoded.ok);
        assert_eq!(decoded.result, None, "JSON null decodes to None (documented collapse)");
        assert!(decoded.error.is_none());
    }

    #[test]
    fn response_ok_omits_error_key() {
        let value = serde_json::to_value(Response::ok(1, json!({ "a": 1 }))).expect("serialize");
        assert_eq!(value["id"], 1);
        assert_eq!(value["ok"], true);
        assert_eq!(value["result"], json!({ "a": 1 }));
        assert!(value.get("error").is_none(), "error key must be omitted");
    }

    #[test]
    fn response_err_roundtrip() {
        // spec: { "id": 42, "ok": false, "error": { "code": "unknown_scene", "message": "..." } }
        let response = Response::err(42, "unknown_scene", "no such scene");
        let value = serde_json::to_value(&response).expect("serialize");
        assert_eq!(value["ok"], false);
        assert!(value.get("result").is_none(), "result key must be omitted");
        assert_eq!(value["error"]["code"], "unknown_scene");
        assert_eq!(value["error"]["message"], "no such scene");

        let frame = response.clone().into_frame();
        let incoming = from_frame(KIND_RESPONSE, &frame[5..]).expect("decode");
        match incoming {
            Incoming::Response(decoded) => assert_eq!(decoded, response),
            other => panic!("expected response, got {other:?}"),
        }
    }

    #[test]
    fn event_state_delta_roundtrip() {
        // spec: { "event": "state/delta",
        //         "data": { "type": "groups/UPDATE_GROUP", "payload": { "...": "..." } } }
        let event = Event::new(
            "state/delta",
            json!({ "type": "groups/UPDATE_GROUP", "payload": { "...": "..." } }),
        );
        let frame = event.clone().into_frame();
        let incoming = from_frame(KIND_EVENT, &frame[5..]).expect("decode");
        match incoming {
            Incoming::Event(decoded) => assert_eq!(decoded, event),
            other => panic!("expected event, got {other:?}"),
        }
        assert_eq!(event.data["type"], "groups/UPDATE_GROUP");
    }

    #[test]
    fn event_data_defaults_to_null() {
        let payload = br#"{"event":"ping"}"#;
        let incoming = from_frame(KIND_EVENT, payload).expect("decode");
        match incoming {
            Incoming::Event(event) => assert_eq!(event.data, serde_json::Value::Null),
            other => panic!("expected event, got {other:?}"),
        }
    }

    #[test]
    fn invalid_json_is_a_json_error() {
        match from_frame(KIND_REQUEST, b"{not json") {
            Err(MessageError::JsonError(_)) => {}
            other => panic!("expected JsonError, got {other:?}"),
        }
        match from_frame(KIND_RESPONSE, b"") {
            Err(MessageError::JsonError(_)) => {}
            other => panic!("expected JsonError, got {other:?}"),
        }
    }

    #[test]
    fn kind_3_binary_is_reserved() {
        // spec: kind 3 = binary (reserved); rejected until the binary channel is defined.
        match from_frame(crate::frame::KIND_BINARY, b"anything") {
            Err(MessageError::InvalidKind(3)) => {}
            other => panic!("expected InvalidKind(3), got {other:?}"),
        }
        match from_frame(0xFE, b"{}") {
            Err(MessageError::InvalidKind(0xFE)) => {}
            other => panic!("expected InvalidKind(0xFE), got {other:?}"),
        }
    }

    #[test]
    fn request_into_frame_wire_layout() {
        let request = Request {
            id: 9,
            method: "state/snapshot".to_string(),
            params: serde_json::Value::Null,
        };
        let frame = request.into_frame();
        assert_eq!(frame[4], KIND_REQUEST);
        let len = u32::from_le_bytes(frame[..4].try_into().unwrap()) as usize;
        assert_eq!(len, frame.len() - 5);
        let decoded: Request = serde_json::from_slice(&frame[5..]).expect("decode");
        assert_eq!(decoded.id, 9);
        assert_eq!(decoded.method, "state/snapshot");
    }
}
