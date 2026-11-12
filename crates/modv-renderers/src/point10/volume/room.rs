//! Shared room, audio and fire-light maths for SmokeRoom and DvdRoom.
//!
//! Both JS scenes are the same construction: the `12 x 12 x 24` volume grid is
//! the room, a thin glass box wraps it, a fixed/orbiting camera frames it, and
//! the audio drives a hi-hat/snare emission pulse plus a bass-phrase hue flip.
//! This module holds everything that is not the simulation itself
//! ([`super::VolumeSim`]) or the render chain ([`super::stage::VolumeStage`]),
//! so the two scenes only describe what makes them different (emitter, lights,
//! logo, palette).
//!
//! Ported from `SmokeRoom.js`:
//!
//! * `:60-88` the room constants and `GLASS_PADDING` / `GLASS_TOP_MARGIN`;
//! * `:130-185` `boxHalfExtents` / `requiredGlassTop` / `glassFit`;
//! * `:340-374` the audio mapping (`BASS_BINS`, hi-hat/snare bands,
//!   `uFireSize`);
//! * `:850-858` and `:92-96` the fire ramp and the luminance-equalised hue;
//! * `:1332-1349` `_updateTemporalUniforms` (the CPU Perlin flicker, sway,
//!   flame height and colour noise);
//! * `:1080-1199` the volume point-light colour;
//! * `:1351-1392` the fixed-timestep simulation accumulator.

#![forbid(unsafe_code)]

use crate::point10::audio_bridge::AUDIO_SAMPLES;
use crate::point10::shared::phrase_color::{equalized_hue, srgb_hex_to_linear};

// ---------------------------------------------------------------------------
// Room geometry
// ---------------------------------------------------------------------------

/// `VOLUME_WORLD_SIZE_X` (`SmokeRoom.js:65`) — the sim grid's world footprint.
pub const VOLUME_WORLD_SIZE_X: f32 = super::VOLUME_WORLD_SIZE_X;
/// `VOLUME_WORLD_SIZE_Y` (`SmokeRoom.js:66`).
pub const VOLUME_WORLD_SIZE_Y: f32 = super::VOLUME_WORLD_SIZE_Y;
/// `VOLUME_WORLD_SIZE_Z` (`SmokeRoom.js:67`).
pub const VOLUME_WORLD_SIZE_Z: f32 = super::VOLUME_WORLD_SIZE_Z;

/// `GLASS_PADDING` (`SmokeRoom.js:77`).
pub const GLASS_PADDING: f32 = 0.6;
/// `GLASS_HALF_X` = `(VOLUME_WORLD_SIZE_X + GLASS_PADDING) / 2`.
pub const GLASS_HALF_X: f32 = (VOLUME_WORLD_SIZE_X + GLASS_PADDING) / 2.0;
/// `GLASS_HALF_Y`.
pub const GLASS_HALF_Y: f32 = (VOLUME_WORLD_SIZE_Y + GLASS_PADDING) / 2.0;
/// `GLASS_HALF_Z`.
pub const GLASS_HALF_Z: f32 = (VOLUME_WORLD_SIZE_Z + GLASS_PADDING) / 2.0;
/// `GLASS_TOP_MARGIN` (`SmokeRoom.js:81`).
pub const GLASS_TOP_MARGIN: f32 = 0.75;

/// `SPHERE_RADIUS` (`SmokeRoom.js:73`).
pub const SPHERE_RADIUS: f32 = 0.9;

/// `TEXEL_*` (`SmokeRoom.js:70-72`) for the JS grid.
pub const TEXEL_X: f32 = 1.0 / super::GRID_SIZE_X as f32;
/// See [`TEXEL_X`].
pub const TEXEL_Y: f32 = 1.0 / super::GRID_SIZE_Y as f32;
/// See [`TEXEL_X`].
pub const TEXEL_Z: f32 = 1.0 / super::GRID_SIZE_Z as f32;

/// `BASS_BINS` (`SmokeRoom.js:83`).
pub const BASS_BINS: usize = 5;
/// `_band(1, 9)`'s lower bound (`DvdRoom.js:378`).
pub const BAND_TWO_START: usize = 1;
/// `_band(1, 9)`'s upper bound.
pub const BAND_TWO_END: usize = 9;
/// `FIRE_HAT_K` (`SmokeRoom.js:86`).
pub const FIRE_HAT_K: f32 = 2.5;
/// `FIRE_SNARE_K` (`SmokeRoom.js:87`).
pub const FIRE_SNARE_K: f32 = 8.0;
/// `uFireStartColor` (`0xffe68c`).
pub const FIRE_START_HEX: u32 = 0x00ff_e68c;
/// `uFireMidColor` (`0xff7305`).
pub const FIRE_MID_HEX: u32 = 0x00ff_7305;
/// `uFireEndColor` (`0xff0000`).
pub const FIRE_END_HEX: u32 = 0x00ff_0000;

