//! Headless test harness for the Point10 renderer.
//!
//! Same pattern as `crates/modv-render/src/graph_tests.rs` and
//! `crates/modv-renderers/src/two_d/tests.rs`: request the **fallback**
//! (software/lavapipe) adapter, and skip every GPU test with a log line when
//! there is none, so a GPU-less CI machine still compiles and runs the pure
//! unit tests.
//!
//! `#[cfg(test)]` only — nothing here ships.

#![cfg(test)]
#![forbid(unsafe_code)]

/// Minimal `std`-only `block_on`, same approach as `modv-render/src/device.rs`.
pub(crate) fn block_on<F: std::future::Future>(future: F) -> F::Output {
    use std::task::{Context, Poll, Wake};

    struct NoopWaker;
    impl Wake for NoopWaker {
        fn wake(self: std::sync::Arc<Self>) {}
    }

    let waker = std::sync::Arc::new(NoopWaker).into();
    let mut cx = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(output) => return output,
            Poll::Pending => std::thread::yield_now(),
        }
    }
}

/// A software-adapter device, or `None` (with a log line) when unavailable.
pub(crate) fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::default();
    let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::LowPower,
        force_fallback_adapter: true,
        compatible_surface: None,
        apply_limit_buckets: false,
    }))
    .map_err(|error| eprintln!("skipping point10 GPU test: no adapter ({error})"))
    .ok()?;

    block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("modv-point10-test"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        experimental_features: wgpu::ExperimentalFeatures::default(),
        memory_hints: wgpu::MemoryHints::Performance,
        trace: wgpu::Trace::Off,
    }))
    .map_err(|error| eprintln!("skipping point10 GPU test: no device ({error})"))
    .ok()
}

/// An 8-bit UNORM render target plus its view, for read-back.
pub(crate) struct TestTarget {
    pub(crate) texture: wgpu::Texture,
    pub(crate) view: wgpu::TextureView,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

impl TestTarget {
    /// Create an `Rgba8Unorm` target (the composer applies the sRGB transfer
    /// itself for non-sRGB formats).
    pub(crate) fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("point10-test-target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            texture,
            view,
            width,
            height,
        }
    }

    /// Read the target back as tightly packed RGBA8 (`width * height * 4`).
    pub(crate) fn read(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Vec<u8> {
        let unpadded = self.width as u64 * 4;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as u64;
        let padded = unpadded.div_ceil(align) * align;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-test-readback"),
            size: padded * self.height as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("point10-test-readback"),
        });
        encoder.copy_texture_to_buffer(
            self.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded as u32),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(Some(encoder.finish()));

        let slice = buffer.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("poll must succeed");
        receiver
            .recv()
            .expect("map callback must run")
            .expect("mapping must succeed");

        let mapped = slice.get_mapped_range().expect("buffer must be mapped");
        let mut out = Vec::with_capacity((unpadded * self.height as u64) as usize);
        for row in 0..self.height as u64 {
            let start = (row * padded) as usize;
            let end = start + unpadded as usize;
            out.extend_from_slice(&mapped[start..end]);
        }
        drop(mapped);
        buffer.unmap();
        out
    }

    /// The RGBA pixel at `(x, y)`.
    pub(crate) fn pixel(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        x: u32,
        y: u32,
    ) -> [u8; 4] {
        let pixels = self.read(device, queue);
        let index = ((y * self.width + x) * 4) as usize;
        [
            pixels[index],
            pixels[index + 1],
            pixels[index + 2],
            pixels[index + 3],
        ]
    }
}

/// True when any pixel has non-zero alpha — a Point10 scene's "something was
/// drawn" check (scenes clear to transparent black).
pub(crate) fn any_visible(pixels: &[u8]) -> bool {
    pixels.chunks_exact(4).any(|pixel| pixel[3] > 0)
}

/// True when any pixel carries visible RGB and alpha.
pub(crate) fn any_lit(pixels: &[u8]) -> bool {
    pixels
        .chunks_exact(4)
        .any(|pixel| pixel[3] > 0 && (pixel[0] as u32 + pixel[1] as u32 + pixel[2] as u32) > 0)
}
