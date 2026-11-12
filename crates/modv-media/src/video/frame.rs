//! The RGBA8 frame buffer shared between a GStreamer streaming thread and the
//! engine thread.
//!
//! [`FrameSlot`] holds at most one ready frame and a small pool of recycled
//! buffers. Publishing never waits for the consumer and never lets the buffer
//! count grow: when the consumer has not read the previous frame, the new frame
//! replaces it (`dropped` counts that) and the old buffer goes back into the
//! pool. That is the "newest frame wins" rule from WS-10.
//!
//! Buffer accounting:
//!
//! * the streaming thread takes a buffer from the pool (or allocates one the
//!   first time), fills it and publishes it;
//! * [`FrameSlot::take_into`] swaps the caller's buffer into the pool and the
//!   freshly filled buffer into the caller's [`VideoFrame`], so both sides keep
//!   a buffer of their own and steady-state capture performs **no** allocation;
//! * [`FrameSlot::take`] (the owned-`VideoFrame` convenience) moves the buffer
//!   out of the slot, so the next publish allocates a replacement unless the
//!   pool still has one. Engines should prefer `take_into`.
//!
//! `buffers_allocated` in [`SlotStats`] makes the "no per-frame allocation"
//! guarantee testable.

use std::sync::Mutex;

/// Buffers a slot keeps for reuse. Two is enough for one in flight plus one
/// being filled; a third would only exist while a consumer holds frames.
pub const POOL_CAPACITY: usize = 2;

/// A tightly packed RGBA8 video frame.
///
/// `rgba.len()` is `width * height * 4` (row-major, no padding), which is the
/// layout of a `wgpu::TextureFormat::Rgba8Unorm` upload.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VideoFrame {
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Tightly packed RGBA8 pixels, `width * height * 4` bytes.
    pub rgba: Vec<u8>,
    /// Monotonic per-source frame counter; the first frame is `1`.
    pub sequence: u64,
}

impl VideoFrame {
    /// An empty frame with room for `width × height` RGBA8 pixels.
    pub fn with_size(width: u32, height: u32) -> Self {
        let pixels = width as usize * height as usize * 4;

        Self {
            width,
            height,
            rgba: vec![0; pixels],
            sequence: 0,
        }
    }

    /// The number of pixels the frame claims to hold.
    pub fn pixel_count(&self) -> usize {
        self.width as usize * self.height as usize
    }

    /// Whether the buffer is the size the dimensions imply.
    pub fn is_tightly_packed(&self) -> bool {
        self.rgba.len() == self.pixel_count() * 4
    }

    /// Read one RGBA pixel, or `None` when it is outside the frame.
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }

        let offset = (y as usize * self.width as usize + x as usize) * 4;

        self.rgba
            .get(offset..offset + 4)
            .and_then(|pixel| <[u8; 4]>::try_from(pixel).ok())
    }

    /// Whether the frame contains more than one distinct pixel value.
    ///
    /// A test pattern must not be a flat colour; this is the live-test check
    /// that the pipeline really delivered content rather than a blank buffer.
    pub fn has_varied_pixels(&self) -> bool {
        let (pixels, _) = self.rgba.as_chunks::<4>();
        let Some(first) = pixels.first() else {
            return false;
        };

        pixels.iter().any(|pixel| pixel != first)
    }
}

/// Counters published alongside the frame slot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SlotStats {
    /// Width of the newest published frame.
    pub width: u32,
    /// Height of the newest published frame.
    pub height: u32,
    /// Frames published by the streaming thread.
    pub frames: u64,
    /// Frames replaced before the consumer read them.
    pub dropped: u64,
    /// Sequence of the newest published frame.
    pub sequence: u64,
    /// Buffers the slot had to allocate (stays flat in steady state).
    pub buffers_allocated: u64,
}

struct SlotState {
    ready: Option<VideoFrame>,
    pool: Vec<Vec<u8>>,
    stats: SlotStats,
}

/// The latest-frame slot for one source.
///
/// Lock ordering: this is a leaf lock (nothing is called while holding it), so
/// the streaming thread and the engine thread only contend for the few
/// microseconds a publish or a drain takes.
pub struct FrameSlot {
    state: Mutex<SlotState>,
}

impl Default for FrameSlot {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameSlot {
    /// An empty slot with no buffers allocated yet.
    pub fn new() -> Self {
        Self {
            state: Mutex::new(SlotState {
                ready: None,
                pool: Vec::new(),
                stats: SlotStats::default(),
            }),
        }
    }