/// `BLOOM_BASE` (`SmokeRoom.js:84`).
pub const BLOOM_BASE: f32 = 0.12;
/// `BLOOM_PER_BASS` (`SmokeRoom.js:85`).
pub const BLOOM_PER_BASS: f32 = 2.2;
/// The JS clamps the smoke-room bloom at 2.8 (`SmokeRoom.js:1396-1399`).
pub const BLOOM_MAX: f32 = 2.8;

/// `camera.position.set(14.0, 5.5, 4.4)` (`SmokeRoom.js:433`).
pub const CAMERA_START: [f32; 3] = [14.0, 5.5, 4.4];
/// `PerspectiveCamera(60, aspect, 0.1, 100)` (`SmokeRoom.js:432`).
pub const CAMERA_FOV: f32 = 60.0;
/// Near plane.
pub const CAMERA_NEAR: f32 = 0.1;
/// Far plane.
pub const CAMERA_FAR: f32 = 100.0;
/// `renderer.toneMappingExposure = 2` (`SmokeRoom.js:418`).
pub const TONE_MAPPING_EXPOSURE: f32 = 2.0;

/// `scene.background = null` (`SmokeRoom.js:426`): the module clears to
/// transparent so it composites over the layers below.
pub const CLEAR_ALPHA: f64 = 0.0;

// ---------------------------------------------------------------------------
// Glass room fitting (`SmokeRoom.js:130-185`, `:276-294`)
// ---------------------------------------------------------------------------

/// `SmokeRoom.boxHalfExtents(scale, xyScale)`.
pub fn box_half_extents(scale: f32, xy_scale: f32) -> [f32; 3] {
    let scale = if scale.is_finite() {
        scale.max(0.0)
    } else {
        1.0
    };
    let xy = if xy_scale.is_finite() {
        xy_scale.max(0.0)
    } else {
        1.0
    };
    [
        GLASS_HALF_X * scale * xy,
        GLASS_HALF_Y * scale * xy,
        GLASS_HALF_Z * scale,
    ]
}

/// The world-space Y that puts the room's highest edge on the output's top
/// edge (`SmokeRoom.js:146-174`).
///
/// `forward` is the camera's world direction, `up` its up vector. Returns the
/// natural half height when the intersection degenerates (the JS would divide
/// by ~0).
pub fn required_glass_top(
    camera_position: [f32; 3],
    forward: [f32; 3],
    up: [f32; 3],
    fov_degrees: f32,
    scale: f32,
    xy_scale: f32,
) -> f32 {
    let forward = normalise(forward);
    let right = normalise([
        forward[1] * up[2] - forward[2] * up[1],
        forward[2] * up[0] - forward[0] * up[2],
        forward[0] * up[1] - forward[1] * up[0],
    ]);
    let up = [
        right[1] * forward[2] - right[2] * forward[1],
        right[2] * forward[0] - right[0] * forward[2],
        right[0] * forward[1] - right[1] * forward[0],
    ];
    let tan_half_fov = (fov_degrees.to_radians() * 0.5).tan();
    let denominator = up[1] - tan_half_fov * forward[1];
    let extents = box_half_extents(scale, xy_scale);
    if denominator.abs() < 1e-5 {
        return extents[1];
    }

    let mut top = f32::NEG_INFINITY;
    for x in [-extents[0], extents[0]] {
        for z in [-extents[2], extents[2]] {
            let ex = x - camera_position[0];
            let ez = z - camera_position[2];
            let offset =
                ex * up[0] + ez * up[2] - tan_half_fov * (ex * forward[0] + ez * forward[2]);
            top = top.max(camera_position[1] - offset / denominator);
        }
    }
    top
}

/// `SmokeRoom.glassFit(top, scale, xyScale)` — bottom-anchored room height and
/// centre.
pub fn glass_fit(top: f32, scale: f32, xy_scale: f32) -> (f32, f32) {
    let half_y = box_half_extents(scale, xy_scale)[1];
    let top_y = if top.is_finite() {
        top.max(half_y)
    } else {
        half_y
    };
    (top_y + half_y, (top_y - half_y) / 2.0)
}

