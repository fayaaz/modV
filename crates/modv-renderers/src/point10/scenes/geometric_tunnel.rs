//! GeometricTunnel — Rust/wgpu port of
//! `src/renderer/src/application/point10/scenes/GeometricTunnel.js`.
//!
//! Twelve neon polygonal hoops recede down a -Z tunnel: an extruded outer band,
//! a smaller counter-rotated inner band, radial braces with diamond nodes,
//! occasional "ros" clusters of tile rings, additive dust, lightning arcs
//! between hoop corners, and a large target ring at the far end. A camera that
//! drifts in a small circle looks down the tunnel; the scroll speed, sway,
//! emissive brightness, dust and bloom all ride the bass, mids and the kick.
//!
//! # Ported exactly
//!
//! * the hoop layout (`:210-321`): 12 rings, `spacing 3.2`,
//!   `tunnelLen = ringCount * spacing`, `radius 4.2`, the ring side tables
//!   (`OUTER_SIDES` / `INNER_SIDES`), the inner band at `radius * 0.42` and
//!   thickness `0.22`, `frame.rotation.z = ((i % 3) - 1) * PI / sides`,
//!   `inner.rotation.z = PI / innerSides + (i % 2) * 0.16`, the brace count
//!   (`i % 2 ? 3 : 4`) with the `(innerR + outerR) / 2` placement and the
//!   `outerR - innerR`/`0.07`/`0.16` brace scale, and the `i % 3 === 0` ros
//!   cluster (ring tiles at `radius * 0.16`, thickness 0.1);
//! * the target ring (`:471-488`): a 32-sided band at `radius * 0.82`, a
//!   4-sided inner band at `radius * 0.34` rotated `PI / 10`, and
//!   `target.position.z = -tunnelLen - 2`;
//! * the per-frame ring motion (`:675-736`): `DT`-independent `PASSTHR = 6`
//!   wrap, `depth = 1 + z / tunnelLen`, the sway formula
//!   `(0.3 + b * 1.2) * swayAmp * (0.35 + depth * 0.65)` on both axes, the
//!   alternating `group.rotation.z`, `pulse = 1 + b * 0.06 + flash * 0.04`,
//!   `near = clamp((3 - z) / 7, 0, 1)`, `p = pulse * (0.02 + 0.98 * near^2)`
//!   and the z scale `0.82 + b * 0.22 + flash * 0.18`;
//! * the colouring (`:703-735`): `hue = (0.62 + i * 0.045 + time * 0.04) % 1`,
//!   `l = min(0.48, 0.34 + m * 0.06 + flash * 0.08)`, `setHSL(hue, 0.9, l)`,
//!   `bright = (0.48 + depth * 0.28 + b * 0.2 + flash * 0.38) *
//!   brightnessScale`, the frame `emissive * 0.5` / intensity `bright * 0.32`,
//!   the inner `emissive * 0.42` / `bright * 0.5`, the ros tile
//!   `emissive * 0.85` / `bright * 0.7 * near`, and the target's own
//!   `setHSL((0.55 + time * 0.02) % 1, 1, 0.38)` ramps;
//! * the audio mapping (`:627-673`): `beta = 1 - exp(-dt * 3.5)` one-pole on
//!   `band(0, 0, 0)`/`band(1, 0, 0)`, `flash = max(kick, flash * exp(-dt*3))`,
//!   `turn = (2.6 + b*5 + m*2.5 + flash*3) * speedScale`,
//!   `scroll += dt * turn`, the dust colour/size/opacity ramps, the light
//!   intensities, and the two bloom strengths
//!   (`0.68 + b*0.28 + flash*0.22`, mix `0.55 + b*0.14 + flash*0.12`);
//! * the camera (`:997-1007`): `(sin(t*0.15)*0.3, cos(t*0.12)*0.2, 3)`,
//!   `lookAt(0, 0, -tunnelLen/2)`, a local Z roll of
//!   `sin(t*0.17)*0.025 + sin(t*0.07)*0.01`, and `fov = 64 + smBass * 3`;
//! * the surface maps (`:48-156`): the periodic value-noise grain and the
//!   equirect studio environment, byte for byte;
//! * the post chain (`:490-559`, `:1019-1045`): RenderPass → SSAO →
//!   `base + bloom * strength` (with `alpha = clamp(max(rgb))`) → OutputPass
//!   with ACES tone mapping at exposure 1.
//!
//! # Substitutions
//!
//! | three.js | Rust |
//! |----------|------|
//! | `ExtrudeGeometry` + `Shape`/holes with rounded corners | [`build_ring_band`] extrudes an annulus polygon band with straight corners (no bevel/rounded arcs) |
//! | `OctahedronGeometry(0.11)` nodes | a small rotated box instance (`0.22` across) |
//! | `Line2`/`LineMaterial` lightning (screen-space lines) | world-space additive triangle ribbons built from [`bolt_path`] / [`branch_path`], same 4-level midpoint jag, same flicker/life |
//! | `MeshStandardMaterial` + point lights + PMREM env | a hemi/key/rim + point-light `fs_main` with the same tints, plus linear fog to `0x07071a` (no PMREM pass exists here) |
//! | `SSAOPass` (deferred: beauty + normal + kernel) | [`SsaoPass`] depth-only screen-space AO — see its module docs; same `kernelRadius`/`minDistance`/`maxDistance` parameters, same "multiply the beauty" output |
//! | two composers (bloom-only + final) | the port renders the scene once, blooms it and adds `bloom * mixStrength` in its final pass; the JS's bloom-only pass exists so the bloom does not recurse, which one pass already guarantees |
//! | `LineMaterial.resolution` pixel widths | fixed world-space ribbon widths (documented above) |
//! | `Math.random()` for sway/bolt/dust seeds | [`seeded_rand`] with frame counters, so a run is deterministic |
//! | `createParticleTexture()` dust sprite | the same circular mask computed in the dust fragment shader (no texture upload) |
//!
//! # Transparency
//!
//! `screenClearAlpha = 0` (`:214`) and the final shader writes
//! `alpha = clamp(max(rgb))` (`:541`), so the tunnel composites over the
//! layers below and empty space stays clear. The port keeps that rule in its
//! final pass; the SSAO pass leaves background pixels at AO 1 so the clear
//! survives, and the bloom is added before the alpha is derived (as in the JS).

#![forbid(unsafe_code)]

use crate::point10::audio_bridge::AudioBridge;
use crate::point10::postfx::{Composer, SsaoParams, SsaoPass, ToneMapping, UnrealBloomPass};
use crate::point10::scenes::cube_mesh::{
    DEPTH_FORMAT, Mat4, Mesh, MeshGpu, Vertex, depth_state, look_at, mat4_mul, normalise,
    perspective, rotation_z, scale, transform_point, translation, trs,
};
use crate::point10::shared::phrase_color::srgb_hex_to_linear;
use crate::point10::shared::random::seeded_rand;
use crate::point10::{
    FULLSCREEN_TRIANGLE_WGSL, HDR_FORMAT, Point10Scene, SceneContext, SceneMeta, SceneProp,
    UnitQuad, frame_dt, read_scene_props, scene_props_with_levers,
};

// ---------------------------------------------------------------------------
// Constants (`GeometricTunnel.js:15-42`, `:210-226`)
// ---------------------------------------------------------------------------

/// `TUNNEL_BLOOM_LAYER` — the JS renders only this layer into the bloom
/// composer; the port blooms the whole frame (see the module docs).
pub const TUNNEL_BLOOM_LAYER: u32 = 1;
/// `TUNNEL_HOOP_DEPTH`.
pub const TUNNEL_HOOP_DEPTH: f32 = 0.05;
/// `TUNNEL_GRAIN_SIZE`.
pub const TUNNEL_GRAIN_SIZE: u32 = 96;
/// `TUNNEL_ENV_WIDTH`.
pub const TUNNEL_ENV_WIDTH: u32 = 128;
/// `TUNNEL_ENV_HEIGHT`.
pub const TUNNEL_ENV_HEIGHT: u32 = 64;

/// `TUNNEL_SURFACE`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TunnelSurface {
    /// `metalness`.
    pub metalness: f32,
    /// `roughness`.
    pub roughness: f32,
    /// `envMapIntensity`.
    pub env_intensity: f32,
    /// `bumpScale`.
    pub bump_scale: f32,
}

/// `TUNNEL_SURFACE` exactly.
pub const TUNNEL_SURFACE: TunnelSurface = TunnelSurface {
    metalness: 0.9,
    roughness: 0.8,
    env_intensity: 1.45,
    bump_scale: 0.012,
};

/// `TUNNEL_SSAO` (`kernelRadius` / `minDistance` / `maxDistance`) with the
/// port's strength of 1 and screen-space bias.
pub const TUNNEL_SSAO: SsaoParams = SsaoParams {
    radius: 0.5,
    min_distance: 0.000_08,
    max_distance: 0.0025,
    strength: 1.0,
    bias: 0.04,
};

/// The JS neon material's `color: 0x3a4358` (`_neonMat`).
pub const TUNNEL_BASE_TINT_HEX: u32 = 0x003a_4358;
/// `TUNNEL_BASE_TINT_HEX` converted from sRGB to linear, like three's
/// `MeshStandardMaterial.color` (the renderer's colour management converts
/// it once at material creation).
pub const TUNNEL_BASE_TINT: [f32; 3] = [0.042_312, 0.056_128, 0.097_587];

/// `OUTER_SIDES`.
pub const OUTER_SIDES: [u32; 6] = [4, 8, 3, 32, 10, 12];
/// `INNER_SIDES`.
pub const INNER_SIDES: [u32; 6] = [32, 12, 4, 8, 3, 10];

/// `ringCount`.
pub const RING_COUNT: usize = 12;
/// `spacing`.
pub const SPACING: f32 = 3.2;
/// `tunnelLen`.
pub const TUNNEL_LEN: f32 = RING_COUNT as f32 * SPACING;
/// `radius`.
pub const RADIUS: f32 = 4.2;
/// `PASSTHR` (`:679`).
pub const PASSTHR: f32 = 6.0;

/// `dustCount`.
pub const DUST_COUNT: usize = 280;
/// The bolt pool size (`for (let b = 0; b < 10; b++)`).
pub const BOLT_COUNT: usize = 10;
/// `BOLT_PTS`.
pub const BOLT_POINTS: usize = 17;
/// The branch path's point count (5).
pub const BRANCH_POINTS: usize = 5;

/// `camera.fov` base (`64 + smBass * 3`).
pub const CAMERA_FOV: f32 = 64.0;
/// `PerspectiveCamera(…, 0.1, 1000)`.
pub const CAMERA_NEAR: f32 = 0.1;
/// Far plane.
pub const CAMERA_FAR: f32 = 1000.0;
/// `new THREE.Fog(0x07071a, 10, tunnelLen * 1.3)`.
pub const FOG_HEX: u32 = 0x0007_071a;
/// The fog near distance.
pub const FOG_NEAR: f32 = 10.0;
/// The fog far distance (`tunnelLen * 1.3`).
pub const FOG_FAR: f32 = TUNNEL_LEN * 1.3;

/// `screenClearAlpha`.
pub const CLEAR_ALPHA: f64 = 0.0;

/// `BLOOM.strength` base (`UnrealBloomPass(size, 0.72, 1, 0.35)`).
pub const BLOOM_STRENGTH: f32 = 0.72;
/// `BLOOM.radius`.
pub const BLOOM_RADIUS: f32 = 1.0;
/// `BLOOM.threshold`.
pub const BLOOM_THRESHOLD: f32 = 0.35;

/// The maximum number of emissive geometry instances in one frame.
pub const MAX_INSTANCES: usize = 512;
/// The bolt ribbon vertex buffer capacity (24 active ribbons at 17 points).
pub const MAX_BOLT_VERTICES: usize = 4096;
/// Vertices one bolt writes: glow (16 segments) + core (16) + branch (4), six
/// per segment.
pub const BOLT_VERTICES_PER_BOLT: usize = (BOLT_POINTS - 1) * 6 * 2 + (BRANCH_POINTS - 1) * 6;

// ---------------------------------------------------------------------------
// Pure maths
// ---------------------------------------------------------------------------

/// JS `%` (the result takes the sign of the divisor).
pub fn js_mod(value: f32, modulus: f32) -> f32 {
    let result = value % modulus;
    if result < 0.0 {
        result + modulus
    } else {
        result
    }
}