    /// Copy one sample into a recycled buffer and publish it.
    ///
    /// `stride` is the source row stride in bytes (GStreamer's
    /// `VideoInfo::stride(0)`); padding bytes past `width * 4` are discarded so
    /// the stored frame is tightly packed. Returns an error message when the
    /// buffer is shorter than the described frame or the stride is negative —
    /// in that case nothing is published and the stats are untouched.
    ///
    /// The slot mutex is taken twice (pop a buffer, publish the frame) and
    /// **not** held while the sample is copied, so a publishing streaming
    /// thread never blocks the engine for longer than a swap.
    pub fn publish(
        &self,
        width: u32,
        height: u32,
        stride: usize,
        rgba: &[u8],
    ) -> Result<(), String> {
        let mut buffer = self.lock().pool.pop().unwrap_or_default();
        let needed = width as usize * height as usize * 4;
        let allocated = buffer.capacity() < needed;

        if let Err(message) = copy_rgba_rows(rgba, stride, width, height, &mut buffer) {
            self.recycle(buffer);

            return Err(message);
        }

        let mut state = self.lock();

        if allocated {
            state.stats.buffers_allocated += 1;
        }

        state.stats.frames += 1;
        state.stats.sequence += 1;
        state.stats.width = width;
        state.stats.height = height;

        let frame = VideoFrame {
            width,
            height,
            rgba: buffer,
            sequence: state.stats.sequence,
        };

        if let Some(previous) = state.ready.replace(frame) {
            state.stats.dropped += 1;

            if state.pool.len() < POOL_CAPACITY && !previous.rgba.is_empty() {
                state.pool.push(previous.rgba);
            }
        }

        Ok(())
    }

    /// Move the newest frame into `frame`, reusing both buffers.
    ///
    /// Returns `false` when no frame is waiting. The caller's previous buffer
    /// goes to the pool, so after the first two frames neither side allocates.
    /// `frame.width`/`height`/`sequence` describe the frame that was copied.
    pub fn take_into(&self, frame: &mut VideoFrame) -> bool {
        let mut state = self.lock();
        let Some(mut ready) = state.ready.take() else {
            return false;
        };

        std::mem::swap(&mut ready.rgba, &mut frame.rgba);

        if state.pool.len() < POOL_CAPACITY && !ready.rgba.is_empty() {
            state.pool.push(ready.rgba);
        }

        frame.width = ready.width;
        frame.height = ready.height;
        frame.sequence = ready.sequence;
        frame
            .rgba
            .truncate(ready.width as usize * ready.height as usize * 4);

        true
    }

    /// Take the newest frame by value.
    ///
    /// Allocates the next buffer unless the pool still holds one; prefer
    /// [`Self::take_into`] on the render path.
    pub fn take(&self) -> Option<VideoFrame> {
        self.lock().ready.take()
    }

    /// Whether a frame is waiting to be taken.
    pub fn has_frame(&self) -> bool {
        self.lock().ready.is_some()
    }

    /// The slot's counters.
    pub fn stats(&self) -> SlotStats {
        self.lock().stats
    }

    /// Drop the waiting frame and the pooled buffers (used by `stop`).
    pub fn clear(&self) {
        let mut state = self.lock();

        state.ready = None;
        state.pool.clear();
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, SlotState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn recycle(&self, buffer: Vec<u8>) {
        if buffer.is_empty() {
            return;
        }

        let mut state = self.lock();

        if state.pool.len() < POOL_CAPACITY {
            state.pool.push(buffer);
        }
    }
}

/// Copy a possibly strided RGBA row range into a tightly packed buffer.
///
/// `out` is cleared and refilled; it only grows when the frame gets bigger, so
/// it is safe to call this every frame from the streaming thread.
pub fn copy_rgba_rows(
    rgba: &[u8],
    stride: usize,
    width: u32,
    height: u32,
    out: &mut Vec<u8>,
) -> Result<(), String> {
    let row_bytes = width as usize * 4;
    let rows = height as usize;

    if row_bytes == 0 || rows == 0 {
        return Err("video frame has no pixels".to_string());
    }

    if stride < row_bytes {
        return Err(format!(
            "video frame stride {stride} is shorter than the {row_bytes}-byte row"
        ));
    }

    let needed = stride
        .checked_mul(rows - 1)
        .and_then(|bytes| bytes.checked_add(row_bytes))
        .ok_or_else(|| "video frame dimensions overflow".to_string())?;

    if rgba.len() < needed {
        return Err(format!(
            "video frame buffer holds {} bytes, needs {needed}",
            rgba.len()
        ));
    }

    let total = row_bytes * rows;

    out.clear();

    if out.capacity() < total {
        out.reserve(total);
    }

    if stride == row_bytes {
        out.extend_from_slice(&rgba[..total]);

        return Ok(());
    }

    for row in 0..rows {
        let start = row * stride;

        out.extend_from_slice(&rgba[start..start + row_bytes]);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame_bytes(width: u32, height: u32, seed: u8) -> Vec<u8> {
        (0..width as usize * height as usize)
            .flat_map(|pixel| {
                let value = (pixel as u8).wrapping_add(seed);

                [value, value.wrapping_mul(2), value.wrapping_mul(3), 255]
            })
            .collect()
    }

    #[test]
    fn video_frame_reports_its_layout_and_pixels() {
        let mut frame = VideoFrame::with_size(2, 2);

        assert!(frame.is_tightly_packed());
        assert_eq!(frame.pixel_count(), 4);

        frame
            .rgba
            .copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]);

        assert_eq!(frame.pixel(0, 0), Some([1, 2, 3, 4]));
        assert_eq!(frame.pixel(1, 1), Some([13, 14, 15, 16]));
        assert_eq!(frame.pixel(2, 0), None);
        assert_eq!(frame.pixel(0, 5), None);
        assert!(frame.has_varied_pixels());
    }

