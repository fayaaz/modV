//! Shared report/adapter-info serialisation helpers for `--print-adapter`
//! and `--bench`.

use modv_render::{Engine, FrameReport};
use serde_json::{Value, json};

/// Plain-data snapshot of `wgpu::AdapterInfo` (the wgpu crate is not a
/// dependency of `modv-app`, so the values are copied out eagerly).
#[derive(Clone, Debug)]
pub struct AdapterSummary {
    /// Adapter name reported by the driver.
    pub name: String,
    /// Vendor ID (PCI vendor ID on Vulkan).
    pub vendor: u32,
    /// Device ID.
    pub device: u32,
    /// Device type (`IntegratedGpu`, `DiscreteGpu`, ...), debug-formatted.
    pub device_type: String,
    /// Driver name.
    pub driver: String,
    /// Driver-reported version/info string.
    pub driver_info: String,
    /// wgpu backend (`Vulkan`, `Gl`, ...), debug-formatted.
    pub backend: String,
}

impl AdapterSummary {
    /// Copy the summary out of a live engine's adapter info.
    pub fn from_engine(engine: &Engine) -> Self {
        let info = engine.adapter_info();
        Self {
            name: info.name.clone(),
            vendor: info.vendor,
            device: info.device,
            device_type: format!("{:?}", info.device_type),
            driver: info.driver.clone(),
            driver_info: info.driver_info.clone(),
            backend: format!("{:?}", info.backend),
        }
    }

    /// The adapter object used by `--print-adapter` and the bench report.
    pub fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "vendor": self.vendor,
            "device": self.device,
            "device_type": self.device_type,
            "driver": self.driver,
            "driver_info": self.driver_info,
            "backend": self.backend,
        })
    }
}

/// Build the full JSON report printed by `--bench`.
pub fn report_json(
    report: &FrameReport,
    resolution: (u32, u32),
    audio_fixture_hz: f64,
    adapter: &AdapterSummary,
) -> Value {
    json!({
        "frames": report.frames,
        "wallSeconds": report.wall_seconds,
        "fps": report.frames_per_second,
        "meanMs": report.mean_ms,
        "p50Ms": report.p50_ms,
        "p90Ms": report.p90_ms,
        "p99Ms": report.p99_ms,
        "minMs": report.min_ms,
        "maxMs": report.max_ms,
        "cpuPercent": report.cpu_percent,
        "rssMb": report.rss_mb,
        "adapter": adapter.to_json(),
        "resolution": { "width": resolution.0, "height": resolution.1 },
        "audioFixtureHz": audio_fixture_hz,
    })
}
