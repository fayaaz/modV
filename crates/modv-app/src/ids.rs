//! Id generation for engine-created objects.
//!
//! The engine has no `uuid` dependency (it lives in `modv-engine`, not
//! `modv-app`), so ids are generated from `/dev/urandom` with a
//! time + pid fallback. The format matches the UUID v4 strings the JS store
//! writes (`groups.js:151`, `modules.js:289`), so presets and the UI's
//! `getById` lookups are indistinguishable between the two implementations.
//!
//! Ported from the M1 supervisor helper; moved here so both the store and the
//! supervisor use one implementation.

use std::fs;
use std::io::Read;
use std::time::{SystemTime, UNIX_EPOCH};

/// A UUID v4-shaped string.
///
/// Reads 16 bytes from `/dev/urandom` and applies the RFC 4122 version/variant
/// bits. Falls back to process id + nanosecond entropy when `/dev/urandom` is
/// unavailable (only ever seen in stripped-down sandboxes).
pub fn uuid_v4() -> String {
    let mut bytes = [0u8; 16];
    match fs::File::open("/dev/urandom").and_then(|mut file| file.read_exact(&mut bytes)) {
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
            // Fallback: time + pid entropy, hex-only. Unique enough for a
            // process-local id; the store never compares ids across processes.
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            format!(
                "{:08x}-0000-4000-8000-{:012x}",
                std::process::id(),
                nanos & 0xffff_ffff_ffff
            )
        }
    }
}
