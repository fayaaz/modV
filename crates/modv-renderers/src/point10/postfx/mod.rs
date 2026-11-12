//! The shared Point10 post-processing chain.
//!
//! 20 of the 25 vendored scenes build a three.js `EffectComposer`:
//! `RenderPass` → `UnrealBloomPass` → (scene `ShaderPass`) → `OutputPass`.
//! This module ports those passes to WGSL/wgpu once and reuses them.
//!
//! # Chain (and how it maps)
//!
//! ```text
//! scene ──▶ scene_target (HDR, Rgba16Float)
//!              │
//!              ├── RenderPass (blit) ──▶ base_target
//!              │
//!              ├── UnrealBloomPass: bright ─▶ 5 mip blur pairs ─▶ composite (half res)
//!              │
//!              ├── combine (base + bloom) ──▶ hdr_target
//!              │
//!              └── OutputPass (tone map + sRGB) ──▶ destination
//! ```
//!
//! three's `UnrealBloomPass` blends its composite **additively into the read
//! buffer**, i.e. it writes into the same texture the next pass samples. wgpu
//! forbids sampling and rendering to the same texture, so [`Composer`] keeps a
//! separate `base` copy and lets the combine pass add `base + bloom` into a
//! third target. The arithmetic is identical: `dst = src + bloom` with
//! `blendFunc(ONE, ONE)`.
//!
//! # Formats
//!
//! Every intermediate target is [`crate::point10::HDR_FORMAT`]
//! (`Rgba16Float`), matching the `HalfFloatType` render targets three's
//! composer allocates (`UnrealBloomPass.js:90-125`). The final destination is
//! whatever the engine presents; [`Composer`] applies the sRGB transfer in the
//! output shader unless the destination format is itself `*Srgb`, in which
//! case the hardware does it (avoiding a double encode).
//!
//! # Missing passes
//!
//! `AfterimagePass` (LatentBackrooms) and `SSAOPass` (GeometricTunnel) are
//! scene-specific and land with those scenes (batches 4-5).

#![forbid(unsafe_code)]

pub mod composer;
pub mod output_pass;
pub mod render_pass;
pub mod shader_pass;
pub mod unreal_bloom_pass;

pub use composer::Composer;
pub use output_pass::{OutputPass, ToneMapping};
pub use render_pass::RenderPass;
pub use shader_pass::{ShaderPass, ShaderPassUniforms};
pub use unreal_bloom_pass::UnrealBloomPass;

#[cfg(test)]
mod tests;

/// Clamp-to-edge, linear, non-mipmapped sampler used by every pass.
///
/// three's `WebGLRenderTarget` textures default to `ClampToEdgeWrapping` with
/// `LinearFilter` and no mipmaps, so this is the sampler every pass in the JS
/// chain samples with.
pub(crate) fn linear_sampler(device: &wgpu::Device, label: &str) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some(label),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Nearest,
        ..Default::default()
    })
}

/// Create an HDR render target (single mip, single sample).
pub(crate) fn create_target(
    device: &wgpu::Device,
    label: &str,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let width = width.max(1);
    let height = height.max(1);
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: crate::point10::HDR_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

/// `Math.round(width / 2)` for the bloom mip chain
/// (`UnrealBloomPass.js:86-87`, `setSize`, `:246-256`).
pub(crate) fn half(size: u32) -> u32 {
    ((size as f32) / 2.0).round().max(1.0) as u32
}
