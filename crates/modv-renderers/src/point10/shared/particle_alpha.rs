//! Circular particle sprite maps.
//!
//! Port of `src/renderer/src/application/point10/shared/particleAlpha.js` and
//! the texture half of `shared/particleSprite.js`.
//!
//! The JS file's header explains the why: `THREE.Points` draws square
//! fragments unless its material has a map, and an untextured billboard quad
//! is square too. These helpers build one radial alpha map that both paths
//! share, so particles read as round dots.
//!
//! `particleSprite.js` wraps [`build_particle_alpha_map`] in a
//! `THREE.DataTexture` with linear min/mag filters; [`create_particle_texture`]
//! is that wrapper for wgpu (an `Rgba8Unorm` texture with white RGB, so the
//! map multiplies a particle's vertex colour unchanged).

#![forbid(unsafe_code)]

/// Fraction of the sprite radius that stays fully opaque; the alpha ramps down
/// from here to the edge so the dot has a soft outline instead of a hard cut.
///
/// `particleAlpha.js: export const PARTICLE_SPRITE_INNER = 0.55`.
pub const PARTICLE_SPRITE_INNER: f32 = 0.55;

/// Default sprite resolution. `particleSprite.js: createParticleTexture(size = 64)`;
/// `particleAlpha.js: buildParticleAlphaMap(size = 64)`.
pub const PARTICLE_SPRITE_SIZE: u32 = 64;

/// GLSL/JS `smoothstep(edge0, edge1, x)`.
///
/// `particleAlpha.js` local `smoothstep` (lines 12-16): note the JS clamps the
/// *normalised* value to 0..1 before the cubic.
pub fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Alpha for a normalised sprite coordinate.
///
/// `particleAlpha.js: particleAlphaAt(x, y, inner)` (lines 20-29): `(0,0)` is
/// the top-left corner, `(0.5,0.5)` the centre, so the corners fall outside
/// the unit circle.
pub fn particle_alpha_at(x: f32, y: f32, inner: f32) -> f32 {
    let dx = x * 2.0 - 1.0;
    let dy = y * 2.0 - 1.0;
    let radius = (dx * dx + dy * dy).sqrt();
    1.0 - smoothstep(inner, 1.0, radius)
}

/// White RGBA map whose alpha is the circular particle mask.
///
/// `particleAlpha.js: buildParticleAlphaMap(size)` (lines 33-51). Keeping the
/// RGB channels white means the map multiplies a particle's vertex colour
/// unchanged. Row 0 is `y = 0` (the top row), matching the JS loop order and
/// wgpu's `write_texture` row order.
pub fn build_particle_alpha_map(size: u32) -> Vec<u8> {
    let size = size.max(1);
    let mut data = vec![0_u8; (size * size * 4) as usize];
    let denom = (size - 1).max(1) as f32;

    for y in 0..size {
        for x in 0..size {
            let index = ((y * size + x) * 4) as usize;
            let alpha =
                (particle_alpha_at(x as f32 / denom, y as f32 / denom, PARTICLE_SPRITE_INNER)
                    * 255.0)
                    .round() as u8;

            data[index] = 255;
            data[index + 1] = 255;
            data[index + 2] = 255;
            data[index + 3] = alpha;
        }
    }

    data
}

/// Single-channel (`R8`) variant of [`build_particle_alpha_map`].
///
/// The JS only ships the RGBA map; particle scenes that bind one channel save
/// bandwidth. Same coverage bytes as the alpha channel of the RGBA map.
pub fn build_particle_alpha_map_r8(size: u32) -> Vec<u8> {
    let size = size.max(1);
    let mut data = vec![0_u8; (size * size) as usize];
    let denom = (size - 1).max(1) as f32;

    for y in 0..size {
        for x in 0..size {
            let index = (y * size + x) as usize;
            data[index] =
                (particle_alpha_at(x as f32 / denom, y as f32 / denom, PARTICLE_SPRITE_INNER)
                    * 255.0)
                    .round() as u8;
        }
    }

    data
}

