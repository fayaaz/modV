//! Control-channel framing.
//!
//! spec: docs/rust-hybrid.md — “Protocol specification — control channel”:
//!
//! ```text
//! [ u32 LE payload length ][ u8 kind ][ payload bytes ]
//! ```
//!
//! The `kind` byte is opaque to this module; see [`crate::message`] for how the
//! JSON kinds (0/1/2) are interpreted. Kind 3 is reserved by the spec and is
//! passed through here like any other kind.

use std::fmt;

/// JSON request frame. spec: kind table — 0 = JSON request.
pub const KIND_REQUEST: u8 = 0;
/// JSON response frame. spec: kind table — 1 = JSON response.
pub const KIND_RESPONSE: u8 = 1;
/// JSON event frame. spec: kind table — 2 = JSON event.
pub const KIND_EVENT: u8 = 2;
/// Binary frame (reserved by the spec). spec: kind table — 3 = binary (reserved).
pub const KIND_BINARY: u8 = 3;

/// Largest payload (in bytes) a frame may carry. spec: 64 MiB hard cap.
pub const MAX_FRAME_BYTES: u32 = 64 * 1024 * 1024;

/// Length of the frame header: `u32` payload length + `u8` kind.
const HEADER_BYTES: usize = 5;

/// Errors produced while encoding or decoding frames.
///
/// Note: after a [`FrameError::PayloadTooLarge`] the byte stream can no longer
/// be resynchronised (the payload length is attacker/driver-controlled garbage),
/// so [`FrameReader::push`] clears its internal buffer and the connection must
/// be dropped. Callers that maintain connections (see [`crate::socket`]) treat
/// a frame error as fatal for that client.
#[derive(Debug)]
pub enum FrameError {
    /// The frame header declared a payload larger than [`MAX_FRAME_BYTES`].
    PayloadTooLarge(u32),
    /// `payload length + 5` overflowed the platform `usize` while computing the
    /// total frame size (only reachable on 32-bit targets), or the payload did
    /// not fit into a `u32` while encoding.
    LengthOverflow,
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FrameError::PayloadTooLarge(len) => {
                write!(
                    f,
                    "frame payload too large: {len} bytes (max {MAX_FRAME_BYTES})"
                )
            }
            FrameError::LengthOverflow => write!(f, "frame length overflows addressable memory"),
        }
    }
}

impl std::error::Error for FrameError {}

/// Encode a frame into `[ u32 LE length ][ u8 kind ][ payload ]`.
///
/// Fails with [`FrameError::PayloadTooLarge`] when `payload` exceeds
/// [`MAX_FRAME_BYTES`], or [`FrameError::LengthOverflow`] when its length does
/// not fit into a `u32`.
pub fn encode_frame(kind: u8, payload: &[u8]) -> Result<Vec<u8>, FrameError> {
    let len: u32 = payload
        .len()
        .try_into()
        .map_err(|_| FrameError::LengthOverflow)?;
    if len > MAX_FRAME_BYTES {
        return Err(FrameError::PayloadTooLarge(len));
    }
    let mut out = Vec::with_capacity(HEADER_BYTES + payload.len());
    out.extend_from_slice(&len.to_le_bytes());
    out.push(kind);
    out.extend_from_slice(payload);
    Ok(out)
}

/// Incremental frame decoder.
///
/// Bytes are appended with [`FrameReader::push`]; every complete frame that is
/// available after the append is returned in wire order. Split frames and
/// back-to-back frames in a single append are handled transparently.
pub struct FrameReader {
    buf: Vec<u8>,
}

impl FrameReader {
    /// Create an empty reader.
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    /// Append `bytes` to the internal buffer and return every frame that is
    /// complete after the append.
    ///
    /// Returns an empty `Vec` when only a partial frame is available. A frame
    /// whose header declares more than [`MAX_FRAME_BYTES`] yields one
    /// [`FrameError::PayloadTooLarge`] entry and clears the buffer; the caller
    /// must drop the connection in that case (the stream is unrecoverable).
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Result<(u8, Vec<u8>), FrameError>> {
        let mut frames = Vec::new();
        self.buf.extend_from_slice(bytes);

        loop {
            if self.buf.len() < HEADER_BYTES {
                break;
            }
            let len = u32::from_le_bytes(self.buf[..4].try_into().expect("4-byte slice"));
            // spec: length + header must not overflow usize (checked before the
            // MAX_FRAME_BYTES clamp so the check is meaningful on 32-bit).
            let total = match (len as usize).checked_add(HEADER_BYTES) {
                Some(total) => total,
                None => {
                    frames.push(Err(FrameError::LengthOverflow));
                    self.buf.clear();
                    break;
                }
            };
            if len > MAX_FRAME_BYTES {
                frames.push(Err(FrameError::PayloadTooLarge(len)));
                self.buf.clear();
                break;
            }
            if self.buf.len() < total {
                break;
            }
            let kind = self.buf[4];
            let payload = self.buf[HEADER_BYTES..total].to_vec();
            self.buf.drain(..total);
            frames.push(Ok((kind, payload)));
        }

        frames
    }
}

