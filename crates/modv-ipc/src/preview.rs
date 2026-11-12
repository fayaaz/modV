//! Shared-memory preview transport ("VMP1" ring).
//!
//! spec: docs/rust-hybrid.md — “Protocol specification — Preview channel”:
//!
//! The engine creates a ring of `slots` (default 3) slots in a shared-memory
//! file (`/dev/shm` on Linux; a `memfd` is a later, equivalent optimisation).
//! Each slot is a 32-byte little-endian header followed by `height * stride`
//! bytes of pixels:
//!
//! | Offset | Type  | Field                         |
//! | -----: | ----- | ----------------------------- |
//! |      0 | `u32` | magic `VMP1` (`0x3150_4D56`)  |
//! |      4 | `u32` | width                         |
//! |      8 | `u32` | height                        |
//! |     12 | `u32` | stride (bytes per row)        |
//! |     16 | `u32` | format (1 = RGBA8, 2 = BGRA8) |
//! |     20 | `u32` | sequence number               |
//! |     24 | `u64` | frame id                      |
//!
//! Slots are written round-robin. The sequence number acts as a seqlock: the
//! writer bumps it to an *odd* value before writing, then to an *even* value
//! after. Readers copy the header and pixels, then re-read the header; a
//! mismatch (or an odd sequence, bad magic, or implausible geometry) means the
//! slot was never written or was overwritten mid-read and the frame is dropped
//! — previews are never queued.
//!
//! Ambiguity resolved: the spec does not pin down where the file lives when no
//! hint is given, nor the exact on-disk name. We use `/dev/shm/modv-preview-<pid>-<nanos>`
//! via plain `std::fs` (no `memfd`, `memmap2` or `libc` needed for a file in
//! `tmpfs`), and tests cover the fallback. The file is unlinked on drop.
//!
//! # Robustness
//!
//! - The ring file is created `0600` (previews are fullscreen capture
//!   contents), independent of the umask.
//! - Fresh slots are initialised with an *invalid* magic and an odd sequence,
//!   so an unwritten slot reads back as "no frame" instead of as a black frame
//!   with `frameId: 0`.
//! - A failed write poisons the slot header (no magic, odd sequence) so a
//!   reader that catches the slot mid-failure still rejects it
//!   deterministically, and does not count the frame as published.
//!
//! Note: this module writes pixels with plain `std::fs` calls (`pread`/`pwrite`
//! via `FileExt`, not mmap) into a `tmpfs`-backed file, so there is no
//! page-cache-vs-shm duplication cost worth optimising here. The renderer
//! (Electron) reads the same file.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{FileExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

/// Size of the per-slot header in bytes.
pub const HEADER_BYTES: usize = 32;

/// Offset of the `sequence` field inside a slot header.
const SEQUENCE_OFFSET: u64 = 20;

/// Magic number `VMP1` little-endian (`'V' | 'M' | 'P' | '1'`).
pub const MAGIC: u32 = 0x3150_4D56;

/// Pixel format: 4 × u8, R, G, B, A byte order.
pub const FORMAT_RGBA8: u32 = 1;
/// Pixel format: 4 × u8, B, G, R, A byte order.
pub const FORMAT_BGRA8: u32 = 2;

const BYTES_PER_PIXEL: u32 = 4;

/// Parsed slot header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreviewHeader {
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Bytes per row (>= `width * 4`).
    pub stride: u32,
    /// Pixel format ([`FORMAT_RGBA8`] or [`FORMAT_BGRA8`]).
    pub format: u32,
    /// Sequence number; even between writes, odd while a write is in flight.
    pub sequence: u32,
    /// Monotonic id of the frame written into this slot.
    pub frame_id: u64,
}

impl PreviewHeader {
    fn encode(&self) -> [u8; HEADER_BYTES] {
        let mut buf = [0u8; HEADER_BYTES];
        buf[0..4].copy_from_slice(&MAGIC.to_le_bytes());
        buf[4..8].copy_from_slice(&self.width.to_le_bytes());
        buf[8..12].copy_from_slice(&self.height.to_le_bytes());
        buf[12..16].copy_from_slice(&self.stride.to_le_bytes());
        buf[16..20].copy_from_slice(&self.format.to_le_bytes());
        buf[20..24].copy_from_slice(&self.sequence.to_le_bytes());
        buf[24..32].copy_from_slice(&self.frame_id.to_le_bytes());
        buf
    }