/// The room top for a camera looking at the origin, including the
/// `GLASS_TOP_MARGIN` (`_fitGlassToOutput`, `SmokeRoom.js:286-290`).
pub fn room_top(camera_position: [f32; 3], scale: f32, xy_scale: f32) -> f32 {
    let forward = normalise([
        -camera_position[0],
        -camera_position[1],
        -camera_position[2],
    ]);
    required_glass_top(
        camera_position,
        forward,
        [0.0, 1.0, 0.0],
        CAMERA_FOV,
        scale,
        xy_scale,
    ) + GLASS_TOP_MARGIN * scale
}

/// `Vector3.normalize()` with a zero guard (`cube_mesh::normalise`).
pub fn normalise(v: [f32; 3]) -> [f32; 3] {
    crate::point10::scenes::cube_mesh::normalise(v)
}

// ---------------------------------------------------------------------------
// Audio (`SmokeRoom.js:223-229`, `:340-374`, `DvdRoom.js:376-408`)
// ---------------------------------------------------------------------------

/// `_band(lo, hi)`: mean of the `freqMag` bins in `[lo, hi)`.
pub fn band(freq_mag: &[f32; AUDIO_SAMPLES], lo: usize, hi: usize) -> f32 {
    let hi = hi.min(freq_mag.len());
    if hi <= lo {
        return 0.0;
    }
    let mut sum = 0.0;
    for value in freq_mag.iter().take(hi).skip(lo) {
        sum += value;
    }
    sum / (hi - lo) as f32
}

/// The top-15% treble mean (`SmokeRoom.js:353-359`, `DvdRoom.js:379-385`).
pub fn treble_band(freq_mag: &[f32; AUDIO_SAMPLES]) -> f32 {
    let lo = (AUDIO_SAMPLES as f32 * 0.85).floor() as usize;
    band(freq_mag, lo, AUDIO_SAMPLES)
}

/// The snare band: bins 2%..13% (`SmokeRoom.js:358-363`).
pub fn snare_band(freq_mag: &[f32; AUDIO_SAMPLES]) -> f32 {
    let lo = (AUDIO_SAMPLES as f32 * 0.02).floor() as usize;
    let hi = (AUDIO_SAMPLES as f32 * 0.13).floor() as usize;
    band(freq_mag, lo, hi)
}

/// `uFireSize = 1 + hi * FIRE_HAT_K + snare * FIRE_SNARE_K`
/// (`SmokeRoom.js:364-365`).
pub fn fire_size(hi: f32, snare: f32) -> f32 {
    1.0 + hi * FIRE_HAT_K + snare * FIRE_SNARE_K
}

/// The bass the smoke room reads: the first five bins (`SmokeRoom.js:346-349`).
pub fn smoke_bass(freq_mag: &[f32; AUDIO_SAMPLES]) -> f32 {
    band(freq_mag, 0, BASS_BINS)
}

// ---------------------------------------------------------------------------
// Fire colour
// ---------------------------------------------------------------------------

/// `fireRamp(t)` (`SmokeRoom.js:850-858`) in linear RGB.
pub fn fire_ramp(t: f32) -> [f32; 3] {
    let start = srgb_hex_to_linear(FIRE_START_HEX);
    let mid = srgb_hex_to_linear(FIRE_MID_HEX);
    let end = srgb_hex_to_linear(FIRE_END_HEX);
    let mix = |a: [f32; 3], b: [f32; 3], k: f32| {
        [
            a[0] + (b[0] - a[0]) * k,
            a[1] + (b[1] - a[1]) * k,
            a[2] + (b[2] - a[2]) * k,
        ]
    };
    let steps = |edge0: f32, edge1: f32, x: f32| {
        let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };

    let colour = mix([0.0; 3], end, steps(0.05, 0.35, t));
    let colour = mix(colour, mid, steps(0.35, 0.65, t));
    mix(colour, start, steps(0.65, 1.0, t))
}

/// `fireHue(fireRamp(t), hue)`: the luminance-equalised hue rotation
/// (`SmokeRoom.js:92-96`).
pub fn fire_colour(t: f32, hue: f32) -> [f32; 3] {
    equalized_hue(fire_ramp(t), hue)
}

