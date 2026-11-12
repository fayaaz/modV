//! The Point10 water height-field simulation.
//!
//! Port of `src/renderer/src/application/point10/shared/waterWave.js`, which is
//! itself a vendored adaptation of jeantimex/threejs-water (MIT), a WebGL port
//! of Evan Wallace's WebGL Water. The JS runs the 2D wave equation on the GPU,
//! ping-ponging two `HalfFloatType` render targets: height in R, vertical
//! velocity in G.
//!
//! Each propagation step (`WAVE_UPDATE_FRAG`):
//!
//! ```text
//! ave = (h(x+1) + h(x-1) + h(y+1) + h(y-1)) * 0.25
//! g += (ave - r) * 2.0
//! g *= 0.995
//! r += g
//! ```
//!
//! and each drop (`WAVE_DROP_FRAG`) adds a cosine bump:
//!
//! ```text
//! drop = max(0.0, 1.0 - length(center - coord) / radius)
//! r += (0.5 - cos(drop * PI) * 0.5) * strength
//! ```
//!
//! # Why CPU, not a wgpu ping-pong
//!
//! The JS sim is a pure function of the previous field, one texel per texel:
//! sampling at `coord ± 1/res` from a texel centre lands exactly on a
//! neighbouring centre, so `LinearFilter` returns that texel's value
//! unchanged. [`WaveSim`] therefore reproduces the pass on the CPU as a plain
//! buffer update, and the scene uploads the resulting field into an
//! `Rgba16Float` texture once per frame. That also lets the field be
//! unit-tested (and pinned against a Node run of the JS) without a GPU.
//!
//! # Boundary conditions
//!
//! The JS render targets use `wrapS: RepeatWrapping` and
//! `wrapT: ClampToEdgeWrapping`: the u axis is periodic (waves cross the
//! caller's angular seam) and the v axis reflects (clamp repeats the edge
//! texel, which acts as a wall). [`WaveSim::step`] reproduces that exactly.
//!
//! # Texture layout
//!
//! `state` is interleaved `[height, velocity, …]` with **row 0 = v 0**, which
//! is wgpu's `write_texture` row order. The JS sim stores the field in a GL
//! texture where v = 0 is the bottom row, and scene shaders using it flip their
//! fragment y to GL's `gl_FragCoord` convention; the Rust scenes do the same,
//! so drop coordinates keep their JS meaning.
//!
//! # Deviations
//!
//! * The JS field is half-float: every pass quantises height and velocity to
//!   f16. The CPU simulation keeps f32 and quantises to f16 on upload, so the
//!   field the shader samples matches the JS to half precision while the
//!   propagating waves are not rounded between steps. The visible difference is
//!   far below the sampling noise of the procedural pattern.
//! * `drop()` with a non-positive radius is a no-op here. In the JS, `radius =
//!   0` evaluates `0/0` and `Math.max(0, NaN)` is `NaN`, which would poison the
//!   whole field; no scene passes a radius that small.

#![forbid(unsafe_code)]

/// Sim resolution the JS scenes use (`WAVE_RES = 256` in WaterRipple and
/// AroundPlus).
pub const WAVE_RES: u32 = 256;

/// Propagation damping per step (`info.g *= 0.995`).
pub const UPDATE_DAMPING: f32 = 0.995;

/// Default drop radius (`dropMat.uniforms.radius.value = 0.04`).
pub const DEFAULT_DROP_RADIUS: f32 = 0.04;

/// The water height field: height in `state[2 * i]`, velocity in
/// `state[2 * i + 1]`, row-major with row 0 = v 0.
#[derive(Clone, Debug, PartialEq)]
pub struct WaveSim {
    res: u32,
    state: Vec<f32>,
}

impl WaveSim {
    /// An all-zero field at `res` × `res` (the JS clears both render targets).
    pub fn new(res: u32) -> Self {
        let res = res.max(1);
        Self {
            res,
            state: vec![0.0; (res * res * 2) as usize],
        }
    }

    /// Grid resolution (texels per side).
    pub fn res(&self) -> u32 {
        self.res
    }

