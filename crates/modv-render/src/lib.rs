//! wgpu device/queue, surface/present, headless targets, frame stats,
//! blend modes, texture pool and the frame-graph compositor (M2).
#![forbid(unsafe_code)]

pub mod blend;
pub mod device;
pub mod frame_graph;
#[cfg(test)]
mod graph_tests;
pub mod headless;
pub mod stats;
pub mod surface;
pub mod texture_pool;

pub use blend::{BLEND_WGSL, BlendMode, UnknownBlendMode};
pub use device::{Engine, EngineConfig, EngineError};
pub use frame_graph::{
    Executor, FrameGraph, GraphError, MAIN_TEXTURE_ID, PassNode, TextureId, TextureSurfaces,
};
pub use stats::{FrameReport, FrameStats};
pub use texture_pool::{PooledTexture, TexturePool, TexturePoolKey, UsageClass};
/// Re-export so dependent crates (e.g. `modv-app`) can name the wgpu types
/// this crate's API exposes, without adding a direct dependency on wgpu.
pub use wgpu;
