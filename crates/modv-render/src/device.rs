//! wgpu instance/device/queue creation.
//!
//! [`Engine`] owns the instance's only device and queue. A single device is
//! shared by every surface/output window of the process (see the multi-monitor
//! plan in docs/rust-hybrid.md, "Wayland plan").

use std::fmt;

/// Configuration for [`Engine::new`].
#[derive(Clone, Debug)]
pub struct EngineConfig {
    /// Force the fallback (software) adapter instead of the default
    /// high-performance hardware adapter. Useful for CI and headless sandboxes.
    pub force_fallback_adapter: bool,
}

impl Default for EngineConfig {
    /// Defaults to the high-performance hardware adapter.
    fn default() -> Self {
        Self {
            force_fallback_adapter: false,
        }
    }
}

/// Errors produced by engine setup and headless read-back.
///
/// Implemented manually (no `thiserror`) to keep `modv-render` dependency-light.
#[derive(Debug)]
pub enum EngineError {
    /// No adapter matching the request was found.
    NoAdapter,
    /// Requesting the device failed.
    RequestDevice(String),
    /// The surface reported no supported texture formats.
    NoSurfaceFormats,
    /// Creating the surface failed.
    CreateSurface(String),
    /// Mapping a read-back buffer failed.
    Map(String),
    /// Polling the device failed.
    Poll(String),
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoAdapter => write!(f, "no compatible GPU adapter found"),
            Self::RequestDevice(message) => write!(f, "failed to request device: {message}"),
            Self::NoSurfaceFormats => {
                write!(f, "surface reports no supported texture formats")
            }
            Self::CreateSurface(message) => write!(f, "failed to create surface: {message}"),
            Self::Map(message) => write!(f, "failed to map read-back buffer: {message}"),
            Self::Poll(message) => write!(f, "failed to poll device: {message}"),
        }
    }
}

impl std::error::Error for EngineError {}

/// Minimal `std`-only `block_on` for wgpu's setup futures.
///
/// On the native (`wgpu-core`) backends, `request_adapter` and
/// `request_device` resolve synchronously when first polled, so a simple
/// poll loop suffices. The no-op waker never needs to be woken on that
/// path; a pending future would be re-polled after a yield.
fn block_on<F: std::future::Future>(future: F) -> F::Output {
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

/// wgpu device/queue plus the adapter they were created from.
pub struct Engine {
    /// The wgpu device owned by this engine.
    pub device: wgpu::Device,
    /// The wgpu queue owned by this engine.
    pub queue: wgpu::Queue,
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    adapter_info: wgpu::AdapterInfo,
}

impl Engine {
    /// Create the instance, request an adapter and then a device.
    ///
    /// Adapter selection uses [`wgpu::PowerPreference::HighPerformance`] unless
    /// `config.force_fallback_adapter` is set. Compatible surfaces are not
    /// required, so this may be called before any window exists.
    pub fn new(config: &EngineConfig) -> Result<Self, EngineError> {
        let instance = wgpu::Instance::default();
        let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: config.force_fallback_adapter,
            compatible_surface: None,
            apply_limit_buckets: false,
        }))
        .map_err(|_| EngineError::NoAdapter)?;
        let adapter_info = adapter.get_info();
        let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("modv-engine"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            experimental_features: wgpu::ExperimentalFeatures::default(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        }))
        .map_err(|e| EngineError::RequestDevice(e.to_string()))?;
        Ok(Self {
            device,
            queue,
            instance,
            adapter,
            adapter_info,
        })
    }

    /// Information about the adapter the device was requested from.
    pub fn adapter_info(&self) -> &wgpu::AdapterInfo {
        &self.adapter_info
    }

    /// The wgpu instance this engine was created from.
    pub(crate) fn instance(&self) -> &wgpu::Instance {
        &self.instance
    }

    /// The adapter the device was requested from.
    pub(crate) fn adapter(&self) -> &wgpu::Adapter {
        &self.adapter
    }
}