/// `tunnelHash(x, y, seed)` (`:48-51`).
pub fn tunnel_hash(x: u32, y: u32, seed: u32) -> f32 {
    let value =
        ((x as f32) * 127.1 + (y as f32) * 311.7 + (seed as f32) * 74.7).sin() * 43_758.545_312_3;
    value - value.floor()
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// `tileableNoise(u, v, cells, seed)` (`:55-71`) — a periodic value noise whose
/// lattice wraps at `cells`, so the surface maps tile.
pub fn tileable_noise(u: f32, v: f32, cells: u32, seed: u32) -> f32 {
    let x = u * cells as f32;
    let y = v * cells as f32;
    let xi = x.floor();
    let yi = y.floor();
    // The JS uses `((xi % cells) + cells) % cells` on the floored value; a
    // non-negative u/v only needs the positive modulo.
    let x0 = (xi as i64).rem_euclid(cells as i64) as u32;
    let y0 = (yi as i64).rem_euclid(cells as i64) as u32;
    let x1 = (x0 + 1) % cells;
    let y1 = (y0 + 1) % cells;
    let tx = x - xi;
    let ty = y - yi;
    let sx = tx * tx * (3.0 - 2.0 * tx);
    let sy = ty * ty * (3.0 - 2.0 * ty);
    let top = lerp(tunnel_hash(x0, y0, seed), tunnel_hash(x1, y0, seed), sx);
    let bottom = lerp(tunnel_hash(x0, y1, seed), tunnel_hash(x1, y1, seed), sx);
    lerp(top, bottom, sy)
}

/// `buildTunnelGrain(size)` (`:75-106`) — the grayscale map that drives bump,
/// roughness (G) and emissive wear (RGB).
pub fn build_tunnel_grain(size: u32) -> Vec<u8> {
    let size = size.max(1);
    let mut data = vec![0_u8; (size * size * 4) as usize];
    let octaves = [
        (3_u32, 0.5_f32, 11_u32),
        (7, 0.28, 23),
        (15, 0.14, 37),
        (31, 0.08, 53),
    ];

    for y in 0..size {
        for x in 0..size {
            let u = x as f32 / size as f32;
            let v = y as f32 / size as f32;
            let mut noise = 0.0;
            let mut weight = 0.0;
            for (cells, amplitude, seed) in octaves {
                noise += tileable_noise(u, v, cells, seed) * amplitude;
                weight += amplitude;
            }
            noise /= weight;
            let brushed = ((u * 61.0 + noise * 9.0) * std::f32::consts::PI * 2.0).sin() * 0.5 + 0.5;
            let mixed = (noise * 0.82 + brushed * 0.18).clamp(0.0, 1.0);
            let height = ((mixed - 0.5) * 1.8 + 0.5).clamp(0.0, 1.0);
            let value = ((0.3 + height * 0.56) * 255.0).round().clamp(0.0, 255.0) as u8;
            let index = ((y * size + x) * 4) as usize;
            data[index] = value;
            data[index + 1] = value;
            data[index + 2] = value;
            data[index + 3] = 255;
        }
    }
    data
}

/// `buildTunnelEnvironment(width, height)` (`:110-156`) — the equirect studio
/// the metal reflects.
pub fn build_tunnel_environment(width: u32, height: u32) -> Vec<u8> {
    let width = width.max(1);
    let height = height.max(1);
    let mut data = vec![0_u8; (width * height * 4) as usize];
    let floor = [16.0_f32, 6.0, 30.0];
    let horizon = [34.0_f32, 42.0, 82.0];
    let sky = [138.0_f32, 176.0, 244.0];
    let panels = [
        (0.16_f32, 0.045_f32, [255.0_f32, 124.0, 200.0], 0.55_f32),
        (0.54, 0.05, [186.0, 214.0, 255.0], 0.8),
        (0.82, 0.04, [226.0, 238.0, 255.0], 1.0),
    ];
    let clamp_byte = |value: f32| value.clamp(0.0, 255.0).round() as u8;

    for y in 0..height {
        let v = if height > 1 {
            y as f32 / (height - 1) as f32
        } else {
            0.0
        };
        let base = if v < 0.58 {
            [
                lerp(floor[0], horizon[0], v / 0.58),
                lerp(floor[1], horizon[1], v / 0.58),
                lerp(floor[2], horizon[2], v / 0.58),
            ]
        } else {
            [
                lerp(horizon[0], sky[0], (v - 0.58) / 0.42),
                lerp(horizon[1], sky[1], (v - 0.58) / 0.42),
                lerp(horizon[2], sky[2], (v - 0.58) / 0.42),
            ]
        };

        for x in 0..width {
            let u = if width > 1 {
                x as f32 / (width - 1) as f32
            } else {
                0.0
            };
            let streak = 0.82 + tileable_noise(u, v * 0.35, 9, 71) * 0.36;
            let panel_width = 0.55 + 0.45 * (u * std::f32::consts::PI * 10.0).cos().powi(2);
            let mut rgb = [base[0] * streak, base[1] * streak, base[2] * streak];

            for (panel_v, width_ratio, tint, gain) in panels {
                let d = (v - panel_v) / width_ratio;
                let band = (-d * d).exp() * gain * panel_width;
                for channel in 0..3 {
                    rgb[channel] += tint[channel] * band;
                }
            }

            let bounce = (1.0 - v / 0.22).max(0.0) * 0.5;
            rgb[0] += 46.0 * bounce;
            rgb[1] += 10.0 * bounce;
            rgb[2] += 34.0 * bounce;

            let index = ((y * width + x) * 4) as usize;
            data[index] = clamp_byte(rgb[0]);
            data[index + 1] = clamp_byte(rgb[1]);
            data[index + 2] = clamp_byte(rgb[2]);
            data[index + 3] = 255;
        }
    }
    data
}

/// `tunnelRingSides(index, inner)` (`:158-161`).
pub fn ring_sides(index: usize, inner: bool) -> u32 {
    let table = if inner { &INNER_SIDES } else { &OUTER_SIDES };
    table[index % table.len()]
}

/// Three's `Color.setHSL` (h, s, l in `0..1`, RGB out) — the scenes need the
/// CPU twin because the instance colours are authored on the CPU.
pub fn hsl_to_rgb(hue: f32, saturation: f32, lightness: f32) -> [f32; 3] {
    let hue = js_mod(hue, 1.0);
    if saturation <= 0.0 {
        return [lightness; 3];
    }
    let q = if lightness < 0.5 {
        lightness * (1.0 + saturation)
    } else {
        lightness + saturation - lightness * saturation
    };
    let p = 2.0 * lightness - q;
    let hue_to_rgb = |mut t: f32| {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    [
        hue_to_rgb(hue + 1.0 / 3.0),
        hue_to_rgb(hue),
        hue_to_rgb(hue - 1.0 / 3.0),
    ]
}

/// The scene's per-frame drive (`:627-673`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TunnelDrive {
    /// `this.scroll` for the next frame.
    pub scroll: f32,
    /// `this.smBass`.
    pub smoothed_bass: f32,
    /// `this.smMid`.
    pub smoothed_mid: f32,
    /// `this.flash`.
    pub flash: f32,
    /// `turn` — the scroll velocity this frame.
    pub turn: f32,
}

/// Advance `smBass`/`smMid`/`flash`/`scroll` exactly like `update()`.
pub fn advance_tunnel(
    scroll: f32,
    smoothed_bass: f32,
    smoothed_mid: f32,
    flash: f32,
    dt: f32,
    bass: f32,
    mid: f32,
    kick: f32,
    speed_scale: f32,
) -> TunnelDrive {
    let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
    let scale = if speed_scale.is_finite() {
        speed_scale.max(0.0)
    } else {
        1.0
    };
    let beta = 1.0 - (-dt * 3.5).exp();
    let smoothed_bass = smoothed_bass + (bass - smoothed_bass) * beta;
    let smoothed_mid = smoothed_mid + (mid - smoothed_mid) * beta;
    let flash = kick.max(flash * (-dt * 3.0).exp());
    let turn = (2.6 + smoothed_bass * 5.0 + smoothed_mid * 2.5 + flash * 3.0) * scale;
    TunnelDrive {
        scroll: scroll + dt * turn,
        smoothed_bass,
        smoothed_mid,
        flash,
        turn,
    }
}

/// One hoop's transform for a frame (`:675-701`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RingPose {
    /// World position.
    pub position: [f32; 3],
    /// Local Z rotation.
    pub rotation_z: f32,
    /// Uniform X/Y scale.
    pub scale: f32,
    /// Z scale (`0.82 + b * 0.22 + flash * 0.18`).
    pub depth_scale: f32,
    /// `depth = 1 + z / tunnelLen`.
    pub depth: f32,
    /// `near = clamp((3 - z) / 7, 0, 1)`.
    pub near: f32,
}

/// `ringPose(i, scroll, time, b, flash, sway, swayAxis, swayAmp)`.
pub fn ring_pose(
    index: usize,
    scroll: f32,
    time: f32,
    bass: f32,
    flash: f32,
    sway: f32,
    sway_axis: f32,
    sway_amp: f32,
) -> RingPose {
    let wrapped = js_mod((index as f32 + 0.5) * SPACING - scroll, TUNNEL_LEN);
    let z = -wrapped + PASSTHR;
    let depth = 1.0 + z / TUNNEL_LEN;

    let sway_on = (0.3 + bass * 1.2) * sway_amp * (0.35 + depth * 0.65);
    let x = (time * 0.5 + sway).cos() * sway_on * sway_axis.cos();
    let y = (time * 0.4 + sway * 1.7).sin() * sway_on * sway_axis.sin();

    let sign = if index % 2 == 1 { 1.0 } else { -1.0 };
    let rotation_z =
        sign * (index as f32 * 0.05 + scroll * 0.22 + (time * 0.3 + index as f32).sin() * 0.09);
    let pulse = 1.0 + bass * 0.06 + flash * 0.04;
    let near = ((3.0 - z) / 7.0).clamp(0.0, 1.0);
    let scale = pulse * (0.02 + 0.98 * near * near);

    RingPose {
        position: [x, y, z],
        rotation_z,
        scale,
        depth_scale: 0.82 + bass * 0.22 + flash * 0.18,
        depth,
        near,
    }
}

/// A hoop ring's emissive colour (`:703-735`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RingColour {
    /// The HSL colour (`setHSL(hue, 0.9, l)`).
    pub colour: [f32; 3],
    /// `bright`, already multiplied by `brightnessScale`.
    pub bright: f32,
    /// The frame band's emissive intensity (`bright * 0.32`).
    pub frame_intensity: f32,
    /// The inner band's (`bright * 0.5`).
    pub inner_intensity: f32,
    /// The ros tiles' (`bright * 0.7 * near`).
    pub ros_intensity: f32,
}

/// `ringColour(i, time, m, flash, depth, b, near, brightnessScale)`.
pub fn ring_colour(
    index: usize,
    time: f32,
    mid: f32,
    flash: f32,
    depth: f32,
    bass: f32,
    near: f32,
    brightness_scale: f32,
) -> RingColour {
    let hue = js_mod(0.62 + index as f32 * 0.045 + time * 0.04, 1.0);
    let lightness = 0.48_f32.min(0.34 + mid * 0.06 + flash * 0.08);
    let colour = hsl_to_rgb(hue, 0.9, lightness);
    let bright = (0.48 + depth * 0.28 + bass * 0.2 + flash * 0.38) * brightness_scale;
    RingColour {
        colour,
        bright,
        frame_intensity: bright * 0.32,
        inner_intensity: bright * 0.5,
        ros_intensity: bright * 0.7 * near,
    }
}

/// One dust particle's world position (`:440-446`), fixed for the scene's
/// lifetime.
pub fn dust_seed(index: usize) -> [f32; 3] {
    let angle = seeded_rand(index as i32 * 3 + 1, 0) * std::f32::consts::PI * 2.0;
    let radius = seeded_rand(index as i32 * 5 + 2, 1).sqrt() * RADIUS * 0.9;
    [
        angle.cos() * radius,
        angle.sin() * radius,
        -seeded_rand(index as i32 * 7 + 3, 2) * TUNNEL_LEN,
    ]
}

/// `_boltPath(a, b, amp, phase)` (`:948-977`): 4 midpoint-displacement levels
/// with `amp *= 0.5` each.
pub fn bolt_path(a: [f32; 3], b: [f32; 3], amplitude: f32, phase: f32, time: f32) -> Vec<[f32; 3]> {
    let mut points = vec![a, b];
    let mut amplitude = amplitude;

    for level in 0..4 {
        let mut next: Vec<[f32; 3]> = Vec::with_capacity(points.len() * 2);
        for index in 0..points.len().saturating_sub(1) {
            let p = points[index];
            let q = points[index + 1];
            next.push(p);
            let mid = [
                (p[0] + q[0]) * 0.5,
                (p[1] + q[1]) * 0.5,
                (p[2] + q[2]) * 0.5,
            ];
            let dx = q[0] - p[0];
            let dy = q[1] - p[1];
            let len = (dx * dx + dy * dy).sqrt().max(1.0);
            let seed = phase + time * 17.0 + level as f32 * 19.7 + index as f32 * 7.3;
            let offset = seed.sin() * amplitude;
            next.push([
                mid[0] - (dy / len) * offset,
                mid[1] + (dx / len) * offset,
                mid[2] + (seed * 1.37).sin() * amplitude * 0.12,
            ]);
        }
        if let Some(last) = points.last() {
            next.push(*last);
        }
        points = next;
        amplitude *= 0.5;
    }
    points
}

/// `_branchPath(points, span, phase)` (`:879-917`): a two-level forked accent
/// from the middle of the bolt.
pub fn branch_path(points: &[[f32; 3]], span: f32, phase: f32) -> Vec<[f32; 3]> {
    let Some(start) = points.get(points.len() * 55 / 100).copied() else {
        return Vec::new();
    };
    let angle = phase * 1.37;
    let end = [
        start[0] + angle.cos() * span * 0.24,
        start[1] + angle.sin() * span * 0.24,
        start[2] + (phase * 2.1).sin() * span * 0.08,
    ];
    let mut pts = vec![start, end];

    for level in 0..2 {
        let mut next: Vec<[f32; 3]> = Vec::with_capacity(pts.len() * 2);
        for index in 0..pts.len().saturating_sub(1) {
            let p = pts[index];
            let q = pts[index + 1];
            next.push(p);
            let mid = [
                (p[0] + q[0]) * 0.5,
                (p[1] + q[1]) * 0.5,
                (p[2] + q[2]) * 0.5,
            ];
            let dx = q[0] - p[0];
            let dy = q[1] - p[1];
            let len = (dx * dx + dy * dy).sqrt().max(1.0);
            let offset = ((level + 1) as f32 * 17.3 + phase).sin() * span * 0.06;
            next.push([
                mid[0] - (dy / len) * offset,
                mid[1] + (dx / len) * offset,
                mid[2] + ((level + 2) as f32 * 9.1 + phase).sin() * span * 0.02,
            ]);
        }
        if let Some(last) = pts.last() {
            next.push(*last);
        }
        pts = next;
    }
    pts
}

/// The dust material's colour/size/opacity (`:653-657`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DustLook {
    /// `material.color.setHSL(…).multiplyScalar(1.35)`.
    pub colour: [f32; 3],
    /// `material.size`.
    pub size: f32,
    /// `material.opacity`.
    pub opacity: f32,
}

/// `dustLook(time, b, flash)`.
pub fn dust_look(time: f32, bass: f32, flash: f32) -> DustLook {
    let hue = js_mod(0.58 + time * 0.025, 1.0);
    let colour = hsl_to_rgb(hue, 0.82, 0.72);
    DustLook {
        colour: [colour[0] * 1.35, colour[1] * 1.35, colour[2] * 1.35],
        size: 0.05 + bass * 0.025 + flash * 0.025,
        opacity: 0.58 + bass * 0.22 + flash * 0.16,
    }
}

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