    fn decode(buf: &[u8; HEADER_BYTES]) -> Self {
        let u32_at =
            |offset: usize| u32::from_le_bytes(buf[offset..offset + 4].try_into().expect("slice"));
        Self {
            width: u32_at(4),
            height: u32_at(8),
            stride: u32_at(12),
            format: u32_at(16),
            sequence: u32_at(20),
            frame_id: u64::from_le_bytes(buf[24..32].try_into().expect("slice")),
        }
    }

    /// Whether the header describes a frame the ring can be holding.
    ///
    /// A header read from the file may be stale, torn or corrupted, so the
    /// reader checks the basic geometry (not just the size fit) before trusting
    /// the pixel count it implies.
    fn is_plausible(&self) -> bool {
        self.width > 0
            && self.height > 0
            && self.stride >= self.width.saturating_mul(BYTES_PER_PIXEL)
            && matches!(self.format, FORMAT_RGBA8 | FORMAT_BGRA8)
    }

    /// Number of pixel bytes the header advertises.
    fn pixel_bytes(&self) -> Option<usize> {
        usize::try_from(self.stride)
            .ok()?
            .checked_mul(usize::try_from(self.height).ok()?)
    }
}

/// A frame written into the ring; returned by [`PreviewRing::write_frame`].
///
/// spec: the engine publishes `preview/frame` events whose `data` payload is
/// exactly [`PreviewFrame::to_event`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreviewFrame {
    /// Ring slot index the frame was written into.
    pub slot: u32,
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Bytes per row.
    pub stride: u32,
    /// Pixel format ([`FORMAT_RGBA8`] or [`FORMAT_BGRA8`]).
    pub format: u32,
    /// Sequence number after the write completed (always even).
    pub sequence: u32,
    /// Monotonic frame id.
    pub frame_id: u64,
}

impl PreviewFrame {
    /// Event payload published as `preview/frame`.
    ///
    /// Keys match the spec's `preview/ready` handshake naming style:
    /// `{ slot, width, height, stride, format, sequence, frameId }`.
    pub fn to_event(&self) -> Value {
        json!({
            "slot": self.slot,
            "width": self.width,
            "height": self.height,
            "stride": self.stride,
            "format": self.format,
            "sequence": self.sequence,
            "frameId": self.frame_id,
        })
    }
}

/// The `VMP1` shared-memory ring.
///
/// Created by the engine; the renderer learns the file path through
/// [`PreviewRing::handshake`] (sent as a `preview/ready` event) and maps/reads
/// the same file. Dropping the ring unlinks the file (best effort).
pub struct PreviewRing {
    file: File,
    path: PathBuf,
    slots: u32,
    slot_bytes: usize,
    width: u32,
    height: u32,
    format: u32,
    sequence: u32,
    frame_id: u64,
}