/// TSL `saturation(colour, adjustment)` — `mix(luminance, colour, adj)`.
pub fn saturation(colour: [f32; 3], adjustment: f32) -> [f32; 3] {
    let luminance = 0.2126 * colour[0] + 0.7152 * colour[1] + 0.0722 * colour[2];
    [
        (luminance + (colour[0] - luminance) * adjustment).max(0.0),
        (luminance + (colour[1] - luminance) * adjustment).max(0.0),
        (luminance + (colour[2] - luminance) * adjustment).max(0.0),
    ]
}

fn smoothstep01(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

// ---------------------------------------------------------------------------
// CPU Perlin noise — three's `ImprovedNoise`
// ---------------------------------------------------------------------------

const PERM: [i32; 256] = [
    151, 160, 137, 91, 90, 15, 131, 13, 201, 95, 96, 53, 194, 233, 7, 225, 140, 36, 103, 30, 69,
    142, 8, 99, 37, 240, 21, 10, 23, 190, 6, 148, 247, 120, 234, 75, 0, 26, 197, 62, 94, 252, 219,
    203, 117, 35, 11, 32, 57, 177, 33, 88, 237, 149, 56, 87, 174, 20, 125, 136, 171, 168, 68, 175,
    74, 165, 71, 134, 139, 48, 27, 166, 77, 146, 158, 231, 83, 111, 229, 122, 60, 211, 133, 230,
    220, 105, 92, 41, 55, 46, 245, 40, 244, 102, 143, 54, 65, 25, 63, 161, 1, 216, 80, 73, 209, 76,
    132, 187, 208, 89, 18, 169, 200, 196, 135, 130, 116, 188, 159, 86, 164, 100, 109, 198, 173,
    186, 3, 64, 52, 217, 226, 250, 124, 123, 5, 202, 38, 147, 118, 126, 255, 82, 85, 212, 207, 206,
    59, 227, 47, 16, 58, 17, 182, 189, 28, 42, 223, 183, 170, 213, 119, 248, 152, 2, 44, 154, 163,
    70, 221, 153, 101, 155, 167, 43, 172, 9, 129, 22, 39, 253, 19, 98, 108, 110, 79, 113, 224, 232,
    178, 185, 112, 104, 218, 246, 97, 228, 251, 34, 242, 193, 238, 210, 144, 12, 191, 179, 162,
    241, 81, 51, 145, 235, 249, 14, 239, 107, 49, 192, 214, 31, 181, 199, 106, 157, 184, 84, 204,
    176, 115, 121, 50, 45, 127, 4, 150, 254, 138, 236, 205, 93, 222, 114, 67, 29, 24, 72, 243, 141,
    128, 195, 78, 66, 215, 61, 156, 180,
];

/// three's `ImprovedNoise` (Ken Perlin's 2002 improved noise).
#[derive(Clone, Copy, Debug, Default)]
pub struct ImprovedNoise;

impl ImprovedNoise {
    /// `noise.noise(x, y, z)` — the periodic 3D noise the JS uses for the
    /// flicker, sway, flame height and colour noise.
    pub fn noise(&self, x: f32, y: f32, z: f32) -> f32 {
        let floor_x = x.floor();
        let floor_y = y.floor();
        let floor_z = z.floor();
        let x0 = (floor_x as i32) & 255;
        let y0 = (floor_y as i32) & 255;
        let z0 = (floor_z as i32) & 255;
        let x = x - floor_x;
        let y = y - floor_y;
        let z = z - floor_z;
        let x1 = x - 1.0;
        let y1 = y - 1.0;
        let z1 = z - 1.0;
        let u = fade(x);
        let v = fade(y);
        let w = fade(z);

        let p = |index: i32| PERM[(index & 255) as usize];
        let a = p(x0) + y0;
        let aa = p(a) + z0;
        let ab = p(a + 1) + z0;
        let b = p(x0 + 1) + y0;
        let ba = p(b) + z0;
        let bb = p(b + 1) + z0;

        let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
        lerp(
            lerp(
                lerp(grad(p(aa), x, y, z), grad(p(ba), x1, y, z), u),
                lerp(grad(p(ab), x, y1, z), grad(p(bb), x1, y1, z), u),
                v,
            ),
            lerp(
                lerp(grad(p(aa + 1), x, y, z1), grad(p(ba + 1), x1, y, z1), u),
                lerp(grad(p(ab + 1), x, y1, z1), grad(p(bb + 1), x1, y1, z1), u),
                v,
            ),
            w,
        )
    }
}

fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

fn grad(hash: i32, x: f32, y: f32, z: f32) -> f32 {
    let h = hash & 15;
    let u = if h < 8 { x } else { y };
    let v = if h < 4 {
        y
    } else if h == 12 || h == 14 {
        x
    } else {
        z
    };
    (if (h & 1) == 0 { u } else { -u }) + (if (h & 2) == 0 { v } else { -v })
}

// ---------------------------------------------------------------------------
// Temporal uniforms (`SmokeRoom.js:1332-1349`)
// ---------------------------------------------------------------------------

/// The per-step CPU noise values `_updateTemporalUniforms` writes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FireTemporal {
    /// `uTime` (simulation seconds, wrapped at 1000).
    pub time: f32,
    /// `uFlameHeight`.
    pub flame_height: f32,
    /// `uSway`.
    pub sway: [f32; 3],
    /// `uFlicker`.
    pub flicker: f32,
    /// `uColorNoise`.
    pub color_noise: f32,
}