/// Create the shared particle sprite texture (white RGB, circular alpha).
///
/// Port of `particleSprite.js: createParticleTexture(size)` (lines 12-25):
/// `RGBAFormat`, linear min/mag filters, one mip level. The JS never uploads
/// mips either (`generateMipmaps` stays false for a `DataTexture`).
pub fn create_particle_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    size: u32,
) -> wgpu::Texture {
    let size = size.max(1);
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("point10-particle-sprite"),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });

    let data = build_particle_alpha_map(size);
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(size * 4),
            rows_per_image: Some(size),
        },
        wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
    );

    texture
}

/// Linear-filtered, clamp-to-edge sampler — `DataTexture`'s defaults.
pub fn particle_sampler(device: &wgpu::Device) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("point10-particle-sprite-sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Nearest,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Port of tests/unit/point10-particle-sprite.test.js, "Point10 particle
    // sprites". The JS test uses SIZE = 32.

    const SIZE: u32 = 32;

    fn alpha_at(data: &[u8], x: u32, y: u32) -> u8 {
        data[((y * SIZE + x) * 4) as usize + 3]
    }

    #[test]
    fn builds_a_deterministic_rgba_map_at_the_requested_size() {
        let map = build_particle_alpha_map(SIZE);
        assert_eq!(map.len(), (SIZE * SIZE * 4) as usize);
        assert_eq!(build_particle_alpha_map(SIZE), map);
        assert_eq!(&map[0..3], &[255, 255, 255]);
    }

    #[test]
    fn opaque_in_the_middle_and_transparent_at_the_corners() {
        let map = build_particle_alpha_map(SIZE);
        let centre = (SIZE - 1) / 2;
        assert_eq!(alpha_at(&map, centre, centre), 255);

        for (x, y) in [(0, 0), (SIZE - 1, 0), (0, SIZE - 1), (SIZE - 1, SIZE - 1)] {
            assert_eq!(alpha_at(&map, x, y), 0, "corner ({x}, {y}) must be clear");
        }
    }

    #[test]
    fn clears_a_circular_edge_instead_of_writing_a_square() {
        let map = build_particle_alpha_map(SIZE);
        let centre = (SIZE - 1) / 2;

        for (x, y) in [
            (0, centre),
            (SIZE - 1, centre),
            (centre, 0),
            (centre, SIZE - 1),
        ] {
            assert_eq!(alpha_at(&map, x, y), 0, "edge ({x}, {y}) must be clear");
        }
    }

    #[test]
    fn fades_from_the_opaque_inner_disc_to_the_edge() {
        let map = build_particle_alpha_map(SIZE);
        let centre = (SIZE - 1) / 2;
        let mut previous = alpha_at(&map, centre, centre);

        for y in centre..SIZE {
            let alpha = alpha_at(&map, centre, y);
            assert!(
                alpha <= previous,
                "alpha must be monotone down the centre column (y={y})"
            );
            previous = alpha;
        }

        assert!(PARTICLE_SPRITE_INNER > 0.0 && PARTICLE_SPRITE_INNER < 1.0);
    }

    #[test]
    fn r8_map_matches_the_rgba_alpha_channel() {
        let rgba = build_particle_alpha_map(SIZE);
        let r8 = build_particle_alpha_map_r8(SIZE);
        assert_eq!(r8.len(), (SIZE * SIZE) as usize);

        for index in 0..r8.len() {
            assert_eq!(r8[index], rgba[index * 4 + 3], "coverage byte {index}");
        }
    }

    #[test]
    fn alpha_map_is_zero_outside_the_unit_circle() {
        // The corner pixel's radius is > 1, so the mask is fully clear; a
        // half-way pixel on the diagonal is inside the soft ramp.
        assert_eq!(particle_alpha_at(0.0, 0.0, PARTICLE_SPRITE_INNER), 0.0);
        assert_eq!(particle_alpha_at(0.5, 0.5, PARTICLE_SPRITE_INNER), 1.0);
        let soft = particle_alpha_at(0.5, 0.05, PARTICLE_SPRITE_INNER);
        assert!(soft > 0.0 && soft < 1.0, "expected a soft edge, got {soft}");
    }
}