/// `buildTunnelRing`'s silhouette as an extruded annulus band.
///
/// The JS rounds the polygon corners (`roundedPolygon`, radius
/// `min(0.24, thick * 0.7, outer * 0.08)`) and bevels the extrusion; this keeps
/// the same inner/outer radii and depth with straight polygon edges (see the
/// module docs). Normals are per-face, so the bands catch the key light like
/// the JS's `flatShading: true` material.
pub fn build_ring_band(sides: u32, outer: f32, thickness: f32, depth: f32) -> Mesh {
    let sides = sides.max(3);
    let inner = (outer - thickness).max(0.01);
    let half_depth = depth * 0.5;
    let mut mesh = Mesh::default();

    let point = |index: u32, radius: f32| {
        let angle = index as f32 * std::f32::consts::TAU / sides as f32;
        [angle.cos() * radius, angle.sin() * radius]
    };
    let push_quad = |mesh: &mut Mesh, quad: [[f32; 3]; 4], normal: [f32; 3]| {
        let base = mesh.vertices.len() as u32;
        for position in quad {
            mesh.vertices.push(Vertex {
                position,
                normal,
                uv: [0.0, 0.0],
            });
        }
        mesh.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    };

    for index in 0..sides {
        let next = (index + 1) % sides;
        let outer_a = point(index, outer);
        let outer_b = point(next, outer);
        let inner_a = point(index, inner);
        let inner_b = point(next, inner);

        // Front (+z) and back (-z) annulus quads.
        push_quad(
            &mut mesh,
            [
                [inner_a[0], inner_a[1], half_depth],
                [outer_a[0], outer_a[1], half_depth],
                [outer_b[0], outer_b[1], half_depth],
                [inner_b[0], inner_b[1], half_depth],
            ],
            [0.0, 0.0, 1.0],
        );
        push_quad(
            &mut mesh,
            [
                [inner_b[0], inner_b[1], -half_depth],
                [outer_b[0], outer_b[1], -half_depth],
                [outer_a[0], outer_a[1], -half_depth],
                [inner_a[0], inner_a[1], -half_depth],
            ],
            [0.0, 0.0, -1.0],
        );

        // Outer and inner rims.
        let mid = [
            (outer_a[0] + outer_b[0]) * 0.5,
            (outer_a[1] + outer_b[1]) * 0.5,
        ];
        let outer_normal = normalise([mid[0], mid[1], 0.0]);
        push_quad(
            &mut mesh,
            [
                [outer_a[0], outer_a[1], -half_depth],
                [outer_b[0], outer_b[1], -half_depth],
                [outer_b[0], outer_b[1], half_depth],
                [outer_a[0], outer_a[1], half_depth],
            ],
            outer_normal,
        );
        let mid = [
            (inner_a[0] + inner_b[0]) * 0.5,
            (inner_a[1] + inner_b[1]) * 0.5,
        ];
        let inner_normal = normalise([-mid[0], -mid[1], 0.0]);
        push_quad(
            &mut mesh,
            [
                [inner_b[0], inner_b[1], -half_depth],
                [inner_a[0], inner_a[1], -half_depth],
                [inner_a[0], inner_a[1], half_depth],
                [inner_b[0], inner_b[1], half_depth],
            ],
            inner_normal,
        );
    }

    mesh
}

/// A world-space ribbon (triangle list) along `points`, with the JS glow
/// geometry's XY-plane perpendicular (`:826-862`).
pub fn ribbon(
    points: &[[f32; 3]],
    half_width: f32,
    out: &mut Vec<BoltVertex>,
    colour: [f32; 3],
    alpha: f32,
) {
    for index in 0..points.len().saturating_sub(1) {
        let p = points[index];
        let q = points[index + 1];
        let dx = q[0] - p[0];
        let dy = q[1] - p[1];
        let len = (dx * dx + dy * dy).sqrt().max(1e-4);
        let ox = (-dy / len) * half_width;
        let oy = (dx / len) * half_width;
        let quad = [
            [p[0] - ox, p[1] - oy, p[2]],
            [p[0] + ox, p[1] + oy, p[2]],
            [q[0] - ox, q[1] - oy, q[2]],
            [q[0] - ox, q[1] - oy, q[2]],
            [q[0] + ox, q[1] + oy, q[2]],
            [p[0] + ox, p[1] + oy, p[2]],
        ];
        for position in quad {
            out.push(BoltVertex {
                position,
                colour,
                alpha,
            });
        }
    }
}

// ---------------------------------------------------------------------------
// GPU types
// ---------------------------------------------------------------------------

/// `GEOMETRIC_TUNNEL_PROPS` (`sceneProps.js:86-90`).
const GEOMETRIC_TUNNEL_PROPS: &[SceneProp] = &[
    SceneProp::float("scrollSpeed", "Scroll Speed", 1.0, 0.0, 4.0, 0.01),
    SceneProp::float("brightness", "Brightness", 1.0, 0.0, 3.0, 0.01),
    SceneProp::float("bloom", "Bloom", 1.0, 0.0, 3.0, 0.01),
];

/// The surface/geometry uniform block (192 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct TunnelUniforms {
    view_proj: Mat4,
    camera_pos: [f32; 4],
    /// rgb = fog colour, w = fog near.
    fog: [f32; 4],
    /// x = fog far, y = hemi, z = key, w = rim.
    lights: [f32; 4],
    /// x = l1 intensity, y = l2 intensity, z = target intensity, w = time.
    point_lights: [f32; 4],
    /// x = l1.x, y = l2.x, z = target.z.
    light_positions: [f32; 4],
    /// x = bolt light 0, y = bolt light 1.
    bolt_lights: [f32; 4],
    bolt_pos0: [f32; 4],
    bolt_pos1: [f32; 4],
}

/// One emissive geometry instance (96 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct EmissiveInstance {
    model: Mat4,
    /// Base metal tint (linear).
    colour: [f32; 4],
    /// rgb = emissive colour, w = emissive intensity.
    emissive: [f32; 4],
}

impl EmissiveInstance {
    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 0,
                    shader_location: 4,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 16,
                    shader_location: 5,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 32,
                    shader_location: 6,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 48,
                    shader_location: 7,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 64,
                    shader_location: 8,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 80,
                    shader_location: 9,
                },
            ],
        }
    }
}

/// One dust billboard (32 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct DustInstance {
    position: [f32; 3],
    size: f32,
    colour: [f32; 3],
    alpha: f32,
}

impl DustInstance {
    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 0,
                    shader_location: 4,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32,
                    offset: 12,
                    shader_location: 5,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 16,
                    shader_location: 6,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32,
                    offset: 28,
                    shader_location: 7,
                },
            ],
        }
    }
}

/// A bolt ribbon vertex (28 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct BoltVertex {
    /// World position.
    pub position: [f32; 3],
    /// Linear colour.
    pub colour: [f32; 3],
    /// Additive alpha.
    pub alpha: f32,
}

impl BoltVertex {
    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 0,
                    shader_location: 0,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 12,
                    shader_location: 1,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32,
                    offset: 24,
                    shader_location: 2,
                },
            ],
        }
    }
}

/// The billboard basis uniform block (96 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct BillboardUniforms {
    view_proj: Mat4,
    camera_right: [f32; 4],
    camera_up: [f32; 4],
}

/// The SSAO beauty-multiply uniform block (16 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct MixUniforms {
    resolution: [f32; 2],
    _pad: [f32; 2],
}

/// The final mix uniform block (16 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct FinalUniforms {
    resolution: [f32; 2],
    bloom_strength: f32,
    _pad: f32,
}

/// Which mesh a batch draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MeshKey {
    /// A ring band with `n` sides.
    Ring(u32),
    /// The unit box (braces and nodes).
    Box,
}

/// One bolt arc (`this.bolts[i]`).
#[derive(Clone, Copy, Debug)]
pub struct Bolt {
    /// `active`.
    pub active: bool,
    /// `life`.
    pub life: f32,
    /// `maxLife`.
    pub max_life: f32,
    /// `a` — (ring index, polygon corner).
    pub anchor_a: (usize, u32),
    /// `b` — (ring index, polygon corner).
    pub anchor_b: (usize, u32),
    /// `phase`.
    pub phase: f32,
    /// `hue`.
    pub hue: f32,
    /// `lightSlot`.
    pub light_slot: usize,
    /// `lightPos`.
    pub light_pos: [f32; 3],
    /// `lightStrength`.
    pub light_strength: f32,
}

impl Bolt {
    fn idle(index: usize) -> Self {
        Self {
            active: false,
            life: 0.0,
            max_life: 0.22,
            anchor_a: (0, 0),
            anchor_b: (0, 0),
            phase: 0.0,
            hue: 0.0,
            light_slot: index % 2,
            light_pos: [0.0; 3],
            light_strength: 0.0,
        }
    }
}

/// GPU resources built in `init`.
struct TunnelGpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    /// `grainMap`/`emissiveMap`/`roughnessMap` (`buildTunnelGrain`).
    _grain_texture: wgpu::Texture,
    /// `scene.environment` (`buildTunnelEnvironment`, equirectangular).
    _environment_texture: wgpu::Texture,
    _surface_sampler: wgpu::Sampler,
    _scene_texture: wgpu::Texture,
    scene_view: wgpu::TextureView,
    _lit_texture: wgpu::Texture,
    lit_view: wgpu::TextureView,
    _depth: wgpu::Texture,
    depth_view: wgpu::TextureView,
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    instance_bind_group: wgpu::BindGroup,
    instance_buffer: wgpu::Buffer,
    ring_meshes: Vec<(u32, MeshGpu)>,
    box_mesh: MeshGpu,
    bolt_pipeline: wgpu::RenderPipeline,
    bolt_uniforms: wgpu::Buffer,
    bolt_bind_group: wgpu::BindGroup,
    bolt_vertices: wgpu::Buffer,
    dust_pipeline: wgpu::RenderPipeline,
    dust_uniforms: wgpu::Buffer,
    dust_bind_group: wgpu::BindGroup,
    dust_instances: wgpu::Buffer,
    quad: UnitQuad,
    ssao: SsaoPass,
    mix_pipeline: wgpu::RenderPipeline,
    mix_uniforms: wgpu::Buffer,
    mix_layout: wgpu::BindGroupLayout,
    mix_bind_group: wgpu::BindGroup,
    mix_sampler: wgpu::Sampler,
    bloom: UnrealBloomPass,
    final_pipeline: wgpu::RenderPipeline,
    final_uniforms: wgpu::Buffer,
    final_layout: wgpu::BindGroupLayout,
    final_bind_group: wgpu::BindGroup,
    final_sampler: wgpu::Sampler,
    composer: Composer,
}

/// GeometricTunnel scene.
pub struct GeometricTunnel {
    /// `this.scroll`.
    scroll: f32,
    /// `this.smBass`.
    smoothed_bass: f32,
    /// `this.smMid`.
    smoothed_mid: f32,
    /// `this.flash`.
    flash: f32,
    /// `this.time` (`BaseScene.update` accumulates the frame delta).
    time: f32,
    /// `this.speedScale`.
    speed_scale: f32,
    /// `this.brightnessScale`.
    brightness_scale: f32,
    /// `this.bloomScale`.
    bloom_scale: f32,
    /// Per-ring sway seeds (`Math.random() * TAU` in the JS).
    sway: [f32; RING_COUNT],
    sway_axis: [f32; RING_COUNT],
    sway_amp: [f32; RING_COUNT],
    /// The bolt pool.
    bolts: [Bolt; BOLT_COUNT],
    /// The seed stream behind `Math.random` for bolts and dust-free randomness.
    seed: u32,
    /// The last `turn` (the scroll velocity) — `this.turn`.
    turn: f32,
    camera_aspect: f32,
    size: (u32, u32),
    gpu: Option<TunnelGpu>,
}

impl Default for GeometricTunnel {
    fn default() -> Self {
        Self::new()
    }
}

impl GeometricTunnel {
    /// Create the scene (`GeometricTunnel.js:210-226` defaults).
    pub fn new() -> Self {
        let mut sway = [0.0; RING_COUNT];
        let mut sway_axis = [0.0; RING_COUNT];
        let mut sway_amp = [0.0; RING_COUNT];
        for index in 0..RING_COUNT {
            sway[index] = seeded_rand(index as i32 * 3 + 1, 11) * std::f32::consts::TAU;
            sway_axis[index] = seeded_rand(index as i32 * 5 + 2, 12) * std::f32::consts::TAU;
            sway_amp[index] = 0.6 + seeded_rand(index as i32 * 7 + 3, 13) * 0.9;
        }
        let mut bolts = [Bolt::idle(0); BOLT_COUNT];
        for (index, bolt) in bolts.iter_mut().enumerate() {
            *bolt = Bolt::idle(index);
        }
        Self {
            scroll: 0.0,
            smoothed_bass: 0.0,
            smoothed_mid: 0.0,
            flash: 0.0,
            time: 0.0,
            speed_scale: 1.0,
            brightness_scale: 1.0,
            bloom_scale: 1.0,
            sway,
            sway_axis,
            sway_amp,
            bolts,
            seed: 0,
            turn: 0.0,
            camera_aspect: 1.0,
            size: (1, 1),
            gpu: None,
        }
    }

    /// Apply the resolved modV props (`sceneProps.js:86-90`).
    pub fn apply_props(&mut self, props: impl FnMut(&str) -> Option<f32>) {
        for (key, value) in read_scene_props(GEOMETRIC_TUNNEL_PROPS, props) {
            match key {
                "scrollSpeed" => self.speed_scale = value,
                "brightness" => self.brightness_scale = value,
                "bloom" => self.bloom_scale = value,
                _ => {}
            }
        }
    }

    /// `this.scroll`.
    pub fn scroll(&self) -> f32 {
        self.scroll
    }

    /// `this.smBass`.
    pub fn smoothed_bass(&self) -> f32 {
        self.smoothed_bass
    }