impl PreviewRing {
    /// Create a ring of `slots` slots for `width` × `height` RGBA8 frames.
    ///
    /// `stride` is `width * 4` (tightly packed; formats are 32-bit). When
    /// `path_hint` is `Some`, exactly that path is created (truncating any
    /// existing file); otherwise a unique `/dev/shm/modv-preview-<pid>-<nanos>`
    /// is used. The file is owner-only (`0600`).
    ///
    /// Fails if `slots` or `width`/`height` is 0, or if the geometry overflows.
    pub fn create(
        path_hint: Option<PathBuf>,
        slots: usize,
        width: u32,
        height: u32,
    ) -> io::Result<Self> {
        if slots == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "slots must be >= 1",
            ));
        }
        if width == 0 || height == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "width and height must be >= 1",
            ));
        }
        let slots = u32::try_from(slots).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "slots does not fit into u32")
        })?;
        let stride = width
            .checked_mul(BYTES_PER_PIXEL)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "stride overflows u32"))?;
        let slot_bytes = HEADER_BYTES
            .checked_add(
                (stride as usize)
                    .checked_mul(height as usize)
                    .ok_or_else(|| {
                        io::Error::new(io::ErrorKind::InvalidInput, "slot size overflows usize")
                    })?,
            )
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "slot size overflows"))?;
        let file_len = u64::from(slots) * slot_bytes as u64;

        let path = match path_hint {
            Some(hint) => hint,
            None => unique_shm_path(),
        };
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&path)?;
        // `.mode` only applies to a file this call creates; tighten a path that
        // already existed (previews are fullscreen capture contents and must
        // not be world-readable).
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        file.set_len(file_len)?;

        // Publish initial headers: an invalid magic and an odd sequence, so a
        // slot that has never been written reads back as "no frame" rather than
        // as a black frame with `frameId: 0`. `write_frame` installs the real
        // magic with its first (odd) header.
        let format = FORMAT_RGBA8;
        let base = PreviewHeader {
            width,
            height,
            stride,
            format,
            sequence: 1,
            frame_id: 0,
        };
        let mut unwritten = base.encode();
        unwritten[0..4].copy_from_slice(&0u32.to_le_bytes());
        for slot in 0..slots {
            let offset = u64::from(slot) * slot_bytes as u64;
            write_all_at(&mut file, &unwritten, offset)?;
        }
        file.flush()?;

        Ok(Self {
            file,
            path,
            slots,
            slot_bytes,
            width,
            height,
            format,
            sequence: 0,
            frame_id: 0,
        })
    }

    /// Path of the ring file the engine advertises over the control socket.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// `preview/ready` handshake payload.
    ///
    /// Keys: `{ path, slots, slotBytes, width, height, stride, format }`.
    pub fn handshake(&self) -> Value {
        json!({
            "path": self.path.to_string_lossy(),
            "slots": self.slots,
            "slotBytes": self.slot_bytes as u64,
            "width": self.width,
            "height": self.height,
            "stride": self.stride(),
            "format": self.format,
        })
    }

    /// Write one frame into the next ring slot.
    ///
    /// `pixels.len()` must equal `stride * height`; anything else fails before
    /// touching the file. The header sequence is bumped to an odd value before
    /// the pixels are written and to the following even value afterwards, so a
    /// concurrent reader either sees a consistent frame or [`Ok(None)`] from
    /// [`PreviewRing::read_slot`].
    ///
    /// The sequence for the whole write is reserved up front: an exhausted
    /// sequence fails before anything is written. An I/O failure mid-write
    /// poisons the slot header (see [`PreviewRing::poison_slot`]), consumes the
    /// reserved sequence pair and does not advance `frame_id`, so the frame is
    /// never reported as published.
    pub fn write_frame(&mut self, pixels: &[u8]) -> io::Result<PreviewFrame> {
        let stride = self.stride();
        let expected = stride as usize * self.height as usize;
        if pixels.len() != expected {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "expected {expected} pixel bytes (stride {stride} x height {}), got {}",
                    self.height,
                    pixels.len()
                ),
            ));
        }

        let slot =
            u32::try_from(self.frame_id % u64::from(self.slots)).expect("slot index fits u32");
        let slot_offset = u64::from(slot) * self.slot_bytes as u64;

        // Seqlock: odd while writing, even afterwards. Both numbers are
        // reserved before the first byte is written.
        let seq_odd = self
            .sequence
            .checked_add(1)
            .ok_or_else(|| io::Error::other("sequence number exhausted"))?;
        let seq_even = seq_odd
            .checked_add(1)
            .ok_or_else(|| io::Error::other("sequence number exhausted"))?;
        let frame_id = self.frame_id;

        let header_odd = PreviewHeader {
            width: self.width,
            height: self.height,
            stride,
            format: self.format,
            sequence: seq_odd,
            frame_id,
        };

        let result = write_frame_bytes(&mut self.file, slot_offset, &header_odd, pixels, seq_even);

        if let Err(error) = result {
            // The slot may hold a half-written header (or complete pixels with
            // an odd sequence). Poison it so readers skip it deterministically
            // regardless of which part of the write failed.
            self.poison_slot(slot_offset);
            // Reserved but not published: a retry uses a fresh odd/even pair, so
            // no reader can mistake the failed attempt's sequence for the retry.
            self.sequence = seq_even;

            return Err(error);
        }

        self.sequence = seq_even;
        self.frame_id += 1;

        Ok(PreviewFrame {
            slot,
            width: self.width,
            height: self.height,
            stride,
            format: self.format,
            sequence: seq_even,
            frame_id,
        })
    }

    /// Invalidate a slot header: no magic, odd sequence.
    ///
    /// Used when a write fails part-way through. A reader rejects the slot on
    /// the magic check before trusting any geometry, so a torn header can never
    /// be handed out as a frame. Best effort — if this write fails too, the odd
    /// sequence from the failed attempt still makes readers skip it.
    fn poison_slot(&mut self, slot_offset: u64) {
        let header = PreviewHeader {
            width: self.width,
            height: self.height,
            stride: self.stride(),
            format: self.format,
            sequence: 1,
            frame_id: 0,
        };
        let mut buf = header.encode();
        buf[0..4].copy_from_slice(&0u32.to_le_bytes());
        let _ = write_all_at(&mut self.file, &buf, slot_offset);
    }

    /// Read one slot with the seqlock double-read protocol.
    ///
    /// This is a *static* helper: it takes the ring file path and re-opens it,
    /// so tests (and conceptually the Electron side) can use it without an
    /// open [`PreviewRing`].
    ///
    /// Returns:
    /// - `Ok(None)` when the slot holds no valid frame — bad magic, an odd
    ///   sequence (a write is in flight or the slot was poisoned), implausible
    ///   geometry, or a header that changed between the two reads (the slot was
    ///   overwritten mid-read).
    /// - `Err(InvalidInput)` when `slot` is outside the ring or `slot_bytes` is
    ///   smaller than a header (an unchecked `slot * slot_bytes` used to read
    ///   past the file).
    /// - `Err(InvalidData)` when the slot geometry cannot hold the pixels the
    ///   header advertises.
    /// - `Ok(Some((header, pixels)))` with the header and pixel copy otherwise.
    pub fn read_slot(
        path: &Path,
        slot_bytes: usize,
        slot: u32,
    ) -> io::Result<Option<(PreviewHeader, Vec<u8>)>> {
        if slot_bytes < HEADER_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "slot_bytes smaller than the 32-byte header",
            ));
        }
        let mut file = File::open(path)?;
        let file_len = file.metadata()?.len();
        let offset = u64::from(slot)
            .checked_mul(slot_bytes as u64)
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "slot offset overflows u64")
            })?;

        // Bound the slot against the ring: never pread past the end of the
        // file, which would ship uninitialised memory to a caller.
        let in_range = offset
            .checked_add(HEADER_BYTES as u64)
            .is_some_and(|end| end <= file_len);
        if !in_range {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("slot {slot} is outside the {file_len}-byte ring"),
            ));
        }

        let mut header_buf = [0u8; HEADER_BYTES];
        read_exact_at(&mut file, &mut header_buf, offset)?;
        let magic = u32::from_le_bytes(header_buf[..4].try_into().expect("slice"));
        if magic != MAGIC {
            return Ok(None);
        }
        let header = PreviewHeader::decode(&header_buf);
        if !header.sequence.is_multiple_of(2) {
            // Write in flight (or poisoned).
            return Ok(None);
        }
        if !header.is_plausible() {
            return Ok(None);
        }

        let pixels_len = header.pixel_bytes().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "stride x height overflows")
        })?;
        if HEADER_BYTES + pixels_len > slot_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "header pixel count exceeds slot_bytes",
            ));
        }

        let mut pixels = vec![0u8; pixels_len];
        read_exact_at(&mut file, &mut pixels, offset + HEADER_BYTES as u64)?;

        // Seqlock second read: the whole header must be unchanged, not just the
        // sequence — a torn header with the same sequence must not be accepted.
        let mut after = [0u8; HEADER_BYTES];
        read_exact_at(&mut file, &mut after, offset)?;
        if after != header_buf {
            return Ok(None);
        }

        Ok(Some((header, pixels)))
    }

    fn stride(&self) -> u32 {
        // Formats are 4 bytes per pixel; stride is tightly packed.
        self.width * BYTES_PER_PIXEL
    }
}