/// `_updateTemporalUniforms(time)`.
pub fn fire_temporal(noise: &ImprovedNoise, time: f32) -> FireTemporal {
    FireTemporal {
        time: time % 1000.0,
        flame_height: 3.5 + noise.noise(0.0, time * 2.5, 0.0) * 0.8,
        sway: [
            noise.noise(time * 3.5, 0.0, 0.0) * 0.4,
            0.0,
            noise.noise(0.0, 0.0, time * 3.5) * 0.4,
        ],
        flicker: noise.noise(0.0, time * 0.8, 0.0) * 0.12
            + noise.noise(0.0, time * 15.0, 0.0) * 0.06
            + 0.82,
        color_noise: noise.noise(time * 5.0, time * 5.0, 0.0) * 0.08,
    }
}

/// The `uFireIntensity`/`uEmitTemperature`/`uEmitDensity`/fade-in scaling the
/// volume point light and the emitter material share
/// (`SmokeRoom.js:1178-1189`).
pub fn emission_scale(emit_temperature: f32, emit_density: f32, fire_intensity: f32) -> f32 {
    let temp_scale = (emit_temperature / 33.36).max(0.0);
    let temp_factor = temp_scale.powi(4);
    let density_scale = (emit_density / 44.08).max(0.0);
    temp_factor * density_scale * fire_intensity
}

/// `smoothstep(0, 3, time)` — the three-second emissive fade-in
/// (`_updateTemporalUniforms`'s `smoothstep(0.0, 3.0, uTime)`).
pub fn fade_in(time: f32) -> f32 {
    smoothstep01(time / 3.0)
}

/// The volume's point-light colour (`pointLightColor` with `isVolume == 1`).
///
/// Returns the linear rgb already multiplied by the temperature/density
/// scalars, `intensity` (`uPointLightVolumeIntensity`), the flicker and the
/// 3-second fade-in — the constants the raymarch's `scatteringNode` multiplies
/// by the local density and phase.
pub fn volume_point_light_scaled(
    temporal: &FireTemporal,
    emit_temperature: f32,
    emit_density: f32,
    fire_intensity: f32,
    hue: f32,
    intensity: f32,
) -> [f32; 3] {
    let colour_t = (emit_temperature / 33.36 * 0.5 + 0.2 + temporal.color_noise).clamp(0.0, 1.0);
    let base = fire_colour(colour_t, hue);
    let base = saturation(base, 1.1);
    let fade_in = fade_in(temporal.time);
    let scale = emission_scale(emit_temperature, emit_density, fire_intensity)
        * intensity
        * temporal.flicker
        * fade_in;
    [base[0] * scale, base[1] * scale, base[2] * scale]
}

/// [`volume_point_light_scaled`] at the room scenes' `uPointLightVolumeIntensity`
/// of 2 (`SmokeRoom.js:509`).
pub fn volume_point_light(
    temporal: &FireTemporal,
    emit_temperature: f32,
    emit_density: f32,
    fire_intensity: f32,
    hue: f32,
) -> [f32; 3] {
    volume_point_light_scaled(
        temporal,
        emit_temperature,
        emit_density,
        fire_intensity,
        hue,
        2.0,
    )
}

/// The lava/logo material's emissive scale (`SmokeRoom.js:1052-1062`).
pub fn emitter_glow(
    emit_temperature: f32,
    emit_density: f32,
    fire_intensity: f32,
    temporal: &FireTemporal,
) -> f32 {
    emission_scale(emit_temperature, emit_density, fire_intensity)
        * temporal.flicker
        * smoothstep01(temporal.time / 3.0)
        * 0.2
}