    /// `this.flash`.
    pub fn flash(&self) -> f32 {
        self.flash
    }

    /// The bolt pool (`this.bolts`).
    pub fn bolts(&self) -> &[Bolt; BOLT_COUNT] {
        &self.bolts
    }

    /// `this.turn` — the scroll velocity of the last frame.
    pub fn turn(&self) -> f32 {
        self.turn
    }

    /// The destination alpha the scene clears to.
    pub fn clear_alpha(&self) -> f64 {
        CLEAR_ALPHA
    }

    /// The camera position for this frame (`:997-1001`).
    pub fn camera_position(&self) -> [f32; 3] {
        [
            (self.time * 0.15).sin() * 0.3,
            (self.time * 0.12).cos() * 0.2,
            3.0,
        ]
    }

    /// The camera roll `camera.rotateZ(...)` (`:1003-1005`).
    pub fn camera_roll(&self) -> f32 {
        (self.time * 0.17).sin() * 0.025 + (self.time * 0.07).sin() * 0.01
    }

    /// `camera.fov = 64 + smBass * 3`.
    pub fn camera_fov(&self) -> f32 {
        CAMERA_FOV + self.smoothed_bass * 3.0
    }

    /// The bloom strength (`:668-673`).
    pub fn bloom_strength(&self) -> f32 {
        (0.68 + self.smoothed_bass * 0.28 + self.flash * 0.22) * self.bloom_scale
    }

    /// The bloom mix strength (`bloomMixPass.material.uniforms.bloomStrength`).
    pub fn bloom_mix_strength(&self) -> f32 {
        (0.55 + self.smoothed_bass * 0.14 + self.flash * 0.12) * self.bloom_scale
    }

    fn view_proj(&self) -> Mat4 {
        let view = mat4_mul(
            &rotation_z(-self.camera_roll()),
            &look_at(
                self.camera_position(),
                [0.0, 0.0, -TUNNEL_LEN * 0.5],
                [0.0, 1.0, 0.0],
            ),
        );
        mat4_mul(
            &perspective(
                self.camera_fov().to_radians(),
                self.camera_aspect,
                CAMERA_NEAR,
                CAMERA_FAR,
            ),
            &view,
        )
    }

    fn group_model(pose: &RingPose) -> Mat4 {
        let mut model = translation(pose.position[0], pose.position[1], pose.position[2]);
        model = mat4_mul(&model, &rotation_z(pose.rotation_z));
        mat4_mul(&model, &scale(pose.scale, pose.scale, pose.depth_scale))
    }

    /// The light intensities (`:659-667`).
    fn light_intensities(&self) -> [f32; 4] {
        let b = self.smoothed_bass;
        let light_pulse = 0.6 + b * 0.9 + self.flash * 1.6;
        [
            0.34 + b * 0.14 + self.flash * 0.1,
            0.9 + light_pulse * 0.34,
            0.4 + b * 0.35 + self.flash * 0.25,
            0.7 + light_pulse * 0.45,
        ]
    }

