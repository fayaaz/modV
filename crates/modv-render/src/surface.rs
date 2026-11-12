//! Surface creation and presentation target for a window.

use std::sync::Arc;

use wgpu::CurrentSurfaceTexture;
use winit::window::Window;

use crate::device::{Engine, EngineError};

/// A configured wgpu surface bound to a winit window.
pub struct SurfaceTarget {
    /// The surface presenting into the window.
    pub surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
}

/// Create and configure a surface for `window`.
///
/// The adapter must be able to present to the surface; the first supported
/// sRGB format (otherwise the first supported format) is selected. Fifo
/// (vsync) presentation is the default, per the Wayland plan in
/// docs/rust-hybrid.md. The initial configuration size is taken from
/// [`Window::inner_size`], clamped to a non-zero size.
pub fn create_surface(engine: &Engine, window: Arc<Window>) -> Result<SurfaceTarget, EngineError> {
    let size = window.inner_size();
    let width = size.width.max(1);
    let height = size.height.max(1);
    let surface = engine
        .instance()
        .create_surface(window)
        .map_err(|e| EngineError::CreateSurface(e.to_string()))?;
    let caps = surface.get_capabilities(engine.adapter());
    if caps.formats.is_empty() {
        return Err(EngineError::NoSurfaceFormats);
    }
    let format = caps
        .formats
        .iter()
        .copied()
        .find(wgpu::TextureFormat::is_srgb)
        .unwrap_or(caps.formats[0]);
    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        color_space: wgpu::SurfaceColorSpace::Auto,
        width,
        height,
        present_mode: wgpu::PresentMode::Fifo,
        desired_maximum_frame_latency: 2,
        alpha_mode: caps.alpha_modes[0],
        view_formats: vec![],
    };
    surface.configure(&engine.device, &config);
    Ok(SurfaceTarget { surface, config })
}

impl SurfaceTarget {
    /// Reconfigure the surface to a new size. Zero-sized dimensions are ignored.
    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(device, &self.config);
    }

    /// The dimensions the surface is currently configured for.
    pub fn dimensions(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    /// The texture format the surface is configured for.
    pub fn format(&self) -> wgpu::TextureFormat {
        self.config.format
    }

    /// Acquire the next surface texture to render into.
    ///
    /// wgpu 30 returns [`CurrentSurfaceTexture`] rather than a `Result`:
    /// callers must handle `Timeout`/`Occluded` (skip the frame) and
    /// `Outdated`/`Lost` (reconfigure or recreate the surface) themselves.
    pub fn current_texture(&self) -> CurrentSurfaceTexture {
        self.surface.get_current_texture()
    }

    /// Reconfigure the surface with its current configuration, e.g. after an
    /// `Outdated`/`Lost`/`Suboptimal` frame acquisition.
    pub fn reconfigure(&mut self, device: &wgpu::Device) {
        self.surface.configure(device, &self.config);
    }
}