    #[test]
    fn flat_and_empty_frames_have_no_variation() {
        assert!(!VideoFrame::default().has_varied_pixels());
        assert!(!VideoFrame::with_size(4, 4).has_varied_pixels());
    }

    #[test]
    fn copy_rgba_rows_packs_a_strided_frame() {
        // Two 2-pixel rows with 2 bytes of padding each.
        let stride = 2 * 4 + 2;
        let mut rgba = vec![0u8; stride * 2];

        rgba[0..8].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        rgba[10..18].copy_from_slice(&[9, 10, 11, 12, 13, 14, 15, 16]);

        let mut out = Vec::new();

        copy_rgba_rows(&rgba, stride, 2, 2, &mut out).expect("strided copy");

        assert_eq!(
            out,
            vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
            "padding bytes are dropped, rows stay in order"
        );
    }

    #[test]
    fn copy_rgba_rows_passes_packed_frames_through() {
        let rgba = frame_bytes(3, 2, 0);
        let mut out = Vec::new();

        copy_rgba_rows(&rgba, 3 * 4, 3, 2, &mut out).expect("packed copy");

        assert_eq!(out, rgba);
    }

    #[test]
    fn copy_rgba_rows_reuses_the_output_buffer() {
        let rgba = frame_bytes(4, 4, 0);
        let mut out = Vec::new();

        copy_rgba_rows(&rgba, 16, 4, 4, &mut out).expect("first copy");
        let capacity = out.capacity();

        for _ in 0..8 {
            copy_rgba_rows(&rgba, 16, 4, 4, &mut out).expect("repeat copy");
        }

        assert_eq!(out.capacity(), capacity, "steady state must not reallocate");
        assert_eq!(out.len(), 64);
    }

    #[test]
    fn copy_rgba_rows_rejects_short_and_impossible_frames() {
        let mut out = Vec::new();

        assert!(
            copy_rgba_rows(&[0; 15], 16, 4, 4, &mut out).is_err(),
            "short frame"
        );
        assert!(
            copy_rgba_rows(&[0; 64], 8, 4, 4, &mut out).is_err(),
            "negative/too small stride"
        );
        assert!(copy_rgba_rows(&[], 16, 0, 4, &mut out).is_err(), "no width");
        assert!(
            copy_rgba_rows(&[], 16, 4, 0, &mut out).is_err(),
            "no height"
        );
        assert!(out.is_empty(), "a rejected frame publishes nothing");
    }

    #[test]
    fn slot_keeps_only_the_newest_frame_and_counts_drops() {
        let slot = FrameSlot::new();

        for (index, seed) in [1u8, 2, 3].into_iter().enumerate() {
            let rgba = frame_bytes(2, 2, seed);

            slot.publish(2, 2, 8, &rgba).expect("publish");
            assert_eq!(slot.stats().frames, index as u64 + 1);
        }

        let stats = slot.stats();

        assert_eq!(stats.sequence, 3);
        assert_eq!(stats.dropped, 2, "two unread frames were replaced");

        let frame = slot.take().expect("a frame is waiting");

        assert_eq!(frame.sequence, 3, "the newest frame wins");
        assert_eq!(frame.rgba, frame_bytes(2, 2, 3));
        assert!(slot.take().is_none());
    }

    #[test]
    fn slot_take_into_swaps_buffers_without_allocating() {
        let slot = FrameSlot::new();
        let mut frame = VideoFrame::with_size(2, 2);
        let rgba = frame_bytes(2, 2, 7);

        slot.publish(2, 2, 8, &rgba).expect("publish");
        assert!(slot.take_into(&mut frame));
        assert_eq!(frame.sequence, 1);
        assert_eq!(frame.rgba, rgba);

        // The consumer's old buffer went back to the pool, so every following
        // publish/drain pair reuses it instead of allocating.
        for _ in 0..16 {
            slot.publish(2, 2, 8, &rgba).expect("publish");

            assert!(slot.take_into(&mut frame));
        }

        let stats = slot.stats();

        assert_eq!(
            stats.buffers_allocated, 1,
            "one allocation total after warmup"
        );
        assert_eq!(stats.sequence, 17);
        assert_eq!(frame.rgba, rgba);
        assert_eq!(frame.rgba.len(), 16, "frames stay tightly packed");
    }

    #[test]
    fn slot_rejects_a_bad_sample_without_publishing() {
        let slot = FrameSlot::new();

        assert!(slot.publish(4, 4, 16, &[0; 8]).is_err());
        assert!(!slot.has_frame());

        let stats = slot.stats();

        assert_eq!(stats.frames, 0);
        assert_eq!(stats.sequence, 0);
    }

    #[test]
    fn slot_clear_drops_the_waiting_frame() {
        let slot = FrameSlot::new();

        slot.publish(1, 1, 4, &[1, 2, 3, 4]).expect("publish");
        assert!(slot.has_frame());

        slot.clear();

        assert!(!slot.has_frame());
        assert!(slot.take().is_none());
        assert_eq!(slot.stats().frames, 1, "stats survive clear");
    }
}