    /// The raw interleaved `[height, velocity]` field, row-major with row 0 =
    /// v 0.
    pub fn data(&self) -> &[f32] {
        &self.state
    }

    /// Height at texel `(x, y)`.
    pub fn height(&self, x: u32, y: u32) -> f32 {
        let x = x.min(self.res - 1);
        let y = y.min(self.res - 1);
        self.state[((y * self.res + x) * 2) as usize]
    }

    /// Velocity at texel `(x, y)`.
    pub fn velocity(&self, x: u32, y: u32) -> f32 {
        let x = x.min(self.res - 1);
        let y = y.min(self.res - 1);
        self.state[((y * self.res + x) * 2 + 1) as usize]
    }

    /// Reset the field to rest.
    pub fn clear(&mut self) {
        self.state.fill(0.0);
    }

    /// Add a cosine bump at `(u, v)` — `WaveSim.drop(u, v, strength, radius)`
    /// (`waterWave.js:110-116`).
    ///
    /// `u`/`v` are in the unit square and land at texel centres
    /// (`(i + 0.5) / res`), exactly like the JS fragment's interpolated
    /// `coord`. The velocity channel is copied through unchanged, matching the
    /// drop pass (`info.g` is written back untouched).
    pub fn drop(&mut self, u: f32, v: f32, strength: f32, radius: f32) {
        if !radius.is_finite() || radius <= 0.0 || !strength.is_finite() {
            return;
        }
        let res = self.res;
        let inv_res = 1.0 / res as f32;

        for y in 0..res {
            let cy = (y as f32 + 0.5) * inv_res;
            for x in 0..res {
                let cx = (x as f32 + 0.5) * inv_res;
                let distance = ((cx - u) * (cx - u) + (cy - v) * (cy - v)).sqrt();
                // `float drop = max(0.0, 1.0 - length(center - coord) / radius);`
                let drop = (1.0 - distance / radius).max(0.0);
                if drop <= 0.0 {
                    continue;
                }
                let bump = (0.5 - (drop * std::f32::consts::PI).cos() * 0.5) * strength;
                self.state[((y * res + x) * 2) as usize] += bump;
            }
        }
    }

    /// One propagation step — `WaveSim.step()`.
    ///
    /// The JS calls this once per frame and passes no time; `dt` is accepted so
    /// a scene can hand it the frame delta without special-casing, and is
    /// otherwise ignored (a fixed discrete step is what the shader is).
    pub fn update(&mut self, _dt: f32) {
        self.step();
    }

    /// One propagation step, reading the whole field before writing (a Jacobi
    /// update, exactly like the single render pass).
    pub fn step(&mut self) {
        let res = self.res;
        if res < 1 {
            return;
        }
        let source = &self.state;
        let mut next = vec![0.0_f32; source.len()];

        // `wrapS: RepeatWrapping` on the u axis.
        let wrap_x = |x: i64| (x.rem_euclid(res as i64)) as usize;
        // `wrapT: ClampToEdgeWrapping` on the v axis.
        let clamp_y = |y: i64| y.clamp(0, res as i64 - 1) as usize;

        let height = |x: i64, y: i64| source[(clamp_y(y) * res as usize + wrap_x(x)) * 2];

        for y in 0..res as i64 {
            for x in 0..res as i64 {
                let index = ((y as u32 * res + x as u32) * 2) as usize;
                let ave =
                    (height(x + 1, y) + height(x - 1, y) + height(x, y + 1) + height(x, y - 1))
                        * 0.25;

                let mut velocity = source[index + 1];
                let mut value = source[index];
                velocity += (ave - value) * 2.0;
                velocity *= UPDATE_DAMPING;
                value += velocity;

                next[index] = value;
                next[index + 1] = velocity;
            }
        }

        self.state = next;
    }