// ---------------------------------------------------------------------------
// Fixed-timestep simulation clock (`SmokeRoom.js:1351-1392`)
// ---------------------------------------------------------------------------

/// One frame's worth of pending simulation work.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SimTick {
    /// Number of `simStep`s to run (capped at 8 per frame).
    pub steps: u32,
    /// `uDt` (`simStep`).
    pub step: f32,
    /// Simulation time before the first step; step `i` (1-based) runs at
    /// `start_time + step * i`.
    pub start_time: f32,
    /// `uTurbulence` for the frame.
    pub turbulence: f32,
    /// `uDissipation`.
    pub dissipation: f32,
    /// `uCooling`.
    pub cooling: f32,
}

impl SimTick {
    /// The `uTime` for the `index`-th step (0-based).
    pub fn time_of(&self, index: u32) -> f32 {
        self.start_time + self.step * (index as f32 + 1.0)
    }
}

/// The accumulator behind `_animate`'s `while (simAccumulator >= simStep)`.
#[derive(Clone, Copy, Debug, Default)]
pub struct SimulationClock {
    accumulator: f32,
    time: f32,
}

impl SimulationClock {
    /// Consume a frame delta and report the simulation work to run.
    pub fn pending(
        &mut self,
        delta: f32,
        simulation_speed: f32,
        smoke_lifespan: f32,
        fire_lifespan: f32,
    ) -> SimTick {
        let speed = if simulation_speed.is_finite() {
            simulation_speed.max(0.0)
        } else {
            1.2
        };
        let delta = if delta.is_finite() {
            delta.clamp(0.0, 1.0 / 30.0)
        } else {
            0.0
        };

        let step = (1.0 / 120.0) * speed;
        self.accumulator += delta * speed;
        let max_accumulator = step * 8.0;
        if self.accumulator > max_accumulator {
            self.accumulator = max_accumulator;
        }

        let start_time = self.time;
        let mut steps = 0;
        // The JS's `while (accumulator >= simStep)` with an epsilon so f32
        // rounding cannot drop a whole step at exact multiples.
        while step > 0.0 && self.accumulator + step * 1e-4 >= step {
            self.time += step;
            self.accumulator -= step;
            steps += 1;
        }

        SimTick {
            steps,
            step,
            start_time,
            turbulence: if speed > 0.0 { 3.2 / speed.sqrt() } else { 0.0 },
            dissipation: if smoke_lifespan >= 100.0 {
                0.0
            } else {
                1.0 / smoke_lifespan.max(0.1)
            },
            cooling: 1.0 / fire_lifespan.max(0.1),
        }
    }

