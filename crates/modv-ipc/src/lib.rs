//! Protocol, framing and shared-memory preview transport. See docs/rust-hybrid.md.
//!
//! This crate implements the M1 control-channel milestone:
//!
//! - [`frame`] — the `[ u32 LE payload length ][ u8 kind ][ payload ]` wire format.
//! - [`message`] — the JSON `Request` / `Response` / `Event` messages carried by frames.
//! - [`preview`] — the `VMP1` shared-memory ring used for preview frames.
//! - [`socket`] — non-blocking Unix domain socket plumbing for the engine main loop.
//!
//! Spec references are given as `spec:` comments pointing at
//! “docs/rust-hybrid.md — Protocol specification”.
#![forbid(unsafe_code)]

pub mod frame;
pub mod message;
pub mod preview;
pub mod socket;