    /// The field packed as `Rgba16Float` texels: R = height, G = velocity,
    /// B = 0, A = 1.
    ///
    /// The JS render target is `HalfFloatType` with four channels; the shaders
    /// only read `.r`. Alpha is written as 1 so a debug view of the field is
    /// visible (it is never sampled).
    pub fn pack_rgba16f(&self) -> Vec<u8> {
        let texels = (self.res * self.res) as usize;
        let mut bytes = Vec::with_capacity(texels * 8);
        for texel in 0..texels {
            let height = f32_to_f16_bits(self.state[texel * 2]);
            let velocity = f32_to_f16_bits(self.state[texel * 2 + 1]);
            bytes.extend_from_slice(&height.to_le_bytes());
            bytes.extend_from_slice(&velocity.to_le_bytes());
            bytes.extend_from_slice(&0_u16.to_le_bytes());
            bytes.extend_from_slice(&f16_one().to_le_bytes());
        }
        bytes
    }

    /// Upload the field into `texture` (an [`Rgba16Float`](wgpu::TextureFormat::Rgba16Float)
    /// texture created by [`create_wave_texture`]).
    pub fn upload(&self, queue: &wgpu::Queue, texture: &wgpu::Texture) {
        let bytes = self.pack_rgba16f();
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(self.res * 8),
                rows_per_image: Some(self.res),
            },
            wgpu::Extent3d {
                width: self.res,
                height: self.res,
                depth_or_array_layers: 1,
            },
        );
    }
}

/// `Rgba16Float` texture for the field (sampled by the scenes).
///
/// `Rgba16Float` rather than `R32Float` because it is filterable on every wgpu
/// adapter (float32 filtering is an optional feature) and it is the format the
/// JS `HalfFloatType` render targets use.
pub fn create_wave_texture(device: &wgpu::Device, res: u32) -> wgpu::Texture {
    let res = res.max(1);
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("point10-water-wave"),
        size: wgpu::Extent3d {
            width: res,
            height: res,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

/// The field's sampler: `wrapS: RepeatWrapping`, `wrapT:
/// ClampToEdgeWrapping`, linear min/mag (the JS render target's defaults).
pub fn wave_sampler(device: &wgpu::Device) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("point10-water-wave-sampler"),
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Nearest,
        ..Default::default()
    })
}

/// `1.0` as IEEE binary16 bits.
const fn f16_one() -> u16 {
    0x3c00
}