    /// `_spawnBolt` (`:758-794`), using the deterministic seed stream.
    fn spawn_bolt(&mut self, ring_z: [f32; RING_COUNT]) {
        let Some(slot) = self.bolts.iter().position(|bolt| !bolt.active) else {
            return;
        };
        let mut order: Vec<usize> = (0..RING_COUNT).collect();
        order.sort_by(|a, b| {
            ring_z[*b]
                .partial_cmp(&ring_z[*a])
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let rand = |offset: u32| seeded_rand(self.seed as i32, offset as i32);
        let ring_a = order[(rand(0) * 6.0).min(5.0) as usize];
        let span = 6usize.min(order.len().saturating_sub(2));
        let ring_b = order[2 + (rand(1) * span as f32) as usize];

        let bolt = &mut self.bolts[slot];
        bolt.anchor_a = (ring_a, (rand(2) * ring_sides(ring_a, false) as f32) as u32);
        bolt.anchor_b = (ring_b, (rand(3) * ring_sides(ring_b, false) as f32) as u32);
        bolt.life = 0.0;
        bolt.max_life = 0.22 + rand(4) * 0.18;
        bolt.phase = rand(5) * 100.0;
        bolt.hue = rand(6);
        bolt.active = true;
        bolt.light_strength = 0.0;
        self.seed = self.seed.wrapping_add(1);
    }

    /// The world-space ring corner a bolt anchors to (`_cornerWorld`).
    fn corner_world(&self, ring: usize, corner: u32) -> [f32; 3] {
        let sides = ring_sides(ring, false);
        let pose = self.current_pose(ring);
        let model = Self::group_model(&pose);
        // `ring.frame.rotation.z` is `((i % 3) - 1) * PI / sides`; the group's
        // own rotation carries the per-frame spin.
        let frame_rotation = ((ring % 3) as f32 - 1.0) * std::f32::consts::PI / sides as f32;
        let angle = frame_rotation + corner as f32 * std::f32::consts::TAU / sides as f32;
        let local = [angle.cos() * RADIUS, angle.sin() * RADIUS, 0.0];
        let world = transform_point(&model, local);
        [world[0], world[1], world[2]]
    }

    /// The ring pose cached for this frame (recomputed on demand).
    fn current_pose(&self, index: usize) -> RingPose {
        ring_pose(
            index,
            self.scroll,
            self.time,
            self.smoothed_bass,
            self.flash,
            self.sway[index],
            self.sway_axis[index],
            self.sway_amp[index],
        )
    }

    /// Build the frame's emissive instance batches.
    fn collect_batches(&self) -> Vec<(MeshKey, Vec<EmissiveInstance>)> {
        let mut batches: Vec<(MeshKey, Vec<EmissiveInstance>)> = Vec::new();
        let push = |key: MeshKey,
                    instance: EmissiveInstance,
                    batches: &mut Vec<(MeshKey, Vec<EmissiveInstance>)>| {
            if let Some((_, list)) = batches.iter_mut().find(|(existing, _)| *existing == key) {
                list.push(instance);
            } else {
                batches.push((key, vec![instance]));
            }
        };

        let b = self.smoothed_bass;
        let m = self.smoothed_mid;
        // `MeshStandardMaterial` colour management converts the sRGB tint to
        // linear; using the raw 0x3a/0x43/0x58 components made the hoops ~5x
        // too bright and washed the grain/environment detail out.
        let base_tint = TUNNEL_BASE_TINT;

        for index in 0..RING_COUNT {
            let pose = self.current_pose(index);
            let outer_sides = ring_sides(index, false);
            let inner_sides = ring_sides(index, true);
            let look = ring_colour(
                index,
                self.time,
                m,
                self.flash,
                pose.depth,
                b,
                pose.near,
                self.brightness_scale,
            );
            let group = Self::group_model(&pose);

            let frame_rotation =
                ((index % 3) as f32 - 1.0) * std::f32::consts::PI / outer_sides as f32;
            // The shared band mesh is unit radius/depth; the JS frame geometry
            // is authored at the tunnel radius (4.2) with TUNNEL_HOOP_DEPTH.
            // Without this scale the hoops are 4.2× too small and ~20× too
            // deep (they read as solid washers).
            let frame_model = mat4_mul(
                &group,
                &mat4_mul(
                    &rotation_z(frame_rotation),
                    &scale(RADIUS, RADIUS, TUNNEL_HOOP_DEPTH),
                ),
            );
            push(
                MeshKey::Ring(outer_sides),
                EmissiveInstance {
                    model: frame_model,
                    colour: [base_tint[0], base_tint[1], base_tint[2], 1.0],
                    emissive: [
                        look.colour[0] * 0.5,
                        look.colour[1] * 0.5,
                        look.colour[2] * 0.5,
                        look.frame_intensity,
                    ],
                },
                &mut batches,
            );

            let inner_rotation =
                std::f32::consts::PI / inner_sides as f32 + (index % 2) as f32 * 0.16;
            let inner_model = mat4_mul(
                &group,
                &mat4_mul(
                    &rotation_z(inner_rotation),
                    &scale(RADIUS * 0.42, RADIUS * 0.42, 0.035),
                ),
            );
            push(
                MeshKey::Ring(inner_sides),
                EmissiveInstance {
                    model: inner_model,
                    colour: [base_tint[0], base_tint[1], base_tint[2], 1.0],
                    emissive: [
                        look.colour[0] * 0.42,
                        look.colour[1] * 0.42,
                        look.colour[2] * 0.42,
                        look.inner_intensity,
                    ],
                },
                &mut batches,
            );

            // Braces and nodes (`:258-282`).
            let brace_count = if index % 2 == 1 { 3 } else { 4 };
            let inner_r = RADIUS * 0.42 + 0.18;
            let outer_r = RADIUS - 0.34;
            for k in 0..brace_count {
                let angle = k as f32 * std::f32::consts::TAU / brace_count as f32
                    + std::f32::consts::PI / outer_sides as f32;
                let brace_model = mat4_mul(
                    &group,
                    &mat4_mul(
                        &translation(
                            angle.cos() * (inner_r + outer_r) * 0.5,
                            angle.sin() * (inner_r + outer_r) * 0.5,
                            0.0,
                        ),
                        &mat4_mul(
                            &rotation_z(angle),
                            &scale((outer_r - inner_r) * 0.5, 0.035, 0.08),
                        ),
                    ),
                );
                push(
                    MeshKey::Box,
                    EmissiveInstance {
                        model: brace_model,
                        colour: [base_tint[0], base_tint[1], base_tint[2], 1.0],
                        emissive: [
                            look.colour[0] * 0.5,
                            look.colour[1] * 0.5,
                            look.colour[2] * 0.5,
                            look.frame_intensity,
                        ],
                    },
                    &mut batches,
                );

                let node_pulse = 1.0
                    + b * 0.65
                    + self.flash * 0.4
                    + (self.time * 2.2 + k as f32 * 1.7 + index as f32).sin() * 0.08;
                let node_model = mat4_mul(
                    &group,
                    &mat4_mul(
                        &translation(angle.cos() * inner_r, angle.sin() * inner_r, 0.0),
                        &trs(
                            [0.0; 3],
                            [
                                self.time * 0.7 + k as f32,
                                self.time * 0.9 + k as f32 * 0.4,
                                0.0,
                            ],
                            [node_pulse; 3],
                        ),
                    ),
                );
                push(
                    MeshKey::Box,
                    EmissiveInstance {
                        model: mat4_mul(&node_model, &scale(0.11, 0.11, 0.11)),
                        colour: [base_tint[0], base_tint[1], base_tint[2], 1.0],
                        emissive: [
                            look.colour[0] * 0.5,
                            look.colour[1] * 0.5,
                            look.colour[2] * 0.5,
                            look.frame_intensity,
                        ],
                    },
                    &mut batches,
                );
            }

            // `ros` tile clusters on every third ring.
            if index % 3 == 0 {
                let swell = 1.0 + b * 0.35 + self.flash * 0.1;
                let ros_model = mat4_mul(
                    &group,
                    &mat4_mul(
                        &rotation_z(
                            (if index % 2 == 1 { -1.0 } else { 1.0 })
                                * (self.time * 0.7 + index as f32 * 1.7).sin()
                                * 0.35,
                        ),
                        &scale(swell, swell, 0.9 + b * 0.2),
                    ),
                );
                let cluster = RADIUS * 0.52;
                for k in 0..outer_sides {
                    let angle = k as f32 * std::f32::consts::TAU / outer_sides as f32;
                    let tile_model = mat4_mul(
                        &ros_model,
                        &mat4_mul(
                            &translation(angle.cos() * cluster, angle.sin() * cluster, 0.0),
                            &mat4_mul(
                                &rotation_z(angle + std::f32::consts::PI / inner_sides as f32),
                                &translation(0.0, 0.0, 0.0),
                            ),
                        ),
                    );
                    push(
                        MeshKey::Ring(inner_sides),
                        EmissiveInstance {
                            model: mat4_mul(
                                &tile_model,
                                &scale(RADIUS * 0.16, RADIUS * 0.16, 0.02),
                            ),
                            colour: [base_tint[0], base_tint[1], base_tint[2], 1.0],
                            emissive: [
                                look.colour[0] * 0.85,
                                look.colour[1] * 0.85,
                                look.colour[2] * 0.85,
                                look.ros_intensity,
                            ],
                        },
                        &mut batches,
                    );
                }
            }
        }

        // The far target ring (`:471-488`, `:745-755`).
        let target_colour = hsl_to_rgb(js_mod(0.55 + self.time * 0.02, 1.0), 1.0, 0.38);
        let s = 1.0 + b * 0.12 + self.flash * 0.08;
        let target_model = mat4_mul(
            &translation(0.0, 0.0, -TUNNEL_LEN - 2.0),
            &mat4_mul(&rotation_z(self.time * 0.25), &scale(s, s, 0.86 + b * 0.18)),
        );
        push(
            MeshKey::Ring(32),
            EmissiveInstance {
                model: mat4_mul(&target_model, &scale(RADIUS * 0.82, RADIUS * 0.82, 0.045)),
                colour: [base_tint[0], base_tint[1], base_tint[2], 1.0],
                emissive: [
                    target_colour[0] * 1.9,
                    target_colour[1] * 1.9,
                    target_colour[2] * 1.9,
                    0.46 + b * 0.2 + self.flash * 0.35,
                ],
            },
            &mut batches,
        );
        push(
            MeshKey::Ring(4),
            EmissiveInstance {
                model: mat4_mul(
                    &target_model,
                    &mat4_mul(
                        &rotation_z(std::f32::consts::PI / 10.0 - self.time * 0.2),
                        &scale(RADIUS * 0.34, RADIUS * 0.34, 0.03),
                    ),
                ),
                colour: [base_tint[0], base_tint[1], base_tint[2], 1.0],
                emissive: [
                    target_colour[0] * 1.55,
                    target_colour[1] * 1.55,
                    target_colour[2] * 1.55,
                    0.34 + b * 0.12 + self.flash * 0.24,
                ],
            },
            &mut batches,
        );

        batches
    }

    /// Build the frame's dust billboards.
    fn dust_instances(&self) -> Vec<DustInstance> {
        let look = dust_look(self.time, self.smoothed_bass, self.flash);
        let rotation_z = self.time * 0.035;
        let rotation_x = (self.time * 0.13).sin() * 0.04;
        let offset_z = 6.0 - js_mod(self.scroll, TUNNEL_LEN);
        let (cos_z, sin_z) = rotation_z.sin_cos();
        let (cos_x, sin_x) = rotation_x.sin_cos();

        (0..DUST_COUNT)
            .map(|index| {
                let p = dust_seed(index);
                let x = p[0] * cos_z - p[1] * sin_z;
                let y = p[0] * sin_z + p[1] * cos_z;
                let z = p[2];
                let y = y * cos_x - z * sin_x;
                let z = y * sin_x + z * cos_x;
                DustInstance {
                    position: [x, y, z + offset_z],
                    size: look.size * 4.0,
                    colour: look.colour,
                    alpha: look.opacity,
                }
            })
            .collect()
    }

    /// Build the bolt ribbons for this frame (`_updateBolt`, `:796-877`).
    fn bolt_vertices(&self, ring_z: &[f32; RING_COUNT]) -> Vec<BoltVertex> {
        let mut vertices: Vec<BoltVertex> = Vec::new();

        for bolt in &self.bolts {
            if !bolt.active {
                continue;
            }
            let t = bolt.life / bolt.max_life.max(1e-4);
            let a = self.corner_world(bolt.anchor_a.0, bolt.anchor_a.1);
            let b = self.corner_world(bolt.anchor_b.0, bolt.anchor_b.1);
            let distance =
                ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
            let points = bolt_path(a, b, distance * 0.14, bolt.phase, self.time);
            let branch = branch_path(&points, distance, bolt.phase);

            let flicker_seed = seeded_rand(self.seed as i32, (bolt.life * 1000.0) as i32);
            let flick = ((1.0 - t)
                * (0.72 + (self.time * 72.0 + bolt.phase).sin() * 0.16 + flicker_seed * 0.12)
                * (0.85 + self.flash * 0.45))
                .min(1.0);

            let core = hsl_to_rgb(0.54 + bolt.hue * 0.08, 0.72, 0.82);
            let glow = hsl_to_rgb(0.6 + bolt.hue * 0.08, 0.9, 0.56);
            let accent = hsl_to_rgb(0.88 + bolt.hue * 0.08, 0.9, 0.62);
            let _ = ring_z;
            ribbon(
                &points,
                0.14,
                &mut vertices,
                [glow[0] * 0.8, glow[1] * 0.8, glow[2] * 0.8],
                flick * 0.78,
            );
            ribbon(
                &points,
                0.035,
                &mut vertices,
                [core[0] * 1.05, core[1] * 1.05, core[2] * 1.05],
                (flick * 1.2).min(1.0),
            );
            if branch.len() > 1 {
                ribbon(
                    &branch,
                    0.06,
                    &mut vertices,
                    [accent[0] * 0.9, accent[1] * 0.9, accent[2] * 0.9],
                    flick * 0.65,
                );
            }
        }

        vertices.truncate(MAX_BOLT_VERTICES);
        vertices
    }

    fn upload_uniforms(&self) {
        let Some(gpu) = self.gpu.as_ref() else {
            return;
        };
        let fog = [
            ((FOG_HEX >> 16) & 0xff) as f32 / 255.0,
            ((FOG_HEX >> 8) & 0xff) as f32 / 255.0,
            (FOG_HEX & 0xff) as f32 / 255.0,
            FOG_NEAR,
        ];
        let lights = self.light_intensities();
        let camera = self.camera_position();
        let b = self.smoothed_bass;
        let mut bolt_lights = [0.0_f32; 4];
        let mut bolt_pos0 = [0.0_f32; 4];
        let mut bolt_pos1 = [0.0_f32; 4];
        for bolt in &self.bolts {
            if !bolt.active {
                continue;
            }
            let slot = bolt.light_slot.min(1);
            let target = if slot == 0 {
                &mut bolt_pos0
            } else {
                &mut bolt_pos1
            };
            if bolt.light_strength > bolt_lights[slot] {
                bolt_lights[slot] = bolt.light_strength;
                *target = [bolt.light_pos[0], bolt.light_pos[1], bolt.light_pos[2], 1.0];
            }
        }

        gpu.queue.write_buffer(
            &gpu.uniforms,
            0,
            bytemuck::bytes_of(&TunnelUniforms {
                view_proj: self.view_proj(),
                camera_pos: [camera[0], camera[1], camera[2], 1.0],
                fog,
                lights: [FOG_FAR, lights[0], lights[1], lights[2]],
                point_lights: [
                    lights[3],
                    0.45 + (0.6 + b * 0.9 + self.flash * 1.6) * 0.3,
                    1.25 + b * 0.9 + self.flash * 1.2,
                    self.time,
                ],
                light_positions: [
                    (self.time * 0.14).sin() * 1.5,
                    (self.time * 0.2).cos() * 1.5,
                    -TUNNEL_LEN - 2.0,
                    1.0,
                ],
                bolt_lights,
                bolt_pos0,
                bolt_pos1,
            }),
        );

        let forward = normalise([-camera[0], -camera[1], -camera[2]]);
        let right = normalise([-forward[2], 0.0, forward[0]]);
        let up = [
            right[1] * forward[2] - right[2] * forward[1],
            right[2] * forward[0] - right[0] * forward[2],
            right[0] * forward[1] - right[1] * forward[0],
        ];
        let view_proj = self.view_proj();
        let billboard = BillboardUniforms {
            view_proj,
            camera_right: [right[0], right[1], right[2], 0.0],
            camera_up: [up[0], up[1], up[2], 0.0],
        };
        gpu.queue
            .write_buffer(&gpu.bolt_uniforms, 0, bytemuck::bytes_of(&billboard));
        gpu.queue
            .write_buffer(&gpu.dust_uniforms, 0, bytemuck::bytes_of(&billboard));
    }

    fn rebuild_targets(&mut self) {
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };
        let (scene_texture, scene_view) = crate::point10::postfx::create_target(
            &gpu.device,
            "point10-tunnel-scene",
            self.size.0,
            self.size.1,
        );
        let (lit_texture, lit_view) = crate::point10::postfx::create_target(
            &gpu.device,
            "point10-tunnel-lit",
            self.size.0,
            self.size.1,
        );
        let (depth, depth_view) = create_sampled_depth(
            &gpu.device,
            "point10-tunnel-depth",
            self.size.0,
            self.size.1,
        );
        gpu._scene_texture = scene_texture;
        gpu.scene_view = scene_view;
        gpu._lit_texture = lit_texture;
        gpu.lit_view = lit_view;
        gpu._depth = depth;
        gpu.depth_view = depth_view;
        gpu.ssao.resize(&gpu.device, self.size.0, self.size.1);
        gpu.ssao.set_depth(&gpu.device, &gpu.depth_view);
        gpu.bloom.resize(self.size.0, self.size.1);
        gpu.bloom.set_input(&gpu.lit_view);
        gpu.composer.resize(self.size.0, self.size.1);
        gpu.queue.write_buffer(
            &gpu.mix_uniforms,
            0,
            bytemuck::bytes_of(&MixUniforms {
                resolution: [self.size.0 as f32, self.size.1 as f32],
                _pad: [0.0; 2],
            }),
        );
        gpu.mix_bind_group = create_mix_bind_group(&gpu.device, gpu);
        gpu.final_bind_group = create_final_bind_group(&gpu.device, gpu);
        self.upload_uniforms();
    }
}

/// A depth texture that can be sampled by the SSAO pass.
fn create_sampled_depth(
    device: &wgpu::Device,
    label: &str,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

impl Point10Scene for GeometricTunnel {
    fn name(&self) -> &'static str {
        "Point10 Geometric Tunnel"
    }

    fn init(&mut self, ctx: &mut SceneContext) {
        let device = ctx.device.clone();
        let queue = ctx.queue.clone();
        self.size = (ctx.width.max(1), ctx.height.max(1));
        self.camera_aspect = ctx.aspect();

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-tunnel-shader"),
            source: wgpu::ShaderSource::Wgsl(tunnel_wgsl().into()),
        });
        // `GeometricTunnel._buildSurfaceMaps()`: the brushed-metal grain is
        // both the roughness and emissive map, and the equirect studio map is
        // `scene.environment`. Without these the hoops render as flat paint.
        let grain_texture = create_surface_texture(
            &device,
            &queue,
            "point10-tunnel-grain",
            TUNNEL_GRAIN_SIZE,
            TUNNEL_GRAIN_SIZE,
            &build_tunnel_grain(TUNNEL_GRAIN_SIZE),
            wgpu::TextureFormat::Rgba8UnormSrgb,
        );
        let environment_texture = create_surface_texture(
            &device,
            &queue,
            "point10-tunnel-environment",
            TUNNEL_ENV_WIDTH,
            TUNNEL_ENV_HEIGHT,
            &build_tunnel_environment(TUNNEL_ENV_WIDTH, TUNNEL_ENV_HEIGHT),
            wgpu::TextureFormat::Rgba8UnormSrgb,
        );
        let grain_view = grain_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let environment_view =
            environment_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let surface_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("point10-tunnel-surface-sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-tunnel-layout"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::VERTEX_FRAGMENT),
                texture_entry(1),
                sampler_entry(2),
                texture_entry(3),
            ],
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-tunnel-uniforms"),
            size: std::mem::size_of::<TunnelUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("point10-tunnel-pipeline-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("point10-tunnel-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(Vertex::layout()), Some(EmissiveInstance::layout())],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(depth_state()),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let instance_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point10-tunnel-bind-group"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&grain_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&surface_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&environment_view),
                },
            ],
        });
        let instance_buffer = {
            use wgpu::util::DeviceExt;
            let empty = vec![
                EmissiveInstance {
                    model: [[0.0; 4]; 4],
                    colour: [0.0; 4],
                    emissive: [0.0; 4],
                };
                MAX_INSTANCES
            ];
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("point10-tunnel-instances"),
                contents: bytemuck::cast_slice(&empty),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            })
        };

        // One band mesh per distinct polygon side count.
        let mut side_counts: Vec<u32> = OUTER_SIDES
            .iter()
            .chain(INNER_SIDES.iter())
            .copied()
            .collect();
        side_counts.sort_unstable();
        side_counts.dedup();
        let ring_meshes = side_counts
            .iter()
            .map(|sides| {
                // A unit-radius, unit-depth band scaled per instance; the
                // band's 0.08 thickness ratio gives the JS's 0.34 outer band at
                // RADIUS (see the module docs for the inner/target ratios).
                let mesh = build_ring_band(*sides, 1.0, 0.08, 1.0);
                (
                    *sides,
                    MeshGpu::new(&device, &format!("point10-tunnel-ring-{sides}"), &mesh),
                )
            })
            .collect();
        let box_mesh = MeshGpu::new(
            &device,
            "point10-tunnel-box",
            &crate::point10::scenes::cube_mesh::build_box(2.0, 2.0, 2.0),
        );

        // --- lightning ribbons ---------------------------------------------
        let bolt_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-tunnel-bolt-shader"),
            source: wgpu::ShaderSource::Wgsl(bolt_wgsl().into()),
        });
        let bolt_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-tunnel-bolt-layout"),
            entries: &[uniform_entry(0, wgpu::ShaderStages::VERTEX_FRAGMENT)],
        });
        let bolt_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-tunnel-bolt-uniforms"),
            size: std::mem::size_of::<BillboardUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bolt_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point10-tunnel-bolt-bind-group"),
            layout: &bolt_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: bolt_uniforms.as_entire_binding(),
            }],
        });
        let bolt_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("point10-tunnel-bolt-pipeline"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("point10-tunnel-bolt-pipeline-layout"),
                    bind_group_layouts: &[Some(&bolt_layout)],
                    immediate_size: 0,
                }),
            ),
            vertex: wgpu::VertexState {
                module: &bolt_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(BoltVertex::layout())],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &bolt_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR_FORMAT,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::SrcAlpha,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::Zero,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let bolt_vertices = {
            use wgpu::util::DeviceExt;
            let empty = vec![
                BoltVertex {
                    position: [0.0; 3],
                    colour: [0.0; 3],
                    alpha: 0.0,
                };
                MAX_BOLT_VERTICES
            ];
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("point10-tunnel-bolt-vertices"),
                contents: bytemuck::cast_slice(&empty),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            })
        };

        // --- dust ----------------------------------------------------------
        let dust_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-tunnel-dust-shader"),
            source: wgpu::ShaderSource::Wgsl(dust_wgsl().into()),
        });
        let dust_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-tunnel-dust-layout"),
            entries: &[uniform_entry(0, wgpu::ShaderStages::VERTEX)],
        });
        let dust_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-tunnel-dust-uniforms"),
            size: std::mem::size_of::<BillboardUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let dust_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point10-tunnel-dust-bind-group"),
            layout: &dust_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: dust_uniforms.as_entire_binding(),
            }],
        });
        let quad = UnitQuad::new(&device);
        let dust_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("point10-tunnel-dust-pipeline"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("point10-tunnel-dust-pipeline-layout"),
                    bind_group_layouts: &[Some(&dust_layout)],
                    immediate_size: 0,
                }),
            ),
            vertex: wgpu::VertexState {
                module: &dust_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[
                    Some(UnitQuad::vertex_layout()),
                    Some(DustInstance::layout()),
                ],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &dust_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR_FORMAT,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::SrcAlpha,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::Zero,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let dust_instances = {
            use wgpu::util::DeviceExt;
            let empty = vec![
                DustInstance {
                    position: [0.0; 3],
                    size: 0.0,
                    colour: [0.0; 3],
                    alpha: 0.0,
                };
                DUST_COUNT
            ];
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("point10-tunnel-dust-instances"),
                contents: bytemuck::cast_slice(&empty),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            })
        };

        // --- targets, SSAO, bloom, final composite --------------------------
        let (scene_texture, scene_view) = crate::point10::postfx::create_target(
            &device,
            "point10-tunnel-scene",
            self.size.0,
            self.size.1,
        );
        let (lit_texture, lit_view) = crate::point10::postfx::create_target(
            &device,
            "point10-tunnel-lit",
            self.size.0,
            self.size.1,
        );
        let (depth, depth_view) =
            create_sampled_depth(&device, "point10-tunnel-depth", self.size.0, self.size.1);

        let mut ssao = SsaoPass::new(&device, &queue, self.size.0, self.size.1);
        ssao.set_depth(&device, &depth_view);
        ssao.set_clip_planes(CAMERA_NEAR, CAMERA_FAR);
        ssao.set_params(TUNNEL_SSAO);

        let mut bloom = UnrealBloomPass::new(&device, &queue, self.size.0, self.size.1);
        bloom.set_strength(BLOOM_STRENGTH);
        bloom.set_radius(BLOOM_RADIUS);
        bloom.set_threshold(BLOOM_THRESHOLD);
        bloom.set_input(&lit_view);

        let final_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-tunnel-final-shader"),
            source: wgpu::ShaderSource::Wgsl(final_wgsl().into()),
        });
        let final_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-tunnel-final-layout"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::FRAGMENT),
                texture_entry(1),
                texture_entry(2),
                sampler_entry(3),
            ],
        });
        let final_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-tunnel-final-uniforms"),
            size: std::mem::size_of::<FinalUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let final_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("point10-tunnel-final-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let final_pipeline = ctx.fullscreen_pipeline_with_format(
            "point10-tunnel-final-pipeline",
            &final_shader,
            &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("point10-tunnel-final-pipeline-layout"),
                bind_group_layouts: &[Some(&final_layout)],
                immediate_size: 0,
            }),
            HDR_FORMAT,
            None,
        );
        let final_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point10-tunnel-final-bind-group"),
            layout: &final_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: final_uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&lit_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(
                        bloom
                            .composite_view()
                            .expect("the bloom pass always has a composite target"),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&final_sampler),
                },
            ],
        });

        // --- SSAO beauty multiply ------------------------------------------
        let mix_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-tunnel-mix-shader"),
            source: wgpu::ShaderSource::Wgsl(mix_wgsl().into()),
        });
        let mix_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-tunnel-mix-layout"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::FRAGMENT),
                texture_entry(1),
                texture_entry(2),
                sampler_entry(3),
            ],
        });
        let mix_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-tunnel-mix-uniforms"),
            size: std::mem::size_of::<MixUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(
            &mix_uniforms,
            0,
            bytemuck::bytes_of(&MixUniforms {
                resolution: [self.size.0 as f32, self.size.1 as f32],
                _pad: [0.0; 2],
            }),
        );
        let mix_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("point10-tunnel-mix-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let mix_pipeline = ctx.fullscreen_pipeline_with_format(
            "point10-tunnel-mix-pipeline",
            &mix_shader,
            &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("point10-tunnel-mix-pipeline-layout"),
                bind_group_layouts: &[Some(&mix_layout)],
                immediate_size: 0,
            }),
            HDR_FORMAT,
            None,
        );
        let mix_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point10-tunnel-mix-bind-group"),
            layout: &mix_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: mix_uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&scene_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(ssao.output_view()),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&mix_sampler),
                },
            ],
        });

        // `renderer.toneMapping = ACESFilmicToneMapping; exposure = 1`.
        let mut composer =
            Composer::new(&device, &queue, self.size.0, self.size.1, ctx.target_format);
        composer
            .output_mut()
            .set_tone_mapping(ToneMapping::AcesFilmic);
        composer.output_mut().set_exposure(1.0);

        self.gpu = Some(TunnelGpu {
            device,
            queue,
            _grain_texture: grain_texture,
            _environment_texture: environment_texture,
            _surface_sampler: surface_sampler,
            _scene_texture: scene_texture,
            scene_view,
            _lit_texture: lit_texture,
            lit_view,
            _depth: depth,
            depth_view,
            pipeline,
            uniforms,
            instance_bind_group,
            instance_buffer,
            ring_meshes,
            box_mesh,
            bolt_pipeline,
            bolt_uniforms,
            bolt_bind_group,
            bolt_vertices,
            dust_pipeline,
            dust_uniforms,
            dust_bind_group,
            dust_instances,
            quad,
            ssao,
            mix_pipeline,
            mix_uniforms,
            mix_layout,
            mix_bind_group,
            mix_sampler,
            bloom,
            final_pipeline,
            final_uniforms,
            final_layout,
            final_bind_group,
            final_sampler,
            composer,
        });
        self.upload_uniforms();
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.size = (width.max(1), height.max(1));
        self.camera_aspect = self.size.0 as f32 / self.size.1 as f32;
        self.rebuild_targets();
    }

    fn update(&mut self, dt: f32, audio: &AudioBridge) {
        let dt = frame_dt(dt);
        self.time += dt;
        let (bass, mid, kick) = if audio.is_available() {
            (audio.bands[0], audio.bands[1], audio.beat_env)
        } else {
            (0.0, 0.0, 0.0)
        };

        let drive = advance_tunnel(
            self.scroll,
            self.smoothed_bass,
            self.smoothed_mid,
            self.flash,
            dt,
            bass,
            mid,
            kick,
            self.speed_scale,
        );
        self.scroll = drive.scroll;
        self.smoothed_bass = drive.smoothed_bass;
        self.smoothed_mid = drive.smoothed_mid;
        self.flash = drive.flash;
        self.turn = drive.turn;

        // Lightning spawning (`:740-743`).
        let energy = 1.0_f32.min(self.smoothed_bass * 1.2 + self.smoothed_mid * 0.8);
        let sample = seeded_rand(self.seed as i32, (self.time * 1000.0) as i32);
        if sample < (0.5 + energy * 2.5) * dt {
            let mut ring_z = [0.0_f32; RING_COUNT];
            for (index, z) in ring_z.iter_mut().enumerate() {
                *z = self.current_pose(index).position[2];
            }
            self.spawn_bolt(ring_z);
        }

        // Advance the bolts (`_updateBolt`).
        for index in 0..BOLT_COUNT {
            let mut bolt = self.bolts[index];
            if !bolt.active {
                continue;
            }
            bolt.life += dt;
            if bolt.life / bolt.max_life.max(1e-4) >= 1.0 {
                bolt.active = false;
                bolt.light_strength = 0.0;
                self.bolts[index] = bolt;
                continue;
            }
            let a = self.corner_world(bolt.anchor_a.0, bolt.anchor_a.1);
            let b = self.corner_world(bolt.anchor_b.0, bolt.anchor_b.1);
            let distance =
                ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
            let points = bolt_path(a, b, distance * 0.14, bolt.phase, self.time);
            let t = bolt.life / bolt.max_life.max(1e-4);
            let flicker_seed = seeded_rand(self.seed as i32, (bolt.life * 1000.0) as i32);
            let flick = ((1.0 - t)
                * (0.72 + (self.time * 72.0 + bolt.phase).sin() * 0.16 + flicker_seed * 0.12)
                * (0.85 + self.flash * 0.45))
                .min(1.0);
            bolt.light_pos = points[points.len() / 2];
            bolt.light_strength = flick * (1.2 + self.smoothed_bass * 2.4 + self.flash * 1.4);
            self.bolts[index] = bolt;
        }

        self.upload_uniforms();
        if let Some(gpu) = self.gpu.as_ref() {
            // Upload the frame's instances and bolts.
            let batches = self.collect_batches();
            let mut instances: Vec<EmissiveInstance> = Vec::new();
            for (_, list) in &batches {
                instances.extend_from_slice(list);
            }
            instances.truncate(MAX_INSTANCES);
            if !instances.is_empty() {
                gpu.queue
                    .write_buffer(&gpu.instance_buffer, 0, bytemuck::cast_slice(&instances));
            }
            let mut ring_z = [0.0_f32; RING_COUNT];
            for (index, z) in ring_z.iter_mut().enumerate() {
                *z = self.current_pose(index).position[2];
            }
            let bolt_vertices = self.bolt_vertices(&ring_z);
            if !bolt_vertices.is_empty() {
                gpu.queue
                    .write_buffer(&gpu.bolt_vertices, 0, bytemuck::cast_slice(&bolt_vertices));
            }
            let dust = self.dust_instances();
            gpu.queue
                .write_buffer(&gpu.dust_instances, 0, bytemuck::cast_slice(&dust));
            gpu.queue.write_buffer(
                &gpu.final_uniforms,
                0,
                bytemuck::bytes_of(&FinalUniforms {
                    resolution: [self.size.0 as f32, self.size.1 as f32],
                    bloom_strength: self.bloom_mix_strength(),
                    _pad: 0.0,
                }),
            );
        }
    }

    fn render(&mut self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        let view_proj = self.view_proj();
        let batches = self.collect_batches();
        let bloom_strength = self.bloom_strength();
        let bolt_count = self
            .bolts
            .iter()
            .filter(|bolt| bolt.active)
            .count()
            .min(BOLT_COUNT);

        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };
        // SSAOPass copies the camera matrices every frame because the
        // bass-driven FOV changes (`:1009-1017`).
        gpu.ssao.set_projection(&view_proj);

        let stride = std::mem::size_of::<EmissiveInstance>() as u64;

        // Scene geometry: clear to transparent black, then draw every batch.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("point10-tunnel-scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &gpu.scene_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &gpu.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&gpu.pipeline);
            pass.set_bind_group(0, &gpu.instance_bind_group, &[]);

            let mut offset = 0_u32;
            for (key, list) in &batches {
                let count = list.len() as u32;
                if count == 0 {
                    continue;
                }
                let mesh = match key {
                    MeshKey::Ring(sides) => gpu
                        .ring_meshes
                        .iter()
                        .find(|(value, _)| value == sides)
                        .map(|(_, mesh)| mesh),
                    MeshKey::Box => Some(&gpu.box_mesh),
                };
                let Some(mesh) = mesh else {
                    continue;
                };
                pass.set_vertex_buffer(
                    1,
                    gpu.instance_buffer
                        .slice(offset as u64 * stride..(offset + count) as u64 * stride),
                );
                mesh.draw_instanced(&mut pass, count);
                offset += count;
            }
        }

        // Lightning ribbons (additive, no depth write).
        if bolt_count > 0 {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("point10-tunnel-bolts"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &gpu.scene_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&gpu.bolt_pipeline);
            pass.set_bind_group(0, &gpu.bolt_bind_group, &[]);
            pass.set_vertex_buffer(0, gpu.bolt_vertices.slice(..));
            pass.draw(0..(bolt_count as u32 * BOLT_VERTICES_PER_BOLT as u32), 0..1);
        }

        // Dust (additive billboards).
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("point10-tunnel-dust"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &gpu.scene_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&gpu.dust_pipeline);
            pass.set_bind_group(0, &gpu.dust_bind_group, &[]);
            pass.set_vertex_buffer(1, gpu.dust_instances.slice(..));
            gpu.quad.draw_instanced(&mut pass, DUST_COUNT as u32);
        }

        // SSAO over the scene depth, then `scene * ao` into the lit target
        // (three's `blendMaterial` multiply).
        gpu.ssao.encode(encoder);
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("point10-tunnel-lit"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &gpu.lit_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&gpu.mix_pipeline);
            pass.set_bind_group(0, &gpu.mix_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        gpu.bloom.set_strength(bloom_strength);
        gpu.bloom.encode(encoder);

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("point10-tunnel-final"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: gpu.composer.scene_view(),
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&gpu.final_pipeline);
            pass.set_bind_group(0, &gpu.final_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        gpu.composer.render(encoder, target);
    }

    fn dispose(&mut self) {
        self.gpu = None;
    }

    fn props(&self) -> Vec<SceneProp> {
        scene_props_with_levers(GEOMETRIC_TUNNEL_PROPS)
    }

    fn meta(&self) -> SceneMeta {
        SceneMeta::for_scene("Point10 Geometric Tunnel", "3d-scenes")
    }
}