impl Drop for PreviewRing {
    fn drop(&mut self) {
        // Best effort: a stale preview file must not accumulate in /dev/shm.
        let _ = std::fs::remove_file(&self.path);
    }
}

fn unique_shm_path() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    PathBuf::from(format!(
        "/dev/shm/modv-preview-{}-{nanos}",
        std::process::id()
    ))
}

fn write_all_at(file: &mut File, buf: &[u8], offset: u64) -> io::Result<()> {
    // pread/pwrite-style: no shared file cursor, safe to interleave with reads.
    file.write_all_at(buf, offset)
}

/// The three writes of one frame: the odd header (the seqlock bump and the new
/// geometry/frame id), the pixels, then the even sequence that publishes it.
///
/// The publish step rewrites only the 4-byte sequence field. Rewriting the whole
/// header there would leave a torn header plausible: a partially written
/// geometry or `frame_id` with an already-even sequence could be accepted with
/// pixels from the previous frame. A torn 4-byte sequence is either odd or
/// different from the reader's first read, so the frame is dropped.
fn write_frame_bytes(
    file: &mut File,
    slot_offset: u64,
    header_odd: &PreviewHeader,
    pixels: &[u8],
    seq_even: u32,
) -> io::Result<()> {
    write_all_at(file, &header_odd.encode(), slot_offset)?;
    write_all_at(file, pixels, slot_offset + HEADER_BYTES as u64)?;
    write_all_at(file, &seq_even.to_le_bytes(), slot_offset + SEQUENCE_OFFSET)
}