    /// The accumulated simulation time.
    pub fn time(&self) -> f32 {
        self.time
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::point10::shared::phrase_color::ROYGBIV_HUES;

    fn close(a: f32, b: f32, tolerance: f32) {
        assert!((a - b).abs() < tolerance, "{a} != {b}");
    }

    #[test]
    fn room_extents_and_padding_match_the_js() {
        assert_eq!(
            (
                VOLUME_WORLD_SIZE_X,
                VOLUME_WORLD_SIZE_Y,
                VOLUME_WORLD_SIZE_Z
            ),
            (12.0, 12.0, 24.0)
        );
        assert_eq!(GLASS_PADDING, 0.6);
        assert_eq!((GLASS_HALF_X, GLASS_HALF_Y, GLASS_HALF_Z), (6.3, 6.3, 12.3));
        assert_eq!(GLASS_TOP_MARGIN, 0.75);
        close(TEXEL_X, 1.0 / 150.0, 1e-9);
        close(TEXEL_Z, 1.0 / 300.0, 1e-9);
        assert_eq!(box_half_extents(1.0, 1.0), [6.3, 6.3, 12.3]);
        let scaled = box_half_extents(2.5, 1.6);
        close(scaled[0], 6.3 * 2.5 * 1.6, 1e-5);
        close(scaled[1], 6.3 * 2.5 * 1.6, 1e-5);
        close(scaled[2], 12.3 * 2.5, 1e-5);
    }

    #[test]
    fn glass_fit_is_bottom_anchored_to_the_output_top() {
        let top = room_top(CAMERA_START, 1.0, 1.0);
        assert!(top > GLASS_HALF_Y);
        let (height, centre_y) = glass_fit(top, 1.0, 1.0);
        close(centre_y - height * 0.5, -GLASS_HALF_Y, 1e-5);
        close(centre_y + height * 0.5, top, 1e-5);

        // A degenerate camera (looking straight down the top plane) returns
        // the natural half height instead of dividing by zero.
        let degenerate = required_glass_top(
            [0.0, 20.0, 0.0],
            [0.0, -1.0, 0.0],
            [0.0, 0.0, 1.0],
            CAMERA_FOV,
            1.0,
            1.0,
        );
        assert!(degenerate.is_finite());
    }

    #[test]
    fn required_glass_top_keeps_the_room_corners_above_the_frame() {
        use crate::point10::scenes::cube_mesh::{look_at, mat4_mul, perspective, transform_point};
        let view_proj = mat4_mul(
            &perspective(CAMERA_FOV.to_radians(), 16.0 / 9.0, CAMERA_NEAR, CAMERA_FAR),
            &look_at(CAMERA_START, [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        );
        let top = room_top(CAMERA_START, 1.0, 1.0);
        for x in [-GLASS_HALF_X, GLASS_HALF_X] {
            for z in [-GLASS_HALF_Z, GLASS_HALF_Z] {
                let clip = transform_point(&view_proj, [x, top, z]);
                assert!(
                    clip[1] / clip[3] >= 1.0 - 1e-4,
                    "corner ({x}, {top}, {z}) projects to {}",
                    clip[1] / clip[3]
                );
            }
        }
    }

    #[test]
    fn the_audio_bands_use_the_js_bin_ranges() {
        let mut freq = [0.0_f32; AUDIO_SAMPLES];
        for (index, value) in freq.iter_mut().enumerate().take(BASS_BINS) {
            *value = 0.5 + index as f32 * 0.1;
        }
        close(
            smoke_bass(&freq),
            (0..BASS_BINS).map(|i| 0.5 + i as f32 * 0.1).sum::<f32>() / BASS_BINS as f32,
            1e-6,
        );

        let snare_lo = (AUDIO_SAMPLES as f32 * 0.02) as usize;
        let snare_hi = (AUDIO_SAMPLES as f32 * 0.13) as usize;
        freq[snare_lo] = 1.0;
        assert!(snare_band(&freq) > 0.0);
        close(snare_band(&freq), 1.0 / (snare_hi - snare_lo) as f32, 1e-6);

        freq = [0.0; AUDIO_SAMPLES];
        freq[AUDIO_SAMPLES - 1] = 1.0;
        assert!(treble_band(&freq) > 0.0);
        close(
            treble_band(&freq),
            1.0 / (AUDIO_SAMPLES - (AUDIO_SAMPLES as f32 * 0.85) as usize) as f32,
            1e-6,
        );

        assert_eq!(fire_size(0.0, 0.0), 1.0);
        close(fire_size(0.1, 0.0), 1.25, 1e-6);
        close(fire_size(0.0, 0.1), 1.8, 1e-6);
        assert!(fire_size(0.5, 0.5) > 6.0);
    }

    #[test]
    fn the_fire_ramp_runs_red_mid_start() {
        let red = fire_ramp(0.2);
        assert!(red[0] > red[1] && red[0] > red[2]);
        let mid = fire_ramp(0.5);
        assert!(mid[0] > mid[1]);
        let hot = fire_ramp(0.95);
        assert!(hot[1] > 0.5);
        // fireRamp(0.2) is the end colour, fireRamp(1.0) is the start colour.
        let start = srgb_hex_to_linear(FIRE_START_HEX);
        close(hot[1] / hot[0], start[1] / start[0], 0.05);
    }

    #[test]
    fn every_roygbiv_hue_reads_as_bright_as_green() {
        let reference = fire_colour(0.5, ROYGBIV_HUES[4]);
        let reference_luminance =
            0.2126 * reference[0] + 0.7152 * reference[1] + 0.0722 * reference[2];
        for hue in ROYGBIV_HUES {
            let colour = fire_colour(0.5, hue);
            let luminance = 0.2126 * colour[0] + 0.7152 * colour[1] + 0.0722 * colour[2];
            close(luminance, reference_luminance, 1e-4);
        }
        // `fireHue` always equalises to the green hue's luminance, so even
        // hue 0 rescales the ramp colour (the JS applies it unconditionally).
        let base = fire_ramp(0.5);
        let unscaled = fire_colour(0.5, 0.0);
        assert!((unscaled[0] / base[0] - 1.0).abs() > 1e-3);
    }

    #[test]
    fn improved_noise_matches_the_perlin_reference() {
        let noise = ImprovedNoise;
        // Deterministic, in [-1, 1], zero at integer lattice points.
        for i in 0..64 {
            let x = i as f32 * 0.37;
            let value = noise.noise(x, x * 0.5, x * 0.25);
            assert!(value.abs() <= 1.0, "noise out of range: {value}");
        }
        close(noise.noise(0.0, 0.0, 0.0), 0.0, 1e-6);
        close(noise.noise(1.0, 2.0, 3.0), 0.0, 1e-6);
        // The default flicker sits near the JS's 0.82 base.
        let temporal = fire_temporal(&noise, 0.0);
        close(temporal.flicker, 0.82, 1e-6);
        close(temporal.flame_height, 3.5, 1e-6);
        assert_eq!(temporal.sway, [0.0, 0.0, 0.0]);
    }

    #[test]
    fn the_volume_point_light_picks_up_the_fade_in() {
        let noise = ImprovedNoise;
        let early = fire_temporal(&noise, 0.0);
        let late = fire_temporal(&noise, 10.0);
        let early_light = volume_point_light(&early, 22.0, 28.0, 40.0, 0.0);
        let late_light = volume_point_light(&late, 22.0, 28.0, 40.0, 0.0);
        assert_eq!(early_light, [0.0, 0.0, 0.0]);
        assert!(late_light[0] > 0.0 && late_light[0] > late_light[1]);
        // Hue flips keep the energy (luminance) equal.
        let blue = volume_point_light(&late, 22.0, 28.0, 40.0, ROYGBIV_HUES[4]);
        let warm = 0.2126 * late_light[0] + 0.7152 * late_light[1] + 0.0722 * late_light[2];
        let cool = 0.2126 * blue[0] + 0.7152 * blue[1] + 0.0722 * blue[2];
        close(warm, cool, warm * 0.02);
        // The 0.2 factor is the lava/logo material's, not the light's.
        assert!(
            late_light[0] > 5.0 * emitter_glow(22.0, 28.0, 40.0, &late),
            "volume light must be much brighter than the surface emissive"
        );

        // The intensity is the scene's `uPointLightVolumeIntensity`: the room
        // scenes use 2, RollingFire 1.4.
        let fire = volume_point_light_scaled(&late, 14.0, 16.0, 26.0, 0.0, 1.4);
        let two = volume_point_light_scaled(&late, 14.0, 16.0, 26.0, 0.0, 2.0);
        close(two[0] / fire[0], 2.0 / 1.4, 1e-5);
        assert_eq!(fade_in(0.0), 0.0);
        close(fade_in(1.5), 0.5, 1e-6);
        close(fade_in(3.0), 1.0, 1e-6);
        close(fade_in(9.0), 1.0, 1e-6);
    }

    #[test]
    fn the_clock_runs_a_fixed_120hz_step_capped_at_eight() {
        let mut clock = SimulationClock::default();
        let tick = clock.pending(1.0 / 60.0, 1.2, 3.5, 1.3);
        // 1/60 s * 1.2 = 0.02 s of sim time, at 0.01 s per step: 2 steps.
        assert_eq!(tick.steps, 2);
        close(tick.step, 0.01, 1e-7);
        close(tick.turbulence, 3.2 / 1.2_f32.sqrt(), 1e-6);
        close(tick.dissipation, 1.0 / 3.5, 1e-6);
        close(tick.cooling, 1.0 / 1.3, 1e-6);
        close(clock.time(), 0.02, 1e-6);
        close(tick.time_of(0), 0.01, 1e-6);
        close(tick.time_of(1), 0.02, 1e-6);

        // The frame delta is capped at 1/30 s before it reaches the
        // accumulator (`_animate`'s `Math.min(this._dt, 1 / 30)`), so at most
        // four 1/120 s steps fit at any speed.
        let mut clock = SimulationClock::default();
        let tick = clock.pending(1.0, 1.2, 3.5, 1.3);
        assert_eq!(tick.steps, 4);
        let tick = clock.pending(1.0, 4.0, 3.5, 1.3);
        assert_eq!(tick.steps, 4);

        // A 100 s smoke lifespan means "no dissipation"; 0 speed idles.
        let mut clock = SimulationClock::default();
        let tick = clock.pending(1.0 / 60.0, 1.2, 100.0, 1.3);
        assert_eq!(tick.dissipation, 0.0);
        let tick = clock.pending(1.0 / 60.0, 0.0, 3.5, 1.3);
        assert_eq!(tick.steps, 0);
        assert_eq!(tick.turbulence, 0.0);
    }
}