// ---------------------------------------------------------------------------
// Helpers used by the pipeline above
// ---------------------------------------------------------------------------

fn create_mix_bind_group(device: &wgpu::Device, gpu: &TunnelGpu) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("point10-tunnel-mix-bind-group"),
        layout: &gpu.mix_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: gpu.mix_uniforms.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&gpu.scene_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(gpu.ssao.output_view()),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::Sampler(&gpu.mix_sampler),
            },
        ],
    })
}

fn create_final_bind_group(device: &wgpu::Device, gpu: &TunnelGpu) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("point10-tunnel-final-bind-group"),
        layout: &gpu.final_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: gpu.final_uniforms.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&gpu.lit_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(
                    gpu.bloom
                        .composite_view()
                        .expect("the bloom pass always has a composite target"),
                ),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::Sampler(&gpu.final_sampler),
            },
        ],
    })
}

/// Upload one procedural surface map (`THREE.DataTexture`).
///
/// The grain is `RGBA8` with the height/noise in all three channels and an
/// opaque alpha; the environment is the equirect studio in sRGB. Both use
/// `LinearFilter` and repeat addressing, like `_dataTexture` /
/// `_environmentTexture`.
fn create_surface_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    width: u32,
    height: u32,
    data: &[u8],
    format: wgpu::TextureFormat,
) -> wgpu::Texture {
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
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    texture
}

fn uniform_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

fn tunnel_wgsl() -> String {
    format!("{TUNNEL_COMMON_WGSL}\n{TUNNEL_WGSL}")
}

fn bolt_wgsl() -> String {
    format!("{TUNNEL_BOLT_WGSL}")
}

fn dust_wgsl() -> String {
    format!("{TUNNEL_DUST_WGSL}")
}

fn mix_wgsl() -> String {
    format!("{FULLSCREEN_TRIANGLE_WGSL}\n{TUNNEL_MIX_WGSL}")
}

fn final_wgsl() -> String {
    format!("{FULLSCREEN_TRIANGLE_WGSL}\n{TUNNEL_FINAL_WGSL}")
}