fn read_exact_at(file: &mut File, buf: &mut [u8], offset: u64) -> io::Result<()> {
    file.read_exact_at(buf, offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel_pattern(len: usize, seed: u8) -> Vec<u8> {
        (0..len).map(|i| seed.wrapping_add(i as u8)).collect()
    }

    #[test]
    fn create_write_and_read_roundtrip() {
        let slot_bytes = HEADER_BYTES + 64 * 4 * 48;
        let mut ring = PreviewRing::create(None, 3, 64, 48).expect("create ring");
        let path = ring.path().to_path_buf();
        assert!(path.starts_with("/dev/shm"));
        assert!(path.exists());

        let first = pixel_pattern(64 * 4 * 48, 0x10);
        let second = pixel_pattern(64 * 4 * 48, 0x80);

        let frame_a = ring.write_frame(&first).expect("write a");
        assert_eq!(frame_a.slot, 0);
        assert_eq!(frame_a.sequence % 2, 0);
        assert_eq!(frame_a.frame_id, 0);

        let frame_b = ring.write_frame(&second).expect("write b");
        assert_eq!(frame_b.slot, 1);
        assert_eq!(frame_b.sequence, frame_a.sequence + 2);
        assert_eq!(frame_b.frame_id, 1);

        let (header_a, pixels_a) = PreviewRing::read_slot(&path, slot_bytes, 0)
            .expect("read a")
            .expect("slot a holds a frame");
        assert_eq!(header_a.width, 64);
        assert_eq!(header_a.height, 48);
        assert_eq!(header_a.stride, 64 * 4);
        assert_eq!(header_a.format, FORMAT_RGBA8);
        assert_eq!(header_a.sequence, 2);
        assert_eq!(header_a.frame_id, 0);
        assert_eq!(pixels_a, first);

        let (header_b, pixels_b) = PreviewRing::read_slot(&path, slot_bytes, 1)
            .expect("read b")
            .expect("slot b holds a frame");
        assert_eq!(header_b.frame_id, 1);
        assert_eq!(pixels_b, second);
    }

    #[test]
    fn create_restricts_the_ring_file_to_the_owner() {
        // Preview frames are fullscreen capture contents; nobody but the owner
        // may read them (R-21a).
        let ring = PreviewRing::create(None, 3, 8, 8).expect("create ring");
        let mode = std::fs::metadata(ring.path())
            .expect("ring metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn unwritten_slots_hold_no_frame() {
        // A fresh ring must not read back as a black frame (R-21b): the initial
        // headers carry an invalid magic *and* an odd sequence.
        let ring = PreviewRing::create(None, 3, 8, 8).expect("create ring");
        let slot_bytes = HEADER_BYTES + 8 * 4 * 8;

        for slot in 0..3 {
            assert!(
                PreviewRing::read_slot(ring.path(), slot_bytes, slot)
                    .expect("read unwritten slot")
                    .is_none(),
                "slot {slot} must read as unwritten"
            );
        }
    }

    #[test]
    fn read_slot_rejects_an_out_of_range_slot() {
        let mut ring = PreviewRing::create(None, 3, 8, 8).expect("create ring");
        let slot_bytes = HEADER_BYTES + 8 * 4 * 8;
        ring.write_frame(&vec![1u8; 8 * 4 * 8]).expect("write");

        for slot in [3u32, 4, 1_000, u32::MAX] {
            let error = PreviewRing::read_slot(ring.path(), slot_bytes, slot)
                .expect_err("out-of-range slot must be rejected");
            assert_eq!(
                error.kind(),
                io::ErrorKind::InvalidInput,
                "slot {slot} must be out of range"
            );
        }

        // The bounds are exact: the last slot is readable.
        let last = PreviewRing::read_slot(ring.path(), slot_bytes, 2).expect("read last slot");
        assert!(last.is_none(), "slot 2 has not been written yet");
    }

    #[test]
    fn read_slot_rejects_an_invalid_header_geometry() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "modv-ipc-test-geometry-{}-{}",
            std::process::id(),
            unique_nanos()
        ));
        let slot_bytes = HEADER_BYTES + 16;
        // `stride` smaller than `width * 4` cannot describe the frame.
        let header = PreviewHeader {
            width: 8,
            height: 1,
            stride: 8,
            format: FORMAT_RGBA8,
            sequence: 2,
            frame_id: 1,
        };
        let mut bytes = header.encode().to_vec();
        bytes.extend_from_slice(&[0u8; 16]);
        std::fs::write(&path, &bytes).expect("write slot file");

        assert!(matches!(
            PreviewRing::read_slot(&path, slot_bytes, 0),
            Ok(None)
        ));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_failed_write_reports_an_error_and_publishes_no_frame() {
        let mut ring = PreviewRing::create(None, 3, 8, 8).expect("create ring");
        let path = ring.path().to_path_buf();
        let slot_bytes = HEADER_BYTES + 8 * 4 * 8;
        let pixels = vec![5u8; 8 * 4 * 8];

        ring.write_frame(&pixels).expect("write slot 0");

        // A read-only descriptor makes every `pwrite` fail; the frame must not
        // be counted as published, and its reserved sequence pair is not
        // reused by the retry.
        ring.file = File::open(&path).expect("reopen read-only");
        let sequence_before = ring.sequence;
        let frame_id_before = ring.frame_id;
        assert!(ring.write_frame(&pixels).is_err());
        assert_eq!(ring.sequence, sequence_before + 2);
        assert_eq!(ring.frame_id, frame_id_before);
        assert!(
            PreviewRing::read_slot(&path, slot_bytes, 1)
                .expect("read")
                .is_none(),
            "a failed write must not leave a readable frame"
        );

        // The ring keeps working once the descriptor is writable again.
        ring.file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .expect("reopen read-write");
        ring.write_frame(&pixels).expect("write after failure");
        assert!(
            PreviewRing::read_slot(&path, slot_bytes, 1)
                .expect("read")
                .is_some()
        );
    }

    #[test]
    fn a_poisoned_slot_reads_as_empty_and_recovers() {
        let mut ring = PreviewRing::create(None, 3, 8, 8).expect("create ring");
        let path = ring.path().to_path_buf();
        let slot_bytes = HEADER_BYTES + 8 * 4 * 8;
        let pixels = vec![7u8; 8 * 4 * 8];

        ring.write_frame(&pixels).expect("write slot 0");
        assert!(
            PreviewRing::read_slot(&path, slot_bytes, 0)
                .expect("read")
                .is_some()
        );

        // A torn header must be detectable even if the sequence happened to be
        // left even: the poison header has no magic.
        ring.poison_slot(0);
        assert!(
            PreviewRing::read_slot(&path, slot_bytes, 0)
                .expect("read poisoned")
                .is_none()
        );

        // Round-robin: two more frames reuse slot 0 and republish it.
        ring.write_frame(&pixels).expect("write slot 1");
        ring.write_frame(&pixels).expect("write slot 2");
        let frame = ring.write_frame(&pixels).expect("write slot 0 again");
        assert_eq!(frame.slot, 0);
        assert!(
            PreviewRing::read_slot(&path, slot_bytes, 0)
                .expect("read")
                .is_some()
        );
    }

    #[test]
    fn oversize_pixels_are_rejected() {
        let mut ring = PreviewRing::create(None, 3, 64, 48).expect("create ring");
        let too_big = vec![0u8; 64 * 4 * 48 + 1];
        let err = ring.write_frame(&too_big).expect_err("must reject");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        let too_small = vec![0u8; 64 * 4 * 48 - 1];
        assert!(ring.write_frame(&too_small).is_err());
    }

    #[test]
    fn file_is_deleted_on_drop() {
        let path = {
            let ring = PreviewRing::create(None, 3, 32, 32).expect("create ring");
            let path = ring.path().to_path_buf();
            assert!(path.exists());
            path
        };
        assert!(!path.exists(), "ring file must be unlinked on drop");
    }

    #[test]
    fn handshake_json_has_expected_keys() {
        let ring = PreviewRing::create(None, 3, 64, 48).expect("create ring");
        let handshake = ring.handshake();
        assert_eq!(
            handshake["path"],
            json!(ring.path().to_string_lossy().into_owned())
        );
        assert_eq!(handshake["slots"], 3);
        assert_eq!(handshake["slotBytes"], (HEADER_BYTES + 64 * 4 * 48) as u64);
        assert_eq!(handshake["width"], 64);
        assert_eq!(handshake["height"], 48);
        assert_eq!(handshake["stride"], 64 * 4);
        assert_eq!(handshake["format"], FORMAT_RGBA8);
    }

    #[test]
    fn sequence_is_even_after_write_and_increments_by_two() {
        let mut ring = PreviewRing::create(None, 3, 8, 8).expect("create ring");
        let pixels = vec![0u8; 8 * 4 * 8];
        let mut last = 0;
        for expected_frame in 0..6u64 {
            let frame = ring.write_frame(&pixels).expect("write");
            assert_eq!(frame.sequence % 2, 0);
            if expected_frame > 0 {
                assert_eq!(frame.sequence, last + 2);
            }
            last = frame.sequence;
        }
        assert_eq!(last, 12); // 6 frames * 2
    }

    #[test]
    fn slot_advances_round_robin() {
        let mut ring = PreviewRing::create(None, 3, 8, 8).expect("create ring");
        let pixels = vec![7u8; 8 * 4 * 8];
        let slots: Vec<u32> = (0..7)
            .map(|_| ring.write_frame(&pixels).expect("write").slot)
            .collect();
        assert_eq!(slots, vec![0, 1, 2, 0, 1, 2, 0]);
    }

    #[test]
    fn read_slot_rejects_bad_magic() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "modv-ipc-test-badmagic-{}-{}",
            std::process::id(),
            unique_nanos()
        ));
        let slot_bytes = HEADER_BYTES + 16;
        let mut junk = vec![0u8; slot_bytes];
        junk[0..4].copy_from_slice(&0xDEAD_BEEFu32.to_le_bytes());
        std::fs::write(&path, &junk).expect("write junk file");

        let result = PreviewRing::read_slot(&path, slot_bytes, 0);
        assert!(
            matches!(result, Ok(None)),
            "bad magic must yield Ok(None), got {result:?}"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn read_slot_rejects_odd_sequence() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "modv-ipc-test-oddseq-{}-{}",
            std::process::id(),
            unique_nanos()
        ));
        let slot_bytes = HEADER_BYTES + 16;
        let header = PreviewHeader {
            width: 1,
            height: 1,
            stride: 4,
            format: FORMAT_RGBA8,
            sequence: 3, // odd: a write is in flight
            frame_id: 9,
        };
        let mut bytes = header.encode().to_vec();
        bytes.extend_from_slice(&[0u8; 16]);
        std::fs::write(&path, &bytes).expect("write slot file");

        let result = PreviewRing::read_slot(&path, slot_bytes, 0);
        assert!(
            matches!(result, Ok(None)),
            "odd sequence must yield Ok(None), got {result:?}"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn frame_event_payload_shape() {
        let frame = PreviewFrame {
            slot: 2,
            width: 64,
            height: 48,
            stride: 256,
            format: FORMAT_RGBA8,
            sequence: 8,
            frame_id: 3,
        };
        let event = frame.to_event();
        assert_eq!(event["slot"], 2);
        assert_eq!(event["width"], 64);
        assert_eq!(event["height"], 48);
        assert_eq!(event["stride"], 256);
        assert_eq!(event["format"], FORMAT_RGBA8);
        assert_eq!(event["sequence"], 8);
        assert_eq!(event["frameId"], 3);
    }

    fn unique_nanos() -> u128 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    }
}
