//! wgpu device/queue, surface/present, headless targets, frame stats.
pub mod device;
pub mod headless;
pub mod stats;
pub mod surface;
pub use device::{Engine, EngineConfig, EngineError};
pub use stats::{FrameReport, FrameStats};
/// Re-export so dependent crates (e.g. `modv-app`) can name the wgpu types
/// this crate's API exposes, without adding a direct dependency on wgpu.
pub use wgpu;