const TUNNEL_COMMON_WGSL: &str = r#"
struct TunnelUniforms {
  view_proj: mat4x4<f32>,
  camera_pos: vec4<f32>,
  // rgb = fog colour, w = fog near
  fog: vec4<f32>,
  // x = fog far, y = hemi, z = key, w = rim
  lights: vec4<f32>,
  // x = l1 intensity, y = l2 intensity, z = target intensity, w = time
  point_lights: vec4<f32>,
  // x = l1 x, y = l2 x, z = target z
  light_positions: vec4<f32>,
  // x = bolt light 0, y = bolt light 1
  bolt_lights: vec4<f32>,
  bolt_pos0: vec4<f32>,
  bolt_pos1: vec4<f32>,
}
@group(0) @binding(0) var<uniform> u: TunnelUniforms;
@group(0) @binding(1) var grain_texture: texture_2d<f32>;
@group(0) @binding(2) var surface_sampler: sampler;
@group(0) @binding(3) var environment_texture: texture_2d<f32>;

fn point_light(position: vec3<f32>, colour: vec3<f32>, intensity: f32, normal: vec3<f32>, world: vec3<f32>) -> vec3<f32> {
  let to_light = position - world;
  let distance = length(to_light);
  let direction = to_light / max(distance, 0.001);
  let attenuation = 1.0 / (1.0 + distance * 1.5);
  return colour * intensity * max(dot(normal, direction), 0.0) * attenuation;
}

fn fog_mix(colour: vec3<f32>, view_distance: f32) -> vec3<f32> {
  let factor = clamp((view_distance - u.fog.w) / max(u.lights.x - u.fog.w, 0.001), 0.0, 1.0);
  let eased = factor * factor * (3.0 - 2.0 * factor);
  return mix(colour, u.fog.rgb, eased);
}
"#;

const TUNNEL_WGSL: &str = r#"
struct VsOut {
  @builtin(position) position: vec4<f32>,
  @location(0) normal: vec3<f32>,
  @location(1) world: vec3<f32>,
  @location(2) colour: vec3<f32>,
  @location(3) emissive: vec4<f32>,
}

@vertex
fn vs_main(
  @location(0) position: vec3<f32>,
  @location(1) normal: vec3<f32>,
  @location(2) uv: vec2<f32>,
  @location(4) model_0: vec4<f32>,
  @location(5) model_1: vec4<f32>,
  @location(6) model_2: vec4<f32>,
  @location(7) model_3: vec4<f32>,
  @location(8) instance_colour: vec4<f32>,
  @location(9) instance_emissive: vec4<f32>,
) -> VsOut {
  let model = mat4x4<f32>(model_0, model_1, model_2, model_3);
  let world = model * vec4<f32>(position, 1.0);
  var out: VsOut;
  out.position = u.view_proj * world;
  out.normal = normalize((model * vec4<f32>(normal, 0.0)).xyz);
  out.world = world.xyz;
  out.colour = instance_colour.rgb;
  out.emissive = instance_emissive;
  return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
  let normal = normalize(in.normal);
  let view_dir = normalize(u.camera_pos.xyz - in.world);

  // HemisphereLight(0x9fc4ff, 0x1b0a24, 0.42) + a key and a magenta rim.
  let hemi = mix(vec3<f32>(0.02, 0.008, 0.03), vec3<f32>(0.62, 0.77, 1.0), normal.y * 0.5 + 0.5) * u.lights.y;
  let key = max(dot(normal, normalize(vec3<f32>(3.0, 4.0, 6.0))), 0.0) * u.lights.z * 0.5;
  let rim = max(dot(normal, normalize(vec3<f32>(-4.0, -2.0, 5.0))), 0.0) * u.lights.w * 0.5;

  var lit = in.colour * (hemi + vec3<f32>(0.84, 0.89, 1.0) * key + vec3<f32>(1.0, 0.5, 0.77) * rim);

  // PointLight l1 (blue, at (sin(t*0.14)*1.5, 1, 5)) and l2 (pink, at
  // (cos(t*0.2)*1.5, 0, -30)); the light x positions travel with the camera.
  lit = lit + point_light(
    vec3<f32>(u.light_positions.x, 1.0, 5.0),
    vec3<f32>(0.53, 0.67, 1.0),
    u.point_lights.x,
    normal,
    in.world,
  ) * in.colour;
  lit = lit + point_light(
    vec3<f32>(u.light_positions.y, 0.0, -30.0),
    vec3<f32>(1.0, 0.4, 0.67),
    u.point_lights.y,
    normal,
    in.world,
  ) * in.colour;
  lit = lit + point_light(
    vec3<f32>(0.0, 0.0, u.light_positions.z),
    vec3<f32>(0.56, 0.49, 1.0),
    u.point_lights.z,
    normal,
    in.world,
  ) * in.colour;

  // The two bolt lights (`boltLights`) flash the nearby hoops.
  lit = lit + point_light(
    u.bolt_pos0.xyz,
    vec3<f32>(0.3, 0.55, 1.0),
    u.bolt_lights.x * 0.7,
    normal,
    in.world,
  ) * in.colour;
  lit = lit + point_light(
    u.bolt_pos1.xyz,
    vec3<f32>(1.0, 0.31, 0.74),
    u.bolt_lights.y * 0.7,
    normal,
    in.world,
  ) * in.colour;

  // `roughnessMap` / `emissiveMap` / `bumpMap` are all `grainMap`
  // (`_neonMat`). The extruded ring quads carry no meaningful UVs, so the
  // grain is sampled triplanar from world position, at the same 2.5x repeat
  // the JS texture uses.
  let grain_position = in.world * 2.0;
  let blend = pow(abs(normal), vec3<f32>(4.0));
  let blend_total = max(blend.x + blend.y + blend.z, 0.0001);
  let grain = (
    textureSample(grain_texture, surface_sampler, grain_position.zy).r * blend.x
    + textureSample(grain_texture, surface_sampler, grain_position.xz).r * blend.y
    + textureSample(grain_texture, surface_sampler, grain_position.xy).r * blend.z
  ) / blend_total;

  // `frame.material.emissive * emissiveIntensity * emissiveMap` (the palette
  // glow, worn by the brushed grain).
  let emissive = in.emissive.rgb * in.emissive.w * grain * 1.6;

  // `MeshStandardMaterial({ metalness: 0.9, roughness: 0.8,
  // envMapIntensity: 1.45 })`: the equirect studio reflection is what gives
  // the hoops their metal sheen. `TUNNEL_SURFACE.env_intensity` is 1.45.
  let rough = clamp(0.8 * (0.35 + 0.65 * grain), 0.12, 1.0);
  let reflected = reflect(-view_dir, normal);
  let environment_uv = vec2<f32>(
    atan2(reflected.z, reflected.x) * 0.15915494 + 0.5,
    asin(clamp(reflected.y, -1.0, 1.0)) * 0.31830989 + 0.5,
  );
  let environment = textureSample(environment_texture, surface_sampler, environment_uv).rgb;
  let reflection = environment * 1.45 * (1.0 - rough * 0.55) * 0.35;

  // Brushed metal: the grain breaks up the lit base and the environment
  // reflects in the tinted metal (`metalness: 0.9`).
  let brushed = lit * (0.85 + 0.15 * grain) + in.colour * reflection * 0.85;
  let colour = fog_mix(brushed + emissive, length(u.camera_pos.xyz - in.world));
  return vec4<f32>(colour, 1.0);
}
"#;

const TUNNEL_BOLT_WGSL: &str = r#"
struct BoltUniforms {
  view_proj: mat4x4<f32>,
  camera_right: vec4<f32>,
  camera_up: vec4<f32>,
}
@group(0) @binding(0) var<uniform> u: BoltUniforms;

struct VsOut {
  @builtin(position) position: vec4<f32>,
  @location(0) colour: vec3<f32>,
  @location(1) alpha: f32,
}

@vertex
fn vs_main(
  @location(0) position: vec3<f32>,
  @location(1) colour: vec3<f32>,
  @location(2) alpha: f32,
) -> VsOut {
  var out: VsOut;
  out.position = u.view_proj * vec4<f32>(position, 1.0);
  out.colour = colour;
  out.alpha = alpha;
  return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
  return vec4<f32>(in.colour, in.alpha);
}
"#;

const TUNNEL_DUST_WGSL: &str = r#"
struct BillboardUniforms {
  view_proj: mat4x4<f32>,
  camera_right: vec4<f32>,
  camera_up: vec4<f32>,
}
@group(0) @binding(0) var<uniform> u: BillboardUniforms;

struct VsOut {
  @builtin(position) position: vec4<f32>,
  @location(0) colour: vec3<f32>,
  @location(1) alpha: f32,
  @location(2) uv: vec2<f32>,
}

@vertex
fn vs_main(
  @location(2) corner: vec2<f32>,
  @location(3) uv: vec2<f32>,
  @location(4) instance_position: vec3<f32>,
  @location(5) instance_size: f32,
  @location(6) instance_colour: vec3<f32>,
  @location(7) instance_alpha: f32,
) -> VsOut {
  // `sizeAttenuation: true`: a world-space quad, so `size` shrinks with depth.
  let half = instance_size * 0.5;
  let world = instance_position
    + u.camera_right.xyz * corner.x * half * 2.0
    + u.camera_up.xyz * corner.y * half * 2.0;
  var out: VsOut;
  out.position = u.view_proj * vec4<f32>(world, 1.0);
  out.colour = instance_colour;
  out.alpha = instance_alpha;
  out.uv = uv;
  return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
  // `createParticleTexture()`'s circular alpha ramp (0.55 inner radius).
  let radius = length(in.uv * 2.0 - 1.0);
  let mask = 1.0 - smoothstep(0.55, 1.0, radius);
  if (mask <= 0.0) {
    discard;
  }
  return vec4<f32>(in.colour, in.alpha * mask);
}
"#;

const TUNNEL_MIX_WGSL: &str = r#"
struct MixUniforms {
  resolution: vec2<f32>,
  _pad: vec2<f32>,
}
@group(0) @binding(0) var<uniform> u: MixUniforms;
@group(0) @binding(1) var scene_colour: texture_2d<f32>;
@group(0) @binding(2) var ao_factor: texture_2d<f32>;
@group(0) @binding(3) var source_sampler: sampler;

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let uv = position.xy / u.resolution;
  let scene = textureSample(scene_colour, source_sampler, uv);
  let ao = textureSample(ao_factor, source_sampler, uv).r;
  // three's SSAOPass multiplies the beauty by the AO factor.
  return vec4<f32>(scene.rgb * ao, scene.a);
}
"#;