/// IEEE binary32 → IEEE binary16, round-to-nearest-even.
///
/// The field is uploaded as `Rgba16Float`, so the CPU-side `f32` values need a
/// half-float encoding. NaN payloads collapse to a quiet NaN and infinities
/// survive (the simulation never produces either, but the conversion is total).
pub fn f32_to_f16_bits(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xff) as i32;
    let mantissa = bits & 0x007f_ffff;

    if exponent == 0xff {
        // Infinity or NaN.
        return sign | 0x7c00 | if mantissa != 0 { 0x200 } else { 0 };
    }

    // Re-bias 127 -> 15.
    let exponent = exponent - 127 + 15;

    if exponent >= 0x1f {
        // Overflow to infinity (matches hardware f16 conversion).
        return sign | 0x7c00;
    }

    if exponent <= 0 {
        // Subnormal half or zero; below 2^-24 the value rounds to zero.
        if exponent < -10 {
            return sign;
        }
        let mantissa = mantissa | 0x0080_0000;
        let shift = (14 - exponent) as u32;
        let mut half = (mantissa >> shift) as u16;
        let remainder = mantissa & ((1 << shift) - 1);
        let halfway = 1_u32 << (shift - 1);
        if remainder > halfway || (remainder == halfway && (half & 1) == 1) {
            half += 1;
        }
        return sign | half;
    }

    let mut half = ((exponent as u16) << 10) | ((mantissa >> 13) as u16);
    let remainder = mantissa & 0x1fff;
    if remainder > 0x1000 || (remainder == 0x1000 && (half & 1) == 1) {
        // May carry into the exponent (and to infinity), which is correct.
        half = half.wrapping_add(1);
    }
    sign | half
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Decode binary16 for assertions (`f32_to_f16_bits`'s inverse).
    fn f16_bits_to_f32(bits: u16) -> f32 {
        let sign = if bits & 0x8000 != 0 { -1.0_f32 } else { 1.0 };
        let exponent = ((bits >> 10) & 0x1f) as i32;
        let mantissa = (bits & 0x03ff) as f32;
        match exponent {
            0 => sign * mantissa * 2.0_f32.powi(-24),
            0x1f => {
                if mantissa == 0.0 {
                    sign * f32::INFINITY
                } else {
                    f32::NAN
                }
            }
            _ => sign * (1.0 + mantissa / 1024.0) * 2.0_f32.powi(exponent - 15),
        }
    }

    /// Node 24 reference: an 8x8 field, `drop(0.5, 0.5, 0.05, 0.4)`, three
    /// steps, `drop(0.2, 0.7, 0.03, 0.3)`, five steps. Values are the heights
    /// in row-major order with row 0 = v 0.
    const JS_REFERENCE_8: [f32; 64] = [
        0.021_585_243,
        0.016_445_114,
        0.004_026_273,
        -0.005_045_335_6,
        -0.006_950_926_7,
        0.001_112_066_8,
        0.012_917_129,
        0.020_382_684,
        0.018_242_827,
        0.016_344_753,
        0.013_719_702,
        0.009_337_786,
        0.007_880_124_9,
        0.008_466_897,
        0.014_645_759,
        0.018_312_491,
        0.002_646_764_8,
        0.007_775_200_8,
        0.016_361_782,
        0.016_289_346,
        0.019_193_665,
        0.022_318_933,
        0.015_532_094,
        0.005_446_367,
        -0.011_865_958,
        -0.000_048_683_4,
        0.007_172_224_3,
        0.007_637_715_6,
        0.015_549_507,
        0.023_626_933,
        0.011_069_808,
        -0.008_481_483,
        -0.012_460_868,
        0.003_164_880_5,
        0.008_012_885,
        0.005_342_107,
        0.012_848_194,
        0.023_529_317,
        0.008_066_345,
        -0.011_105_64,
        0.002_324_193_4,
        0.012_452_737,
        0.017_052_509,
        0.011_345_102,
        0.014_209_826,
        0.024_280_755,
        0.011_967_018,
        0.000_006_955_4,
        0.013_045_376,
        0.014_688_413,
        0.009_400_144,
        0.004_327_420_7,
        0.010_728_098,
        0.020_096_103,
        0.019_513_807,
        0.014_342_102,
        0.013_253_296,
        0.007_240_180_6,
        -0.005_313_354_7,
        -0.008_337_632,
        0.004_137_175,
        0.020_792_136,
        0.027_133_816,
        0.019_814_735,
    ];

    #[test]
    fn new_field_is_at_rest() {
        let sim = WaveSim::new(8);
        assert_eq!(sim.res(), 8);
        assert_eq!(sim.data().len(), 8 * 8 * 2);
        assert!(sim.data().iter().all(|value| *value == 0.0));
        assert_eq!(sim.height(3, 4), 0.0);
    }

    #[test]
    fn matches_the_js_reference_grid() {
        let mut sim = WaveSim::new(8);
        sim.drop(0.5, 0.5, 0.05, 0.4);
        for _ in 0..3 {
            sim.step();
        }
        sim.drop(0.2, 0.7, 0.03, 0.3);
        for _ in 0..5 {
            sim.step();
        }

        for (index, expected) in JS_REFERENCE_8.iter().enumerate() {
            let x = (index % 8) as u32;
            let y = (index / 8) as u32;
            let actual = sim.height(x, y);
            assert!(
                (actual - expected).abs() < 1e-5,
                "texel ({x}, {y}): {actual} != {expected}"
            );
        }
    }

    /// The same reference against an independent transcription of the two GLSL
    /// passes, so a drift in either the CPU port or the pinned values shows up.
    #[test]
    fn pinned_values_agree_with_a_direct_transcription() {
        fn reference_drop(state: &mut [f32], res: u32, u: f32, v: f32, strength: f32, radius: f32) {
            for y in 0..res {
                for x in 0..res {
                    let index = ((y * res + x) * 2) as usize;
                    let cx = (x as f32 + 0.5) / res as f32;
                    let cy = (y as f32 + 0.5) / res as f32;
                    let distance = ((cx - u).powi(2) + (cy - v).powi(2)).sqrt();
                    let drop = (1.0 - distance / radius).max(0.0);
                    state[index] += (0.5 - (drop * std::f32::consts::PI).cos() * 0.5) * strength;
                }
            }
        }

        fn reference_step(state: &mut [f32], res: u32) {
            let mut next = state.to_vec();
            let h = |x: i64, y: i64| {
                let xx = x.rem_euclid(res as i64);
                let yy = y.clamp(0, res as i64 - 1);
                state[((yy * res as i64 + xx) * 2) as usize]
            };
            for y in 0..res as i64 {
                for x in 0..res as i64 {
                    let index = ((y * res as i64 + x) * 2) as usize;
                    let ave = (h(x + 1, y) + h(x - 1, y) + h(x, y + 1) + h(x, y - 1)) * 0.25;
                    let mut g = state[index + 1];
                    let mut r = state[index];
                    g += (ave - r) * 2.0;
                    g *= 0.995;
                    r += g;
                    next[index] = r;
                    next[index + 1] = g;
                }
            }
            state.copy_from_slice(&next);
        }

        let res = 8;
        let mut state = vec![0.0_f32; (res * res * 2) as usize];
        reference_drop(&mut state, res, 0.5, 0.5, 0.05, 0.4);
        for _ in 0..3 {
            reference_step(&mut state, res);
        }
        reference_drop(&mut state, res, 0.2, 0.7, 0.03, 0.3);
        for _ in 0..5 {
            reference_step(&mut state, res);
        }

        for (index, expected) in JS_REFERENCE_8.iter().enumerate() {
            assert!(
                (state[index * 2] - expected).abs() < 1e-6,
                "reference transcription drifted at {index}"
            );
        }
    }

    #[test]
    fn drop_is_a_cosine_bump_at_texel_centres() {
        let mut sim = WaveSim::new(8);
        sim.drop(0.5, 0.5, 0.05, 0.4);
        // Texel (4, 4) has its centre at 0.5625; texel (3, 3) at 0.4375. Both
        // are 0.0884 away from the drop centre on the diagonal.
        let centre = sim.height(4, 4);
        let mirrored = sim.height(3, 3);
        assert!((centre - mirrored).abs() < 1e-6, "the bump is symmetric");
        let distance = ((0.5625 - 0.5_f32).powi(2) * 2.0).sqrt();
        let drop = 1.0 - distance / 0.4;
        let expected = (0.5 - (drop * std::f32::consts::PI).cos() * 0.5) * 0.05;
        assert!((centre - expected).abs() < 1e-6);
        // The velocity channel is untouched by a drop.
        assert_eq!(sim.velocity(4, 4), 0.0);
    }

    #[test]
    fn drop_outside_the_radius_leaves_the_field_alone() {
        let mut sim = WaveSim::new(8);
        sim.drop(2.0, 2.0, 0.05, 0.4);
        assert!(sim.data().iter().all(|value| *value == 0.0));
        // A non-positive radius is a no-op (see the module docs).
        sim.drop(0.5, 0.5, 0.05, 0.0);
        assert!(sim.data().iter().all(|value| *value == 0.0));
    }

    #[test]
    fn step_wraps_in_u_and_clamps_in_v() {
        let mut sim = WaveSim::new(4);
        // Raise a single texel; the step pulls its four neighbours up through
        // velocity and pulls the source down.
        sim.state[((1 * 4 + 1) * 2) as usize] = 1.0;

        // Texel (0, 1)'s left neighbour wraps to (3, 1): zero, so the average
        // is (h(1,1) + h(3,1) + h(0,2) + h(0,0)) / 4 = 1/4.
        sim.step();
        let velocity = sim.velocity(0, 1);
        assert!((velocity - 0.25 * 2.0 * 0.995).abs() < 1e-6);
        assert!((sim.height(0, 1) - velocity).abs() < 1e-6);

        // The clamped edge does not wrap in v. Raise both (1,0) and (1,3):
        // clamped, (1,0)'s up neighbour reads its own height (1) and the
        // average is 1/4; if v wrapped it would read (1,3) = 1 and the average
        // would be 1/2, changing the velocity.
        let mut sim = WaveSim::new(4);
        sim.state[((0 * 4 + 1) * 2) as usize] = 1.0;
        sim.state[((3 * 4 + 1) * 2) as usize] = 1.0;
        sim.step();
        let average = 1.0 / 4.0;
        let expected = (average - 1.0) * 2.0 * 0.995;
        assert!((sim.velocity(1, 0) - expected).abs() < 1e-6);
    }

    #[test]
    fn waves_stay_finite_and_bounded_over_a_long_run() {
        // A quarter-resolution field keeps the 400 steps cheap while still
        // exercising the propagated wavefronts.
        let mut sim = WaveSim::new(WAVE_RES / 4);
        sim.drop(0.5, 0.5, 0.04, 0.12);
        for step in 0..400 {
            sim.update(1.0 / 60.0);
            if step % 50 == 0 {
                sim.drop(
                    (step as f32 * 0.017).fract(),
                    (step as f32 * 0.031).fract(),
                    0.02,
                    0.05,
                );
            }
        }
        assert!(
            sim.data().iter().all(|value| value.is_finite()),
            "the field must stay finite"
        );
        assert!(
            sim.data().iter().all(|value| value.abs() < 1.0),
            "the field must stay bounded"
        );
    }

    #[test]
    fn update_is_one_step_like_the_js() {
        let mut stepped = WaveSim::new(8);
        stepped.drop(0.5, 0.5, 0.05, 0.4);
        stepped.update(f32::NAN);
        let mut direct = WaveSim::new(8);
        direct.drop(0.5, 0.5, 0.05, 0.4);
        direct.step();
        assert_eq!(stepped.data(), direct.data());
    }

    #[test]
    fn f16_conversion_round_trips_the_field() {
        assert_eq!(f32_to_f16_bits(0.0), 0x0000);
        assert_eq!(f32_to_f16_bits(1.0), 0x3c00);
        assert_eq!(f32_to_f16_bits(-2.0), 0xc000);
        assert_eq!(f32_to_f16_bits(0.5), 0x3800);
        // 65504 is the largest finite half; 65536 overflows to infinity.
        assert_eq!(f32_to_f16_bits(65_504.0), 0x7bff);
        assert_eq!(f32_to_f16_bits(65_536.0), 0x7c00);
        assert_eq!(f32_to_f16_bits(f32::INFINITY), 0x7c00);
        assert!(f32_to_f16_bits(f32::NAN) & 0x7c00 == 0x7c00);

        for value in [0.0, 0.001, 0.01, 0.04, -0.0125, 0.5, 1.0, -3.25] {
            let round_trip = f16_bits_to_f32(f32_to_f16_bits(value));
            assert!((round_trip - value).abs() < 1e-3, "{value} -> {round_trip}");
        }
        // Ties round to even: half an ulp at 0.5 is 2^-12 (f16 has 10
        // mantissa bits, so its ulp there is 2^-11).
        assert_eq!(f32_to_f16_bits(0.5 + 1.0 / 4096.0), 0x3800);
        assert_eq!(f32_to_f16_bits(0.5 + 3.0 / 4096.0), 0x3802);
    }

    #[test]
    fn pack_produces_eight_bytes_per_texel() {
        let mut sim = WaveSim::new(4);
        sim.drop(0.5, 0.5, 0.05, 0.4);
        let bytes = sim.pack_rgba16f();
        assert_eq!(bytes.len(), 4 * 4 * 8);
        // Alpha is opaque in every texel.
        for texel in 0..16 {
            let alpha = u16::from_le_bytes([bytes[texel * 8 + 6], bytes[texel * 8 + 7]]);
            assert_eq!(alpha, f16_one());
        }
        // The centre texel's height is the f16 of the CPU value.
        let centre = ((2 * 4 + 2) * 2) as usize;
        let packed = u16::from_le_bytes([bytes[(2 * 4 + 2) * 8], bytes[(2 * 4 + 2) * 8 + 1]]);
        assert_eq!(packed, f32_to_f16_bits(sim.data()[centre]));
    }
}
