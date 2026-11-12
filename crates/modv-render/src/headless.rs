//! Offscreen render targets (no window/surface) and read-back.

use crate::device::{Engine, EngineError};

/// An offscreen RGBA8 render target.
///
/// The texture is created with `RENDER_ATTACHMENT | COPY_SRC` usage at 1x
/// mip/sample count in sRGB ([`wgpu::TextureFormat::Rgba8UnormSrgb`]) format.
pub struct HeadlessTarget {
    /// The offscreen texture.
    pub texture: wgpu::Texture,
    /// A view into `texture`, suitable as a render attachment.
    pub view: wgpu::TextureView,
    /// Texture width in pixels.
    pub width: u32,
    /// Texture height in pixels.
    pub height: u32,
}

/// Create an offscreen render target of `width` x `height` pixels.
///
/// A zero-sized dimension panics in wgpu validation; callers pass a size from
/// the CLI, so [`crate::device::EngineError`] is not needed here.
pub fn create_headless(engine: &Engine, width: u32, height: u32) -> HeadlessTarget {
    let texture = engine.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("modv-headless"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    HeadlessTarget {
        texture,
        view,
        width,
        height,
    }
}

/// Copy the headless target back to the CPU as tightly-packed RGBA8 bytes
/// (`width * height * 4` bytes, row stride `width * 4`).
///
/// `copy_texture_to_buffer` requires `bytes_per_row` to be a multiple of
/// [`wgpu::COPY_BYTES_PER_ROW_ALIGNMENT`] (256), so rows are padded on the
/// GPU side and de-padded here.
pub fn read_back_rgba8(engine: &Engine, target: &HeadlessTarget) -> Result<Vec<u8>, EngineError> {
    let unpadded_bytes_per_row = target.width as u64 * 4;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as u64;
    let padded_bytes_per_row = unpadded_bytes_per_row.div_ceil(align) * align;
    let buffer_size = padded_bytes_per_row * target.height as u64;

    let buffer = engine.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("modv-read-back"),
        size: buffer_size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = engine
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("modv-read-back"),
        });
    encoder.copy_texture_to_buffer(
        target.texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_bytes_per_row as u32),
                rows_per_image: Some(target.height),
            },
        },
        wgpu::Extent3d {
            width: target.width,
            height: target.height,
            depth_or_array_layers: 1,
        },
    );
    engine.queue.submit(Some(encoder.finish()));

    let slice = buffer.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        // The receiver lives on the stack below; a send failure just means
        // we are already unwinding.
        let _ = sender.send(result);
    });
    engine
        .device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|e| EngineError::Poll(e.to_string()))?;
    receiver
        .recv()
        .map_err(|e| EngineError::Map(format!("map callback channel closed: {e}")))?
        .map_err(|e| EngineError::Map(e.to_string()))?;

    let mapped = slice
        .get_mapped_range()
        .map_err(|e| EngineError::Map(e.to_string()))?;
    let mut out = Vec::with_capacity((unpadded_bytes_per_row * target.height as u64) as usize);
    for row in 0..target.height as u64 {
        let start = (row * padded_bytes_per_row) as usize;
        let end = start + unpadded_bytes_per_row as usize;
        out.extend_from_slice(&mapped[start..end]);
    }
    drop(mapped);
    buffer.unmap();
    Ok(out)
}