const TUNNEL_FINAL_WGSL: &str = r#"
struct FinalUniforms {
  resolution: vec2<f32>,
  bloom_strength: f32,
  _pad: f32,
}
@group(0) @binding(0) var<uniform> u: FinalUniforms;
@group(0) @binding(1) var base_texture: texture_2d<f32>;
@group(0) @binding(2) var bloom_colour: texture_2d<f32>;
@group(0) @binding(3) var source_sampler: sampler;

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let uv = position.xy / u.resolution;
  let base = textureSample(base_texture, source_sampler, uv);
  let bloom = textureSample(bloom_colour, source_sampler, uv);
  let colour = base.rgb + bloom.rgb * u.bloom_strength;
  // `lerp(baseTexture, base + bloom, 1)` with alpha = clamp(max(rgb)).
  return vec4<f32>(colour, clamp(max(colour.r, max(colour.g, colour.b)), 0.0, 1.0));
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32, tolerance: f32) {
        assert!((a - b).abs() < tolerance, "{a} != {b}");
    }

    #[test]
    fn base_tint_is_the_linear_js_colour() {
        // three converts `color: 0x3a4358` from sRGB to linear (0.042/0.056/
        // 0.095). Treating the sRGB bytes as linear made the hoops a flat
        // lavender and hid the surface maps.
        let converted = srgb_hex_to_linear(TUNNEL_BASE_TINT_HEX);
        for (linear, expected) in TUNNEL_BASE_TINT.iter().zip(converted.iter()) {
            close(*linear, *expected, 1e-6);
        }
        assert!(TUNNEL_BASE_TINT.iter().all(|channel| *channel < 0.1));
    }

    #[test]
    fn surface_maps_are_bound_and_sampled() {
        // The JS material maps `grainMap` to roughness and emissive and
        // `scene.environment` to the equirect studio. The port must actually
        // bind/sample them; building the textures alone renders flat paint.
        let program = tunnel_wgsl();
        assert!(program.contains("grain_texture"), "grain map not bound");
        assert!(
            program.contains("environment_texture"),
            "environment map not bound"
        );
        assert!(
            program.contains("textureSample(grain_texture"),
            "the surface shader never samples the grain"
        );
        assert!(
            program.contains("textureSample(environment_texture"),
            "the surface shader never samples the environment"
        );
    }

    #[test]
    fn grain_is_deterministic_grayscale_and_varied() {
        let grain = build_tunnel_grain(TUNNEL_GRAIN_SIZE);
        assert_eq!(
            grain.len(),
            (TUNNEL_GRAIN_SIZE * TUNNEL_GRAIN_SIZE * 4) as usize
        );
        assert_eq!(build_tunnel_grain(TUNNEL_GRAIN_SIZE), grain);
        assert_eq!(grain[3], 255);

        let mut min = 255_u8;
        let mut max = 0_u8;
        for pixel in grain.chunks_exact(4) {
            assert_eq!(pixel[0], pixel[1]);
            assert_eq!(pixel[1], pixel[2]);
            min = min.min(pixel[0]);
            max = max.max(pixel[0]);
        }
        assert!(min > 30, "grain went too dark: {min}");
        assert!(max < 240, "grain clipped: {max}");
        assert!(max - min > 35, "grain has no variation");
    }

    #[test]
    fn noise_tiles_at_the_lattice() {
        for cells in [3_u32, 7, 15, 31] {
            assert_eq!(
                tileable_noise(0.0, 0.37, cells, 3),
                tileable_noise(1.0, 0.37, cells, 3)
            );
            assert_eq!(
                tileable_noise(0.37, 0.0, cells, 3),
                tileable_noise(0.37, 1.0, cells, 3)
            );
        }
        let a = tunnel_hash(3, 7, 11);
        assert_eq!(a, tunnel_hash(3, 7, 11));
        assert!((0.0..1.0).contains(&a));
        assert_ne!(tunnel_hash(3, 7, 12), a);
    }

    #[test]
    fn environment_is_a_bright_cool_top_over_a_dark_floor() {
        let env = build_tunnel_environment(TUNNEL_ENV_WIDTH, TUNNEL_ENV_HEIGHT);
        assert_eq!(
            env.len(),
            (TUNNEL_ENV_WIDTH * TUNNEL_ENV_HEIGHT * 4) as usize
        );
        assert_eq!(
            build_tunnel_environment(TUNNEL_ENV_WIDTH, TUNNEL_ENV_HEIGHT),
            env
        );
        assert_eq!(env[3], 255);

        let row_luminance = |row: u32| {
            let start = row * TUNNEL_ENV_WIDTH;
            let mut sum = 0.0;
            for x in 0..TUNNEL_ENV_WIDTH {
                let index = ((start + x) * 4) as usize;
                sum += (env[index] as f32 + env[index + 1] as f32 + env[index + 2] as f32) / 3.0;
            }
            sum / TUNNEL_ENV_WIDTH as f32
        };
        let top = row_luminance(TUNNEL_ENV_HEIGHT - 1);
        let bottom = row_luminance(0);
        assert!(top > bottom * 1.5, "top {top} vs bottom {bottom}");

        let brightest = env
            .chunks_exact(4)
            .flat_map(|pixel| [pixel[0], pixel[1], pixel[2]])
            .max()
            .unwrap_or(0);
        assert!(brightest > 220, "environment lost its light panels");
    }

    #[test]
    fn surface_and_ssao_constants_match_the_js() {
        assert!(TUNNEL_SURFACE.metalness >= 0.8);
        assert!(TUNNEL_SURFACE.roughness > 0.3);
        assert!(TUNNEL_SURFACE.env_intensity > 1.0);
        assert!(TUNNEL_SURFACE.bump_scale > 0.0);
        assert!(TUNNEL_SSAO.radius > 0.0 && TUNNEL_SSAO.radius < 2.0);
        assert!(TUNNEL_SSAO.max_distance > TUNNEL_SSAO.min_distance);
    }

    #[test]
    fn ring_sides_follow_the_js_tables() {
        assert_eq!(ring_sides(0, false), 4);
        assert_eq!(ring_sides(1, false), 8);
        assert_eq!(ring_sides(3, false), 32);
        assert_eq!(ring_sides(0, true), 32);
        assert_eq!(ring_sides(2, true), 4);
        assert_eq!(ring_sides(12, false), 4);
    }

    #[test]
    fn drive_matches_the_one_pole_and_scroll_maths() {
        let drive = advance_tunnel(0.0, 0.0, 0.0, 0.0, 0.1, 0.5, 0.25, 0.5, 1.0);
        let beta = 1.0 - (-0.1_f32 * 3.5).exp();
        close(drive.smoothed_bass, 0.5 * beta, 1e-6);
        close(drive.smoothed_mid, 0.25 * beta, 1e-6);
        assert_eq!(drive.flash, 0.5);

        let turn = 2.6 + drive.smoothed_bass * 5.0 + drive.smoothed_mid * 2.5 + 0.5 * 3.0;
        close(drive.turn, turn, 1e-5);
        close(drive.scroll, 0.1 * turn, 1e-5);

        // A long frame decays the flash, and a non-finite dt advances nothing
        // (the JS reads a finite clock, so the guard is the port's own).
        let decayed = advance_tunnel(0.0, 0.0, 0.0, 1.0, 0.1, 0.0, 0.0, 0.0, 0.0);
        assert!(decayed.flash < 1.0);
        let bad = advance_tunnel(0.0, 0.0, 0.0, 0.0, f32::NAN, 0.0, 0.0, 0.0, 1.0);
        assert_eq!(bad.scroll, 0.0);
        assert_eq!(bad.turn, 2.6);
    }

    #[test]
    fn frame_hoops_use_the_tunnel_radius_and_hoop_depth() {
        // R-31: the JS frame geometry is authored at RADIUS with
        // TUNNEL_HOOP_DEPTH; the shared unit band must be scaled to match or
        // the hoops render as small, deep washers.
        let mut scene = GeometricTunnel::new();
        scene.time = 0.4;
        let pose = scene.current_pose(0);
        let batches = scene.collect_batches();
        let (_, instances) = batches
            .iter()
            .find(|(key, _)| matches!(key, MeshKey::Ring(_)))
            .expect("ring batches exist");
        let model = &instances[0].model;
        let column_length = |column: usize| {
            (model[column][0] * model[column][0]
                + model[column][1] * model[column][1]
                + model[column][2] * model[column][2])
                .sqrt()
        };
        assert!((column_length(0) - RADIUS * pose.scale).abs() < 1e-4);
        assert!((column_length(1) - RADIUS * pose.scale).abs() < 1e-4);
        assert!(
            (column_length(2) - TUNNEL_HOOP_DEPTH * pose.depth_scale).abs() < 1e-4,
            "frame depth {} != {}",
            column_length(2),
            TUNNEL_HOOP_DEPTH * pose.depth_scale
        );
    }

    #[test]
    fn ring_pose_wraps_through_the_tunnel() {
        let first = ring_pose(0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0);
        // `-((0.5 * spacing) % tunnelLen) + PASSTHR`
        close(first.position[2], -1.6 + PASSTHR, 1e-5);
        close(first.depth, 1.0 + first.position[2] / TUNNEL_LEN, 1e-6);
        // The first ring starts closest, so `near` is 0 and its scale shrinks
        // to the 2% floor; a deep ring is fully scaled.
        assert_eq!(first.near, 0.0);
        close(first.scale, 1.0 * 0.02, 1e-5);
        let deep = ring_pose(9, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0);
        assert_eq!(deep.near, 1.0);
        close(deep.scale, 1.0, 1e-5);

        // A full tunnel length later the ring is back where it started.
        let later = ring_pose(0, TUNNEL_LEN, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0);
        close(later.position[2], first.position[2], 1e-4);

        // Sway is bounded by `swayOn` and only appears with the live bass.
        let swayed = ring_pose(0, 0.0, 1.0, 1.0, 0.0, 1.3, 0.7, 1.5);
        assert!(swayed.position[0].abs() > 0.0);
        assert!(swayed.position[0].abs() <= 1.5 * (0.3 + 1.2) * 1.5);
    }

    #[test]
    fn ring_colour_runs_the_hue_cycle_and_brightness_ramp() {
        let quiet = ring_colour(0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0);
        let loud = ring_colour(0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0, 2.0);
        assert!(loud.bright > quiet.bright);
        assert!(loud.frame_intensity > quiet.frame_intensity);
        assert!(loud.inner_intensity > quiet.inner_intensity);

        let later = ring_colour(0, 5.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0);
        assert_ne!(quiet.colour, later.colour);
        // HSL output stays in the RGB cube.
        for channel in quiet.colour {
            assert!((0.0..=1.0).contains(&channel));
        }
        assert_eq!(hsl_to_rgb(0.0, 0.0, 0.5), [0.5, 0.5, 0.5]);
    }

    #[test]
    fn dust_look_and_seeds_are_bounded() {
        let look = dust_look(0.0, 0.5, 0.2);
        assert!(look.size > 0.0);
        assert!(look.opacity > 0.0);
        for channel in look.colour {
            assert!(channel >= 0.0);
        }
        for index in 0..DUST_COUNT {
            let p = dust_seed(index);
            let radius = (p[0] * p[0] + p[1] * p[1]).sqrt();
            assert!(radius <= RADIUS * 0.9 + 1e-5);
            assert!(p[2] <= 0.0 && p[2] >= -TUNNEL_LEN);
        }
    }

    #[test]
    fn bolt_path_jags_and_keeps_its_endpoints() {
        let a = [0.0, 0.0, 0.0];
        let b = [10.0, 0.0, 0.0];
        let path = bolt_path(a, b, 1.4, 0.3, 1.0);
        assert_eq!(path, bolt_path(a, b, 1.4, 0.3, 1.0));
        assert_eq!(path.len(), BOLT_POINTS);
        assert_eq!(path[0], a);
        assert_eq!(*path.last().unwrap(), b);
        // Midpoint displacement moves interior points off the straight line.
        assert!(
            path.iter()
                .skip(1)
                .take(15)
                .any(|point| point[1].abs() > 1e-4)
        );
        // A later time gives a different jag.
        assert_ne!(path, bolt_path(a, b, 1.4, 0.3, 2.0));
    }

    #[test]
    fn branch_path_follows_the_js_shape() {
        let points = bolt_path([0.0; 3], [10.0, 0.0, 0.0], 1.0, 0.7, 0.0);
        let branch = branch_path(&points, 10.0, 0.7);
        assert_eq!(branch.len(), BRANCH_POINTS);
        assert_eq!(branch[0], points[points.len() * 55 / 100]);
        assert_ne!(branch[0], branch[branch.len() - 1]);
        assert!(branch_path(&[], 1.0, 0.0).is_empty());
    }

    #[test]
    fn ring_band_mesh_spans_the_annulus() {
        let mesh = build_ring_band(4, RADIUS, 0.34, TUNNEL_HOOP_DEPTH);
        assert!(!mesh.vertices.is_empty());
        assert_eq!(mesh.indices.len() % 3, 0);
        for vertex in &mesh.vertices {
            let radius = (vertex.position[0].powi(2) + vertex.position[1].powi(2)).sqrt();
            assert!(radius <= RADIUS + 1e-4);
            assert!(radius >= RADIUS - 0.34 - 1e-4);
            assert!(vertex.position[2].abs() <= TUNNEL_HOOP_DEPTH * 0.5 + 1e-6);
        }
    }

    #[test]
    fn preset_constants_match_the_js() {
        assert_eq!(TUNNEL_HOOP_DEPTH, 0.05);
        assert_eq!(TUNNEL_GRAIN_SIZE, 96);
        assert_eq!((TUNNEL_ENV_WIDTH, TUNNEL_ENV_HEIGHT), (128, 64));
        assert_eq!(RING_COUNT, 12);
        assert_eq!(SPACING, 3.2);
        assert_eq!(TUNNEL_LEN, 38.4);
        assert_eq!(RADIUS, 4.2);
        assert_eq!(PASSTHR, 6.0);
        assert_eq!(
            (BLOOM_STRENGTH, BLOOM_RADIUS, BLOOM_THRESHOLD),
            (0.72, 1.0, 0.35)
        );
        assert_eq!(CLEAR_ALPHA, 0.0);
        assert_eq!(CAMERA_FOV, 64.0);
        assert_eq!(DUST_COUNT, 280);
    }

    #[test]
    fn uniforms_match_the_documented_sizes() {
        assert_eq!(std::mem::size_of::<TunnelUniforms>(), 192);
        assert_eq!(std::mem::size_of::<EmissiveInstance>(), 96);
        assert_eq!(std::mem::size_of::<DustInstance>(), 32);
        assert_eq!(std::mem::size_of::<BoltVertex>(), 28);
        assert_eq!(std::mem::size_of::<BillboardUniforms>(), 96);
        assert_eq!(std::mem::size_of::<FinalUniforms>(), 16);
    }

    #[test]
    fn camera_matches_the_js_orbit() {
        let mut scene = GeometricTunnel::new();
        scene.time = 0.0;
        assert_eq!(scene.camera_position(), [0.0, 0.2, 3.0]);
        assert_eq!(scene.camera_roll(), 0.0);
        assert_eq!(scene.camera_fov(), CAMERA_FOV);
        scene.time = 1.0;
        assert_ne!(scene.camera_position(), [0.0, 0.2, 3.0]);
        assert!(scene.camera_roll().abs() > 0.0);
    }

    #[test]
    fn wgsl_parses_and_validates() {
        for (label, program) in [
            ("tunnel", tunnel_wgsl()),
            ("bolt", bolt_wgsl()),
            ("dust", dust_wgsl()),
            ("final", final_wgsl()),
        ] {
            let module = match naga::front::wgsl::parse_str(&program) {
                Ok(module) => module,
                Err(error) => panic!("{label} WGSL failed to parse: {error}"),
            };
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap_or_else(|error| panic!("{label} WGSL must validate: {error}"));
        }
    }

    #[test]
    fn shaders_keep_the_ported_maths() {
        let program = tunnel_wgsl();
        for needle in [
            "let hemi = mix(",
            "let key = max(dot(normal, normalize(vec3<f32>(3.0, 4.0, 6.0))), 0.0)",
            "let emissive = in.emissive.rgb * in.emissive.w * grain * 1.6;",
            "let factor = clamp((view_distance - u.fog.w) / max(u.lights.x - u.fog.w, 0.001), 0.0, 1.0);",
        ] {
            assert!(program.contains(needle), "tunnel shader lost {needle}");
        }
        let final_shader = final_wgsl();
        assert!(final_shader.contains("bloom.rgb * u.bloom_strength"));
        assert!(final_shader.contains("clamp(max(colour.r, max(colour.g, colour.b)), 0.0, 1.0)"));
        assert!(dust_wgsl().contains("smoothstep(0.55, 1.0, radius)"));
    }

    #[test]
    fn props_and_meta_match_the_js_registry() {
        let scene = GeometricTunnel::new();
        assert_eq!(scene.meta().name, "Point10 Geometric Tunnel");
        assert_eq!(scene.meta().category, "3d-scenes");
        let names: Vec<&str> = scene.props().iter().map(|prop| prop.name).collect();
        assert_eq!(
            &names[names.len() - 3..],
            &["scrollSpeed", "brightness", "bloom"]
        );
        assert_eq!(names[0], "gain");
    }

    #[test]
    fn props_bind_and_clamp() {
        let mut scene = GeometricTunnel::new();
        scene.apply_props(|key| match key {
            "scrollSpeed" => Some(9.0),
            "brightness" => Some(-1.0),
            "bloom" => Some(2.5),
            _ => None,
        });
        assert_eq!(scene.speed_scale, 4.0);
        assert_eq!(scene.brightness_scale, 0.0);
        assert_eq!(scene.bloom_scale, 2.5);
    }
}