impl Default for FrameReader {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_push_roundtrip_single_frame() {
        let payload = br#"{"id":42,"method":"engine/shutdown"}"#;
        let frame = encode_frame(KIND_REQUEST, payload).expect("encode");
        // spec: header layout
        assert_eq!(&frame[..4], &(payload.len() as u32).to_le_bytes());
        assert_eq!(frame[4], KIND_REQUEST);

        let mut reader = FrameReader::new();
        let frames = reader.push(&frame);
        assert_eq!(frames.len(), 1);
        let (kind, body) = frames.into_iter().next().unwrap().expect("frame");
        assert_eq!(kind, KIND_REQUEST);
        assert_eq!(body, payload);
    }

    #[test]
    fn push_byte_at_a_time() {
        let payload = b"hello vmodv";
        let frame = encode_frame(KIND_EVENT, payload).expect("encode");

        let mut reader = FrameReader::new();
        let mut results = Vec::new();
        for byte in &frame {
            results.extend(reader.push(&[*byte]));
        }
        assert_eq!(results.len(), 1);
        let (kind, body) = results.pop().unwrap().expect("frame");
        assert_eq!(kind, KIND_EVENT);
        assert_eq!(body, payload);
    }

    #[test]
    fn push_two_frames_in_one_buffer() {
        let first = encode_frame(KIND_REQUEST, br#"{"id":1,"method":"a"}"#).expect("encode");
        let second = encode_frame(KIND_EVENT, br#"{"event":"ping"}"#).expect("encode");
        let mut blob = first.clone();
        blob.extend_from_slice(&second);

        let mut reader = FrameReader::new();
        let frames = reader.push(&blob);
        assert_eq!(frames.len(), 2);
        let (kind0, body0) = frames[0].as_ref().expect("frame 0");
        let (kind1, body1) = frames[1].as_ref().expect("frame 1");
        assert_eq!(*kind0, KIND_REQUEST);
        assert_eq!(body0.as_slice(), br#"{"id":1,"method":"a"}"#);
        assert_eq!(*kind1, KIND_EVENT);
        assert_eq!(body1.as_slice(), br#"{"event":"ping"}"#);
    }

    #[test]
    fn declared_length_above_max_is_rejected() {
        let mut reader = FrameReader::new();
        // Declare a payload length of u32::MAX - 1; must fail before allocating.
        let mut header = (MAX_FRAME_BYTES + 1).to_le_bytes().to_vec();
        header.push(KIND_REQUEST);
        let frames = reader.push(&header);
        assert_eq!(frames.len(), 1);
        match &frames[0] {
            Err(FrameError::PayloadTooLarge(len)) => assert_eq!(*len, MAX_FRAME_BYTES + 1),
            other => panic!("expected PayloadTooLarge, got {other:?}"),
        }
        // Buffer is cleared; a subsequent valid frame parses cleanly.
        let frame = encode_frame(KIND_REQUEST, b"x").expect("encode");
        let frames = reader.push(&frame);
        assert_eq!(frames.len(), 1);
        assert!(frames[0].is_ok());
    }

    #[test]
    fn empty_push_returns_nothing() {
        let mut reader = FrameReader::new();
        assert!(reader.push(&[]).is_empty());
    }

    #[test]
    fn partial_frame_yields_nothing_until_complete() {
        let frame = encode_frame(KIND_RESPONSE, b"abc").expect("encode");
        let mut reader = FrameReader::new();
        assert!(reader.push(&frame[..2]).is_empty());
        assert!(reader.push(&frame[2..4]).is_empty());
        let frames = reader.push(&frame[4..]);
        assert_eq!(frames.len(), 1);
        let (kind, body) = frames.into_iter().next().unwrap().expect("frame");
        assert_eq!(kind, KIND_RESPONSE);
        assert_eq!(body, b"abc");
    }

    #[test]
    fn encode_full_size_payload_ok() {
        // 64 MiB at the cap encodes fine (the cap is inclusive).
        let payload = vec![0u8; MAX_FRAME_BYTES as usize];
        let frame = encode_frame(KIND_EVENT, &payload).expect("encode");
        assert_eq!(frame.len() as u64, MAX_FRAME_BYTES as u64 + 5);
    }
}
