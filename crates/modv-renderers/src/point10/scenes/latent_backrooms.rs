//! LatentBackrooms — Rust/wgpu port of
//! `src/renderer/src/application/point10/scenes/LatentBackrooms.js`.
//!
//! Twelve procedural rooms are stitched into one endless corridor. The JS scene
//! is a WebGL `EffectComposer` chain — `RenderPass → AfterimagePass(0.78) →
//! UnrealBloomPass(0.8, 0.9, 0.12) → CRT ShaderPass → OutputPass` — walking a
//! demo camera down the corridor automatically (the source's pointer-lock input
//! is gone in the worker).
//!
//! # Ported exactly
//!
//! * the room plan (`:127-148`): the four seeded `hash()` draws per room
//!   (`width = 15 + a * 13`, `height = 8 + b * 14`, `depth = 40 + c * 10`,
//!   `palette = floor(i / 3) % 4`, `phase = e * TAU`,
//!   `complexity = 2 + floor(f * 5)`, `bend = (g - 0.5) * 0.7`), the name/kind/
//!   code tables and the four palettes;
//! * the corridor treadmill (`:150-303`): [`room_cycle`], the 56-unit spacing,
//!   `START_Z = 10`, [`latent_room_layout`]'s slot reuse, and the camera Z;
//! * the automatic drive (`:247-280`): [`walk_speed`] (`6.2 + bass*6.5 +
//!   mid*2 + beat*3.2`), [`ripple_speed`], the `0.1 s` frame clamp, and the
//!   `travel`/`ripplePhase` integration;
//! * the camera path (`:196-242`, `:1699-1753`): [`walk_vector`]'s seven sway
//!   terms and clamps, the 8/4.8 spring integration, the look-ahead at
//!   `travel + 9`, the gaze spring, the `runBob`/`runSway`/`roll` head motion
//!   and the run FOV (`68 + clamp((speed - 7) * 0.7, 0, 5) + bass * 2.4 +
//!   flash * 1.5`);
//! * the audio mapping (`:1655-1700`): the asymmetric one-poles
//!   (`bass 10/3.5`, `mid 9/3.5`, `high 8/3.5`), `flash = max(beat, flash *
//!   exp(-dt * 3.8))`, `morph = max(beat * 0.8, morph * exp(-dt * 2.8))`, the
//!   turn target and the `2.8` smoothing;
//! * the lighting rig (`:1793-1892`): [`lighting_drive`]'s eight levels applied
//!   to the wall/trim/panel/fixture/line/floor materials, the palette
//!   background + `FogExp2` (`density = 0.009 + bass*0.001 + flash*0.001`), the
//!   walk/rim point lights' positions and intensities, the hemisphere/ambient/
//!   fill levels, the afterimage damp
//!   (`clamp((0.48 + high*0.08 + flash*0.04) * trailScale, 0, 0.95)`) and the
//!   bloom strength (`clamp(drive.bloom * bloomScale, 0.35, 2.2)`);
//! * the chevron grid (`:605-673`, `:1762-1791`): six markers at
//!   [`chevron_grid_z`], each a plate plus the 13 pixel strip, with the
//!   ripple-driven opacity `clamp(0.02 + pulse * rippleGain * 0.18 + bass *
//!   0.012, 0.012, 0.24)` and scale `0.94 + pulse * 0.1 + flash * 0.04`;
//! * the dust (`:948-978`): per-room seeded positions, additive, rotating with
//!   the high band;
//! * the CRT grade (`:1609-1644`): the scanline `0.91 + 0.09 *
//!   sin(vUv.y * resolution.y * PI)`, the radial vignette `smoothstep(0.32,
//!   0.92, length(centred))`, the `sin(dot(...))` grain, `intensity 0.72` and
//!   `exposure = brightnessScale`, alpha forced to 1;
//! * the props (`sceneProps.js:186-191`): `speed`, `bloom`, `trail`,
//!   `brightness` → `speedScale`/`bloomScale`/`trailScale`/`brightnessScale`.
//!
//! # Substitutions
//!
//! | three.js | Rust |
//! |----------|------|
//! | `BeforeImage` canvas textures (glyph walls, plaques) | a procedural hash-glyph pattern in the wall/floor shader; plaques are small emissive plates |
//! | `MeshStandardMaterial` PBR + shadow-free lights | a hemi/ambient/fill + two point lights shader with the same palette colours and drive levels |
//! | `AfterimagePass` | [`AfterimagePass`] (reusable, damped ping-pong) |
//! | `UnrealBloomPass` | [`UnrealBloomPass`] |
//! | `ShaderPass` CRT + `OutputPass` | one grade pass combining `afterimage + bloom` with the CRT shader, then [`Composer`]'s output pass (`NoToneMapping` + sRGB) |
//! | `RoomGeometry` counts (90+ books/cards per room) | reduced detail counts per room kind (see the per-kind builders); the placements and palettes are the JS's |
//! | `Sprite` plaques with code text | small emissive plates (no text rasteriser) |
//! | `Points` dust with a glow sprite | small additive boxes |
//! | `Math.random`/`Math.sin` seeded hashes | the same `hash()` (it is already deterministic), [`seeded_rand`] only for the puff-style seeds that are not in the JS |
//!
//! # Transparency
//!
//! The JS sets `scene.background = new THREE.Color(0x050505)` and the CRT pass
//! writes `alpha = 1.0`, so the module is **opaque**. The port clears the scene
//! target to the current palette background with alpha 1 and the grade pass
//! returns alpha 1; the e2e/alpha test asserts every pixel is opaque.

#![forbid(unsafe_code)]

use crate::point10::audio_bridge::{AudioBridge, P10Audio};
use crate::point10::postfx::{AfterimagePass, Composer, ToneMapping, UnrealBloomPass};
use crate::point10::scenes::cube_mesh::{
    Mat4, MeshGpu, Vertex, create_depth, depth_state, identity, look_at, mat4_mul, perspective,
    rotation_y, rotation_z, scale, translation, trs,
};
use crate::point10::shared::phrase_color::srgb_hex_to_linear;
use crate::point10::{
    FULLSCREEN_TRIANGLE_WGSL, HDR_FORMAT, Point10Scene, SceneContext, SceneMeta, SceneProp,
    frame_dt, read_scene_props, scene_props_with_levers,
};

// ---------------------------------------------------------------------------
// Constants and pure maths (`LatentBackrooms.js:23-303`)
// ---------------------------------------------------------------------------

/// `TAU`.
pub const TAU: f32 = std::f32::consts::TAU;
/// `ROOM_COUNT`.
pub const ROOM_COUNT: usize = 12;
/// `latentRoomSpacing`.
pub const LATENT_ROOM_SPACING: f32 = 56.0;
/// `START_Z`.
pub const START_Z: f32 = 10.0;
/// `MAX_DRIVE_DT`.
pub const MAX_DRIVE_DT: f32 = 0.1;

/// `ROOM_NAMES`.
pub const ROOM_NAMES: [&str; ROOM_COUNT] = [
    "The Yellow Vestibule",
    "The Long Library",
    "Hall Of Unsaid Things",
    "Server Nave",
    "The Forked Archive",
    "The Room That Waits",
    "Cathedral Of Context",
    "The Blue Corridor",
    "Garden Of Recurrence",
    "The Empty Wing",
    "Prediction Chapel",
    "The Quiet Wing",
];

/// `ROOM_KINDS`.
pub const ROOM_KINDS: [&str; ROOM_COUNT] = [
    "backrooms",
    "library",
    "hall",
    "server",
    "maze",
    "archive",
    "cathedral",
    "server",
    "garden",
    "void",
    "chapel",
    "backrooms",
];

/// `ROOM_CODES`.
pub const ROOM_CODES: [&str; ROOM_COUNT] = [
    "VESTIBULE / 00",
    "LIBRARY / 17",
    "HALL / 04",
    "SERVER NAVE / 88",
    "FORKED ARCHIVE / 31",
    "WAITING ROOM / 09",
    "CONTEXT CATHEDRAL / 77",
    "BLUE CORRIDOR / 12",
    "RECURRENCE GARDEN / 23",
    "EMPTY WING / 00",
    "PREDICTION CHAPEL / 41",
    "QUIET WING / 01",
];

/// One palette (`:69-102`), as hex, matching the JS.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    /// `background`.
    pub background: u32,
    /// `wall`.
    pub wall: u32,
    /// `ink`.
    pub ink: u32,
    /// `light`.
    pub light: u32,
    /// `accent`.
    pub accent: u32,
    /// `trim`.
    pub trim: u32,
}

/// `PALETTES`.
pub const PALETTES: [Palette; 4] = [
    Palette {
        background: 0x0d0b07,
        wall: 0x393021,
        ink: 0xe0c47e,
        light: 0xffe0a0,
        accent: 0xa98cff,
        trim: 0x17130d,
    },
    Palette {
        background: 0x050b12,
        wall: 0x102431,
        ink: 0x8ed6e5,
        light: 0xa9eaff,
        accent: 0xff70b7,
        trim: 0x081016,
    },
    Palette {
        background: 0x100609,
        wall: 0x32151a,
        ink: 0xefa39b,
        light: 0xffc09b,
        accent: 0x7f9cff,
        trim: 0x16090d,
    },
    Palette {
        background: 0x07100b,
        wall: 0x14251c,
        ink: 0xa1e2bb,
        light: 0xcaffd1,
        accent: 0xffb45e,
        trim: 0x0a140e,
    },
];

/// `ROOM_WORDS` — `latentRoomWords`.
pub const ROOM_WORDS: [&str; 10] = [
    "LATENT",
    "CONTEXT",
    "MEMORY",
    "LOSS",
    "ATTENTION",
    "PREDICTION",
    "PATTERN",
    "SILENCE",
    "RELATION",
    "ORIGIN",
];

/// `fract`.
pub fn fract(value: f32) -> f32 {
    value - value.floor()
}

/// `mod`.
pub fn js_mod(value: f32, modulus: f32) -> f32 {
    let result = value % modulus;
    if result < 0.0 {
        result + modulus
    } else {
        result
    }
}

/// `clamp`.
pub fn clamp(value: f32, low: f32, high: f32) -> f32 {
    low.max(high.min(value))
}

/// `hash(seed)` (`:122`).
pub fn hash(seed: f32) -> f32 {
    fract((seed * 127.1 + 311.7).sin() * 43_758.545_3)
}

/// `finite(v, fallback)`.
pub fn finite(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

/// `unit(v)`.
pub fn unit(value: f32) -> f32 {
    clamp(finite(value, 0.0), 0.0, 1.0)
}

/// One room plan (`createRoomPlan`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoomPlan {
    /// `index`.
    pub index: usize,
    /// `kind`.
    pub kind: &'static str,
    /// `name`.
    pub name: &'static str,
    /// `code`.
    pub code: &'static str,
    /// `width`.
    pub width: f32,
    /// `height`.
    pub height: f32,
    /// `depth`.
    pub depth: f32,
    /// `palette` index.
    pub palette: usize,
    /// `phase`.
    pub phase: f32,
    /// `complexity`.
    pub complexity: u32,
    /// `bend`.
    pub bend: f32,
}

/// `createRoomPlan(index)` (`:127-148`).
pub fn create_room_plan(index: usize) -> RoomPlan {
    let i = index;
    let a = hash(i as f32 * 17.13 + 1.7);
    let b = hash(i as f32 * 31.71 + 8.2);
    let c = hash(i as f32 * 47.29 + 3.4);
    let e = hash(i as f32 * 73.41 + 4.6);
    let f = hash(i as f32 * 89.73 + 6.8);
    let g = hash(i as f32 * 101.9 + 2.2);
    RoomPlan {
        index: i,
        kind: ROOM_KINDS[i % ROOM_KINDS.len()],
        name: ROOM_NAMES[i % ROOM_NAMES.len()],
        code: ROOM_CODES[i % ROOM_CODES.len()],
        width: 15.0 + a * 13.0,
        height: 8.0 + b * 14.0,
        depth: 40.0 + c * 10.0,
        palette: (i / 3) % PALETTES.len(),
        phase: e * TAU,
        complexity: 2 + (f * 5.0) as u32,
        bend: (g - 0.5) * 0.7,
    }
}

impl RoomPlan {
    /// The palette for this room.
    pub fn palette(&self) -> Palette {
        PALETTES[self.palette % PALETTES.len()]
    }
}

/// `roomCycle(travel)` (`:150-151`).
pub fn room_cycle(travel: f32) -> i32 {
    let travel = finite(travel, 0.0);
    ((travel + LATENT_ROOM_SPACING * 0.5) / LATENT_ROOM_SPACING).floor() as i32
}

/// `walkSpeed(bass, mid, beat)` (`:153-157`).
pub fn walk_speed(bass: f32, mid: f32, beat: f32) -> f32 {
    6.2 + clamp(bass, 0.0, 1.0) * 6.5 + clamp(mid, 0.0, 1.0) * 2.0 + clamp(beat, 0.0, 1.0) * 3.2
}

/// `runBob(travel, phase)`.
pub fn run_bob(travel: f32, phase: f32) -> f32 {
    (travel * 2.35 + phase).sin() * 0.06 + (travel * 4.7 + phase * 0.7).sin() * 0.018
}

/// `runSway(travel, phase)`.
pub fn run_sway(travel: f32, phase: f32) -> f32 {
    (travel * 1.18 + phase * 0.5).sin() * 0.035
}

/// `hallwayRipple(phase, z)`.
pub fn hallway_ripple(phase: f32, z: f32) -> f32 {
    let wave = (phase + z * 0.32).sin().max(0.0);
    wave * wave * wave
}

/// `chevronGridZ(cameraZ, index, spacing)`.
pub fn chevron_grid_z(camera_z: f32, index: usize, spacing: f32) -> f32 {
    let first = (-camera_z / spacing).ceil() + 1.0;
    -(first + index as f32) * spacing
}

/// `rippleSpeed(bass, mid, beat)`.
pub fn ripple_speed(bass: f32, mid: f32, beat: f32) -> f32 {
    22.0 + clamp(bass, 0.0, 1.0) * 28.0 + clamp(mid, 0.0, 1.0) * 10.0 + clamp(beat, 0.0, 1.0) * 35.0
}

/// `lightingDrive(bass, mid, high, beat)` (`:179-194`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightingDrive {
    /// `walk`.
    pub walk: f32,
    /// `rim`.
    pub rim: f32,
    /// `wall`.
    pub wall: f32,
    /// `panel`.
    pub panel: f32,
    /// `ceiling`.
    pub ceiling: f32,
    /// `line`.
    pub line: f32,
    /// `floor`.
    pub floor: f32,
    /// `bloom`.
    pub bloom: f32,
}

/// `lightingDrive`.
pub fn lighting_drive(bass: f32, mid: f32, high: f32, beat: f32) -> LightingDrive {
    let b = clamp(bass, 0.0, 1.0);
    let m = clamp(mid, 0.0, 1.0);
    let h = clamp(high, 0.0, 1.0);
    let k = clamp(beat, 0.0, 1.0);
    LightingDrive {
        walk: 1.2 + b * 15.0 + m * 5.0 + k * 10.0,
        rim: 0.8 + m * 10.0 + h * 8.0 + k * 9.0,
        wall: 0.22 + b * 1.1 + m * 0.7 + k * 0.55,
        panel: 0.08 + h * 0.55 + k * 0.28,
        ceiling: 0.28 + b * 0.65 + m * 0.2 + k * 0.35,
        line: 0.07 + h * 0.3 + m * 0.08,
        floor: 0.05 + b * 0.16 + k * 0.08,
        bloom: 0.45 + b * 1.25 + m * 0.65 + k,
    }
}

/// `walkVector(travel, plan, …)`'s `{x, y}`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WalkPoint {
    /// `x`.
    pub x: f32,
    /// `y`.
    pub y: f32,
}

/// `walkVector` (`:196-242`).
pub fn walk_vector(
    travel: f32,
    plan: &RoomPlan,
    energy: f32,
    mid: f32,
    high: f32,
    beat: f32,
    turn: f32,
) -> WalkPoint {
    let bass = clamp(energy, 0.0, 1.0);
    let middle = clamp(mid, 0.0, 1.0);
    let treble = clamp(high, 0.0, 1.0);
    let kick = clamp(beat, 0.0, 1.0);
    let sway = (travel * 0.14 + plan.phase).sin() * plan.width * 0.075;
    let float_sway = (travel * 0.39 + plan.phase * 1.7).sin() * plan.width * 0.045;
    let drift = (travel * 0.047 + plan.phase * 0.7).sin() * plan.width * 0.035;
    let bass_turn = bass * (travel * 0.29 + plan.phase).sin() * plan.width * 0.035;
    let mid_turn = middle * (travel * 0.095 + plan.phase * 1.2).sin() * plan.width * 0.05;
    let kick_turn = kick * (travel * 0.18 + plan.phase).cos() * plan.width * 0.035;
    let guided_turn = turn * plan.width * 0.035;
    let room_bend = plan.bend * (travel * 0.07 + plan.phase).sin() * plan.width * 0.025;
    WalkPoint {
        x: clamp(
            sway + float_sway + drift + bass_turn + mid_turn + kick_turn + guided_turn + room_bend,
            -plan.width * 0.22,
            plan.width * 0.22,
        ),
        y: 1.72
            + (travel * 0.27 + plan.phase).sin() * (0.08 + bass * 0.1)
            + (travel * 0.51 + plan.phase * 0.6).sin() * 0.035
            + treble * 0.06,
    }
}

/// One frame of the automatic camera drive (`latentBackroomsDrive`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Drive {
    /// The clamped step.
    pub step: f32,
    /// The walk speed.
    pub speed: f32,
    /// The new travel.
    pub travel: f32,
    /// The new ripple phase.
    pub ripple_phase: f32,
    /// The new room cycle.
    pub cycle: i32,
    /// `START_Z - travel`.
    pub camera_z: f32,
}

/// `latentBackroomsDrive(args)` (`:247-280`).
pub fn latent_backrooms_drive(
    travel: f32,
    ripple_phase: f32,
    dt: f32,
    bass: f32,
    mid: f32,
    beat: f32,
    speed_scale: f32,
) -> Drive {
    let step = clamp(finite(dt, 0.0), 0.0, MAX_DRIVE_DT);
    let b = unit(bass);
    let m = unit(mid);
    let k = unit(beat);
    let scale = if speed_scale.is_finite() && speed_scale >= 0.0 {
        speed_scale
    } else {
        1.0
    };
    let speed = walk_speed(b, m, k) * scale;
    let next_travel = finite(travel, 0.0) + step * speed;
    let next_ripple = js_mod(
        finite(ripple_phase, 0.0) + step * ripple_speed(b, m, k) * 0.32,
        TAU,
    );
    Drive {
        step,
        speed,
        travel: next_travel,
        ripple_phase: next_ripple,
        cycle: room_cycle(next_travel),
        camera_z: START_Z - next_travel,
    }
}

/// One live room placement (`latentRoomLayout`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoomPlacement {
    /// `slot`.
    pub slot: usize,
    /// `absolute`.
    pub absolute: i32,
    /// `index`.
    pub index: usize,
    /// `z`.
    pub z: f32,
}

/// The room treadmill (`latentRoomLayout`).
#[derive(Clone, Debug, PartialEq)]
pub struct RoomLayout {
    /// `cycle`.
    pub cycle: i32,
    /// `current`.
    pub current: usize,
    /// `cameraZ`.
    pub camera_z: f32,
    /// `rooms`.
    pub rooms: Vec<RoomPlacement>,
}

/// `latentRoomLayout(travel)` (`:284-303`).
pub fn latent_room_layout(travel: f32) -> RoomLayout {
    let travel = finite(travel, 0.0);
    let cycle = room_cycle(travel);
    let mut rooms = Vec::with_capacity(ROOM_COUNT);
    for slot in 0..ROOM_COUNT {
        let absolute = cycle + slot as i32;
        rooms.push(RoomPlacement {
            slot,
            absolute,
            index: js_mod(absolute as f32, ROOM_COUNT as f32) as usize,
            z: START_Z - absolute as f32 * LATENT_ROOM_SPACING,
        });
    }
    RoomLayout {
        cycle,
        current: js_mod(cycle as f32, ROOM_COUNT as f32) as usize,
        camera_z: START_Z - travel,
        rooms,
    }
}

// ---------------------------------------------------------------------------
// GPU types
// ---------------------------------------------------------------------------

/// `LATENT_BACKROOMS_PROPS` (`sceneProps.js:186-191`).
const LATENT_BACKROOMS_PROPS: &[SceneProp] = &[
    SceneProp::float("speed", "Camera Speed", 1.0, 0.0, 4.0, 0.01),
    SceneProp::float("bloom", "Bloom", 1.0, 0.0, 3.0, 0.01),
    SceneProp::float("trail", "Afterimage Trail", 1.0, 0.0, 3.0, 0.01),
    SceneProp::float("brightness", "Brightness", 1.0, 0.0, 3.0, 0.01),
];

/// The scene uniform block (192 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct SceneUniforms {
    view_proj: Mat4,
    camera_pos: [f32; 4],
    /// rgb = palette background, w = fog density.
    background: [f32; 4],
    /// x = hemi, y = ambient, z = fill, w = line.
    lights: [f32; 4],
    /// xyz = walk light position, w = intensity.
    walk: [f32; 4],
    /// rgb = walk colour.
    walk_colour: [f32; 4],
    /// xyz = rim light position, w = intensity.
    rim: [f32; 4],
    /// rgb = rim colour.
    rim_colour: [f32; 4],
    /// x = time, y = flash, z = unused.
    params: [f32; 4],
}

/// The CRT grade uniform block (32 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct GradeUniforms {
    resolution: [f32; 2],
    time: f32,
    intensity: f32,
    /// x = exposure (the `brightness` lever).
    params: [f32; 4],
}

/// One geometry instance (96 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct Instance {
    model: Mat4,
    /// rgb = albedo/emissive colour, a = opacity.
    colour: [f32; 4],
    /// x = emissive intensity, y = material mode.
    params: [f32; 4],
}

impl Instance {
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

/// Material modes the shader understands.
pub const MODE_TRIM: u32 = 0;
/// Glyph wall.
pub const MODE_WALL: u32 = 1;
/// Floor with grid + glyphs.
pub const MODE_FLOOR: u32 = 2;
/// Unlit additive panel.
pub const MODE_PANEL: u32 = 3;
/// Lit accent.
pub const MODE_ACCENT: u32 = 4;
/// Lit book/card.
pub const MODE_BOOK: u32 = 5;
/// Additive fixture light.
pub const MODE_LIGHT: u32 = 6;
/// Additive dust/plaque.
pub const MODE_DUST: u32 = 7;

/// One geometry part's animation class.
#[derive(Clone, Copy, Debug, PartialEq)]
enum DynKind {
    Static,
    Fixture {
        local_z: f32,
    },
    Bar {
        phase: f32,
        pos: f32,
        base_y: f32,
    },
    Mover {
        phase: f32,
        amplitude: f32,
        axis: u8,
    },
    Core {
        phase: f32,
        position: [f32; 3],
        base_scale: [f32; 3],
    },
    Plaque {
        phase: f32,
        base_y: f32,
    },
}

/// One static/animated piece of a room (room-local coordinates).
#[derive(Clone, Copy, Debug)]
struct Part {
    model: Mat4,
    colour: [f32; 3],
    mode: u32,
    emissive: f32,
    alpha: f32,
    dynamic: DynKind,
}

/// The 12 built rooms.
struct Room {
    plan: RoomPlan,
    parts: Vec<Part>,
}

/// GPU resources built in `init`.
struct LatentGpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    additive_pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    mesh: MeshGpu,
    instance_buffer: wgpu::Buffer,
    instance_capacity: usize,
    additive_buffer: wgpu::Buffer,
    additive_capacity: usize,
    scene_texture: wgpu::Texture,
    scene_view: wgpu::TextureView,
    _depth: wgpu::Texture,
    depth_view: wgpu::TextureView,
    afterimage: AfterimagePass,
    bloom: UnrealBloomPass,
    grade_pipeline: wgpu::RenderPipeline,
    grade_uniforms: wgpu::Buffer,
    grade_layout: wgpu::BindGroupLayout,
    grade_bind_group: wgpu::BindGroup,
    grade_sampler: wgpu::Sampler,
    composer: Composer,
}

/// LatentBackrooms scene.
pub struct LatentBackrooms {
    /// `this.travel`.
    travel: f32,
    /// `this.ripplePhase`.
    ripple_phase: f32,
    /// `this.rippleSpeed`.
    ripple_speed: f32,
    /// `this.walkX`.
    walk_x: f32,
    /// `this.walkY`.
    walk_y: f32,
    /// `this.walkVX`.
    walk_vx: f32,
    /// `this.walkVY`.
    walk_vy: f32,
    /// `this.lookX`.
    look_x: f32,
    /// `this.lookY`.
    look_y: f32,
    /// `this.lookVX`.
    look_vx: f32,
    /// `this.lookVY`.
    look_vy: f32,
    /// `this.gazeX`.
    gaze_x: f32,
    /// `this.gazeY`.
    gaze_y: f32,
    /// `this.gazeVX`.
    gaze_vx: f32,
    /// `this.gazeVY`.
    gaze_vy: f32,
    /// `this.turn`.
    turn: f32,
    /// `this.roll`.
    roll: f32,
    /// `this.speed`.
    speed: f32,
    /// `this.headBob`.
    head_bob: f32,
    /// `this.shoulderSway`.
    shoulder_sway: f32,
    /// `this.smBass`.
    smoothed_bass: f32,
    /// `this.smMid`.
    smoothed_mid: f32,
    /// `this.smHigh`.
    smoothed_high: f32,
    /// `this.flash`.
    flash: f32,
    /// `this.morph`.
    morph: f32,
    /// `this.time` (`BaseScene.update` accumulates the frame delta).
    time: f32,
    /// `this.p9` — the P9Audio reader the bar/shape reads use.
    p10: P10Audio,
    /// `this.currentRoom`.
    current_room: usize,
    /// `this.rooms`.
    rooms: Vec<Room>,
    /// Levers.
    speed_scale: f32,
    bloom_scale: f32,
    trail_scale: f32,
    brightness_scale: f32,
    camera_aspect: f32,
    size: (u32, u32),
    gpu: Option<LatentGpu>,
}

impl Default for LatentBackrooms {
    fn default() -> Self {
        Self::new()
    }
}

impl LatentBackrooms {
    /// Create the scene (`LatentBackrooms.js:385-456` defaults).
    pub fn new() -> Self {
        let rooms = (0..ROOM_COUNT)
            .map(|index| build_room(create_room_plan(index)))
            .collect();
        Self {
            travel: 0.0,
            ripple_phase: 0.0,
            ripple_speed: 0.0,
            walk_x: 0.0,
            walk_y: 1.72,
            walk_vx: 0.0,
            walk_vy: 0.0,
            look_x: 0.0,
            look_y: 1.72,
            look_vx: 0.0,
            look_vy: 0.0,
            gaze_x: 0.0,
            gaze_y: 0.0,
            gaze_vx: 0.0,
            gaze_vy: 0.0,
            turn: 0.0,
            roll: 0.0,
            speed: 0.0,
            head_bob: 0.0,
            shoulder_sway: 0.0,
            smoothed_bass: 0.0,
            smoothed_mid: 0.0,
            smoothed_high: 0.0,
            flash: 0.0,
            morph: 0.0,
            time: 0.0,
            p10: P10Audio::new(),
            current_room: 0,
            rooms,
            speed_scale: 1.0,
            bloom_scale: 1.0,
            trail_scale: 1.0,
            brightness_scale: 1.0,
            camera_aspect: 1.0,
            size: (1, 1),
            gpu: None,
        }
    }

    /// Apply the resolved modV props (`sceneProps.js:186-191`).
    pub fn apply_props(&mut self, props: impl FnMut(&str) -> Option<f32>) {
        for (key, value) in read_scene_props(LATENT_BACKROOMS_PROPS, props) {
            match key {
                "speed" => self.speed_scale = value,
                "bloom" => self.bloom_scale = value,
                "trail" => self.trail_scale = value,
                "brightness" => self.brightness_scale = value,
                _ => {}
            }
        }
    }

    /// `this.travel`.
    pub fn travel(&self) -> f32 {
        self.travel
    }

    /// `this.currentRoom` (the room plan index).
    pub fn current_room(&self) -> usize {
        self.current_room
    }

    /// `this.speed`.
    pub fn speed(&self) -> f32 {
        self.speed
    }

    /// The number of built rooms.
    pub fn room_count(&self) -> usize {
        self.rooms.len()
    }

    /// The afterimage damp for the current frame.
    pub fn afterimage_damp(&self) -> f32 {
        clamp(
            (0.48 + self.smoothed_high * 0.08 + self.flash * 0.04) * self.trail_scale,
            0.0,
            0.95,
        )
    }

    /// The bloom strength for the current frame.
    pub fn bloom_strength(&self) -> f32 {
        clamp(
            lighting_drive(
                self.smoothed_bass,
                self.smoothed_mid,
                self.smoothed_high,
                self.flash,
            )
            .bloom
                * self.bloom_scale,
            0.35,
            2.2,
        )
    }

    /// The camera position (`render`, `:1896-1901`).
    pub fn camera_position(&self) -> [f32; 3] {
        [
            self.walk_x + self.shoulder_sway,
            self.walk_y + self.head_bob,
            START_Z - self.travel,
        ]
    }

    /// The camera FOV (`:1908-1909`).
    pub fn camera_fov(&self) -> f32 {
        let run_fov = clamp((self.speed - 7.0) * 0.7, 0.0, 5.0);
        68.0 + run_fov + self.smoothed_bass * 2.4 + self.flash * 1.5
    }

    fn plan(&self, index: usize) -> &RoomPlan {
        &self.rooms[index % self.rooms.len()].plan
    }

    fn camera(&self) -> ([f32; 3], [f32; 3], f32) {
        let camera = self.camera_position();
        let look = [
            self.look_x + self.gaze_x,
            self.look_y + self.gaze_y,
            START_Z - self.travel - 10.0,
        ];
        (camera, look, self.roll)
    }

    fn view_proj(&self) -> Mat4 {
        let (camera, look, roll) = self.camera();
        let view = mat4_mul(&rotation_z(-roll), &look_at(camera, look, [0.0, 1.0, 0.0]));
        mat4_mul(
            &perspective(
                self.camera_fov().to_radians(),
                self.camera_aspect,
                0.1,
                1000.0,
            ),
            &view,
        )
    }

    /// Build the frame's instance lists (opaque, additive).
    fn collect_instances(&self) -> (Vec<Instance>, Vec<Instance>) {
        let layout = latent_room_layout(self.travel);
        let ripple_gain =
            0.12 + self.smoothed_bass * 1.4 + self.smoothed_mid * 0.6 + self.flash * 0.9;
        let camera_z = START_Z - self.travel;
        let mut opaque: Vec<Instance> = Vec::with_capacity(ROOM_COUNT * 80);
        let mut additive: Vec<Instance> = Vec::with_capacity(ROOM_COUNT * 12 + 128);

        for placement in &layout.rooms {
            let room = &self.rooms[placement.index % self.rooms.len()];
            let room_model = translation(0.0, 0.0, placement.z);
            let distance = (placement.z - camera_z).abs();
            let near = clamp(1.0 - distance / (LATENT_ROOM_SPACING * 1.4), 0.0, 1.0);

            for part in &room.parts {
                let (model, colour, alpha, emissive) = match part.dynamic {
                    DynKind::Static => (part.model, part.colour, part.alpha, part.emissive),
                    DynKind::Fixture { local_z } => {
                        let pulse = hallway_ripple(self.ripple_phase, placement.z + local_z);
                        (
                            part.model,
                            part.colour,
                            clamp(0.035 + pulse * ripple_gain, 0.0, 1.0),
                            part.emissive,
                        )
                    }
                    DynKind::Bar { phase, pos, base_y } => {
                        // `this.p9?.spectrum(-1, bar.pos)` clamped to 0..1.
                        let value = unit(self.p10.spectrum(-1, pos, 0.0));
                        let lift = 0.86 + value * 0.24 + self.smoothed_bass * 0.04;
                        let bob = (self.time * 1.2 + phase).sin() * 0.012;
                        let mut model = mat4_mul(&translation(0.0, bob, 0.0), &part.model);
                        model = mat4_mul(&model, &scale(1.0, lift, 1.0));
                        let _ = base_y;
                        (model, part.colour, part.alpha, part.emissive)
                    }
                    DynKind::Mover {
                        phase,
                        amplitude,
                        axis,
                    } => {
                        let wave = (self.time * (0.28 + self.smoothed_mid * 0.5) + phase).sin()
                            * amplitude;
                        let offset = if axis == 0 {
                            translation(wave, 0.0, 0.0)
                        } else {
                            translation(0.0, wave, 0.0)
                        };
                        (
                            mat4_mul(&offset, &part.model),
                            part.colour,
                            part.alpha,
                            part.emissive,
                        )
                    }
                    DynKind::Core {
                        phase,
                        position,
                        base_scale,
                    } => {
                        let pulse = 1.0 + self.smoothed_bass * 0.08 + self.flash * 0.12;
                        let model = mat4_mul(
                            &translation(position[0], position[1], position[2]),
                            &mat4_mul(
                                &rotation_y(self.time * (0.05 + self.smoothed_high * 0.24) + phase),
                                &scale(
                                    base_scale[0] * pulse,
                                    base_scale[1] * pulse,
                                    base_scale[2] * pulse,
                                ),
                            ),
                        );
                        (model, part.colour, part.alpha, part.emissive)
                    }
                    DynKind::Plaque { phase, base_y } => {
                        let bob = (self.time * 0.25 + phase).sin() * 0.025;
                        let _ = base_y;
                        (
                            mat4_mul(&translation(0.0, bob, 0.0), &part.model),
                            part.colour,
                            _label_alpha(self.smoothed_mid),
                            part.emissive,
                        )
                    }
                };

                let instance = Instance {
                    model: mat4_mul(&room_model, &model),
                    colour: [colour[0], colour[1], colour[2], alpha],
                    params: [emissive, part.mode as f32, near, 0.0],
                };
                if part.mode >= MODE_PANEL {
                    additive.push(instance);
                } else {
                    opaque.push(instance);
                }
            }
        }

        // Chevrons (`_updateChevrons`, `:1762-1791`).
        let hue = self.plan(layout.current).palette();
        let ink = srgb_hex_to_linear(hue.ink);
        let ripple_gain =
            0.12 + self.smoothed_bass * 1.4 + self.smoothed_mid * 0.6 + self.flash * 0.9;
        for index in 0..6 {
            let z = chevron_grid_z(camera_z, index, 3.4);
            let marker_x = (z * 0.11).sin() * 0.35;
            let plate = Part {
                model: trs([marker_x, 0.012, z], [0.0, 0.0, 0.0], [1.175, 0.006, 1.9]),
                colour: [0.01, 0.013, 0.02],
                mode: MODE_PANEL,
                emissive: 1.0,
                alpha: 0.42,
                dynamic: DynKind::Static,
            };
            additive.push(Instance {
                model: plate.model,
                colour: [plate.colour[0], plate.colour[1], plate.colour[2], 0.42],
                params: [0.2, MODE_PANEL as f32, 0.0, 0.0],
            });

            let mut pixels = Vec::with_capacity(13);
            pixels.push((0.0, -0.62));
            for side in [-1.0_f32, 1.0] {
                for segment in 0..4 {
                    pixels.push((
                        side * (0.2 + segment as f32 * 0.18),
                        -0.38 + segment as f32 * 0.34,
                    ));
                }
            }
            for (local_x, local_z) in pixels {
                let pulse = hallway_ripple(self.ripple_phase, z + local_z);
                let alpha = clamp(
                    0.02 + pulse * ripple_gain * 0.18 + self.smoothed_bass * 0.012,
                    0.012,
                    0.24,
                );
                let scale_xy = 0.94 + pulse * 0.1 + self.flash * 0.04;
                additive.push(Instance {
                    model: trs(
                        [marker_x + local_x * scale_xy, 0.012, z + local_z],
                        [0.0, 0.0, 0.0],
                        [0.1 * scale_xy, 0.007, 0.1 * scale_xy],
                    ),
                    colour: [ink[0], ink[1], ink[2], alpha],
                    params: [1.0, MODE_DUST as f32, 0.0, 0.0],
                });
            }
        }

        // Dust: the live room's seeded points (`:948-978`), as additive boxes.
        let plan = self.plan(layout.current);
        let count = 90 + plan.complexity as usize * 22;
        for index in 0..count.min(220) {
            let angle = hash(plan.index as f32 * 101.0 + index as f32 * 7.1) * TAU;
            let radius =
                hash(plan.index as f32 * 211.0 + index as f32 * 3.7).sqrt() * plan.width * 0.44;
            let x = angle.cos() * radius;
            let y =
                0.4 + hash(plan.index as f32 * 307.0 + index as f32 * 9.2) * (plan.height - 0.7);
            let z = (hash(plan.index as f32 * 401.0 + index as f32 * 5.3) - 0.5) * plan.depth * 0.9;
            let rotation = self.time * (0.008 + self.smoothed_high * 0.025);
            let (sin, cos) = rotation.sin_cos();
            additive.push(Instance {
                model: trs(
                    [x * cos - z * sin, y, x * sin + z * cos + camera_z],
                    [0.0, 0.0, 0.0],
                    [0.035, 0.035, 0.035],
                ),
                colour: [
                    ink[0],
                    ink[1],
                    ink[2],
                    0.07 + self.smoothed_mid * 0.25 + self.flash * 0.18,
                ],
                params: [0.9, MODE_DUST as f32, 0.0, 0.0],
            });
        }

        (opaque, additive)
    }

    fn upload_uniforms(&self) {
        let Some(gpu) = self.gpu.as_ref() else {
            return;
        };
        let palette = self.plan(self.current_room).palette();
        let background = srgb_hex_to_linear(palette.background);
        let walk_colour = srgb_hex_to_linear(palette.light);
        let rim_colour = srgb_hex_to_linear(palette.accent);
        let drive = lighting_drive(
            self.smoothed_bass,
            self.smoothed_mid,
            self.smoothed_high,
            self.flash,
        );
        let camera = self.camera_position();
        let view_proj = self.view_proj();

        gpu.queue.write_buffer(
            &gpu.uniforms,
            0,
            bytemuck::bytes_of(&SceneUniforms {
                view_proj,
                camera_pos: [camera[0], camera[1], camera[2], 1.0],
                background: [
                    background[0],
                    background[1],
                    background[2],
                    0.009 + self.smoothed_bass * 0.001 + self.flash * 0.001,
                ],
                lights: [
                    0.35 + drive.wall * 0.25,
                    0.06 + drive.panel * 0.08,
                    0.25 + drive.wall * 0.35,
                    drive.line,
                ],
                walk: [self.walk_x, 3.3, START_Z - self.travel - 3.0, drive.walk],
                walk_colour: [walk_colour[0], walk_colour[1], walk_colour[2], 1.0],
                rim: [
                    -self.walk_x * 0.6,
                    2.6,
                    START_Z - self.travel - 11.0,
                    drive.rim,
                ],
                rim_colour: [rim_colour[0], rim_colour[1], rim_colour[2], 1.0],
                params: [self.time, self.flash, 0.0, 0.0],
            }),
        );
        gpu.queue.write_buffer(
            &gpu.grade_uniforms,
            0,
            bytemuck::bytes_of(&GradeUniforms {
                resolution: [self.size.0 as f32, self.size.1 as f32],
                time: self.time,
                intensity: 0.72,
                params: [self.brightness_scale, 0.0, 0.0, 0.0],
            }),
        );

        let (opaque, additive) = self.collect_instances();
        let opaque = &opaque[..opaque.len().min(gpu.instance_capacity)];
        let additive = &additive[..additive.len().min(gpu.additive_capacity)];
        gpu.queue
            .write_buffer(&gpu.instance_buffer, 0, bytemuck::cast_slice(opaque));
        gpu.queue
            .write_buffer(&gpu.additive_buffer, 0, bytemuck::cast_slice(additive));
    }

    fn rebuild_targets(&mut self) {
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };
        let (scene_texture, scene_view) = crate::point10::postfx::create_target(
            &gpu.device,
            "point10-latent-scene",
            self.size.0,
            self.size.1,
        );
        let (depth, depth_view) = create_depth(
            &gpu.device,
            "point10-latent-depth",
            self.size.0,
            self.size.1,
        );
        gpu.scene_texture = scene_texture;
        gpu.scene_view = scene_view;
        gpu._depth = depth;
        gpu.depth_view = depth_view;
        gpu.afterimage.resize(self.size.0, self.size.1);
        gpu.afterimage.reset();
        gpu.bloom.resize(self.size.0, self.size.1);
        gpu.composer.resize(self.size.0, self.size.1);
        gpu.grade_bind_group = create_grade_bind_group(&gpu.device, gpu);
        self.upload_uniforms();
    }
}

impl Point10Scene for LatentBackrooms {
    fn name(&self) -> &'static str {
        "Point10 Latent Backrooms"
    }

    fn init(&mut self, ctx: &mut SceneContext) {
        let device = ctx.device.clone();
        let queue = ctx.queue.clone();
        self.size = (ctx.width.max(1), ctx.height.max(1));
        self.camera_aspect = ctx.aspect();

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-latent-shader"),
            source: wgpu::ShaderSource::Wgsl(latent_wgsl().into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-latent-layout"),
            entries: &[uniform_entry(
                0,
                wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
            )],
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-latent-uniforms"),
            size: std::mem::size_of::<SceneUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point10-latent-bind-group"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("point10-latent-pipeline-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("point10-latent-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(Vertex::layout()), Some(Instance::layout())],
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
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let additive_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("point10-latent-additive-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(Vertex::layout()), Some(Instance::layout())],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: crate::point10::scenes::cube_mesh::DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
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

        let mesh = MeshGpu::new(
            &device,
            "point10-latent-box",
            &crate::point10::scenes::cube_mesh::build_box(2.0, 2.0, 2.0),
        );
        let instance_capacity = 2048;
        let additive_capacity = 1024;
        let instance_buffer = {
            use wgpu::util::DeviceExt;
            let empty = vec![
                Instance {
                    model: identity(),
                    colour: [0.0; 4],
                    params: [0.0; 4],
                };
                instance_capacity
            ];
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("point10-latent-instances"),
                contents: bytemuck::cast_slice(&empty),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            })
        };
        let additive_buffer = {
            use wgpu::util::DeviceExt;
            let empty = vec![
                Instance {
                    model: identity(),
                    colour: [0.0; 4],
                    params: [0.0; 4],
                };
                additive_capacity
            ];
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("point10-latent-additive"),
                contents: bytemuck::cast_slice(&empty),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            })
        };

        let (scene_texture, scene_view) = crate::point10::postfx::create_target(
            &device,
            "point10-latent-scene",
            self.size.0,
            self.size.1,
        );
        let (depth, depth_view) =
            create_depth(&device, "point10-latent-depth", self.size.0, self.size.1);

        let afterimage = AfterimagePass::new(&device, &queue, self.size.0, self.size.1, 0.78);
        let mut bloom = UnrealBloomPass::new(&device, &queue, self.size.0, self.size.1);
        bloom.set_strength(0.8);
        bloom.set_radius(0.9);
        bloom.set_threshold(0.12);
        bloom.set_input(afterimage.output_view());

        // --- CRT grade ------------------------------------------------------
        let grade_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point10-latent-grade-shader"),
            source: wgpu::ShaderSource::Wgsl(grade_wgsl().into()),
        });
        let grade_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("point10-latent-grade-layout"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::FRAGMENT),
                texture_entry(1),
                texture_entry(2),
                sampler_entry(3),
            ],
        });
        let grade_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point10-latent-grade-uniforms"),
            size: std::mem::size_of::<GradeUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let grade_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("point10-latent-grade-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let grade_pipeline = ctx.fullscreen_pipeline_with_format(
            "point10-latent-grade-pipeline",
            &grade_shader,
            &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("point10-latent-grade-pipeline-layout"),
                bind_group_layouts: &[Some(&grade_layout)],
                immediate_size: 0,
            }),
            HDR_FORMAT,
            None,
        );
        let grade_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point10-latent-grade-bind-group"),
            layout: &grade_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: grade_uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(afterimage.output_view()),
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
                    resource: wgpu::BindingResource::Sampler(&grade_sampler),
                },
            ],
        });

        // The JS composer ends with `OutputPass` and the default renderer
        // tone mapping (none).
        let mut composer =
            Composer::new(&device, &queue, self.size.0, self.size.1, ctx.target_format);
        composer.output_mut().set_tone_mapping(ToneMapping::None);

        self.gpu = Some(LatentGpu {
            device,
            queue,
            pipeline,
            additive_pipeline,
            uniforms,
            bind_group,
            mesh,
            instance_buffer,
            instance_capacity,
            additive_buffer,
            additive_capacity,
            scene_texture,
            scene_view,
            _depth: depth,
            depth_view,
            afterimage,
            bloom,
            grade_pipeline,
            grade_uniforms,
            grade_layout,
            grade_bind_group,
            grade_sampler,
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

        self.p10.update(dt, audio);
        let (bass, mid, high, beat) = if audio.is_available() {
            (
                audio.bands[0],
                audio.bands[1],
                audio.bands[2],
                audio.beat_env,
            )
        } else {
            (0.0, 0.0, 0.0, 0.0)
        };
        let bass = clamp(bass, 0.0, 1.0);
        let mid = clamp(mid, 0.0, 1.0);
        let high = clamp(high, 0.0, 1.0);
        let beat = clamp(beat, 0.0, 1.0);

        self.smoothed_bass += (bass - self.smoothed_bass)
            * (1.0 - (-dt * if bass > self.smoothed_bass { 10.0 } else { 3.5 }).exp());
        self.smoothed_mid += (mid - self.smoothed_mid)
            * (1.0 - (-dt * if mid > self.smoothed_mid { 9.0 } else { 3.5 }).exp());
        self.smoothed_high += (high - self.smoothed_high)
            * (1.0 - (-dt * if high > self.smoothed_high { 8.0 } else { 3.5 }).exp());
        self.flash = beat.max(self.flash * (-dt * 3.8).exp());
        self.morph = (beat * 0.8).max(self.morph * (-dt * 2.8).exp());

        let motion_beat = self.flash * 0.3;
        let turn_target =
            (self.time * 0.17).sin() * 0.18 + self.smoothed_mid * 0.72 + self.flash * 0.18;
        self.turn += (turn_target - self.turn) * (1.0 - (-dt * 2.8).exp());

        let drive = latent_backrooms_drive(
            self.travel,
            self.ripple_phase,
            dt,
            self.smoothed_bass,
            self.smoothed_mid,
            self.flash,
            self.speed_scale,
        );
        self.speed = drive.speed;
        self.travel = drive.travel;
        self.ripple_speed = ripple_speed(self.smoothed_bass, self.smoothed_mid, self.flash);
        self.ripple_phase = drive.ripple_phase;

        let layout = latent_room_layout(self.travel);
        self.current_room = layout.current;
        let phase = self.plan(self.current_room).phase;
        self.head_bob = run_bob(self.travel, phase);
        self.shoulder_sway = run_sway(self.travel, phase);

        let plan = *self.plan(self.current_room);
        let path = walk_vector(
            self.travel,
            &plan,
            self.smoothed_bass,
            self.smoothed_mid,
            self.smoothed_high,
            motion_beat,
            self.turn,
        );
        self.walk_vx += ((path.x - self.walk_x) * 8.0 - self.walk_vx * 4.8) * dt;
        self.walk_vy += ((path.y - self.walk_y) * 8.0 - self.walk_vy * 4.8) * dt;
        self.walk_x += self.walk_vx * dt;
        self.walk_y += self.walk_vy * dt;
        self.walk_x = clamp(self.walk_x, -plan.width * 0.24, plan.width * 0.24);
        self.walk_y = clamp(self.walk_y, 1.42, 2.18);

        let ahead = walk_vector(
            self.travel + 9.0,
            &plan,
            self.smoothed_bass,
            self.smoothed_mid,
            self.smoothed_high,
            motion_beat,
            self.turn,
        );
        self.look_vx +=
            ((ahead.x + self.turn * 0.08 - self.look_x) * 6.2 - self.look_vx * 4.4) * dt;
        self.look_vy += ((ahead.y + self.smoothed_high * 0.08 - 0.08 - self.look_y) * 6.2
            - self.look_vy * 4.4)
            * dt;
        self.look_x += self.look_vx * dt;
        self.look_y += self.look_vy * dt;

        let gaze_target_x = (self.time * 0.37).sin() * 0.65
            + (self.travel * 0.16 + phase).sin() * 0.35
            + self.smoothed_mid * 1.4;
        let gaze_target_y = (self.time * 0.29 + 1.2).sin() * 0.32
            + (self.travel * 0.23).sin() * 0.18
            + self.smoothed_high * 0.8;
        self.gaze_vx += ((gaze_target_x - self.gaze_x) * 3.2 - self.gaze_vx * 2.8) * dt;
        self.gaze_vy += ((gaze_target_y - self.gaze_y) * 3.2 - self.gaze_vy * 2.8) * dt;
        self.gaze_x += self.gaze_vx * dt;
        self.gaze_y += self.gaze_vy * dt;

        let roll_target = (self.time * 0.13).sin() * 0.012
            + self.smoothed_high * 0.014
            + self.shoulder_sway * 0.18;
        self.roll += (roll_target - self.roll) * (1.0 - (-dt * 4.0).exp());

        let damp = self.afterimage_damp();
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.afterimage.set_damp(damp);
        }
        self.upload_uniforms();
    }

    fn render(&mut self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        let palette = self.plan(self.current_room).palette();
        let background = srgb_hex_to_linear(palette.background);
        let bloom_strength = self.bloom_strength();
        let (opaque, additive) = self.collect_instances();
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };
        let opaque = &opaque[..opaque.len().min(gpu.instance_capacity)];
        let additive = &additive[..additive.len().min(gpu.additive_capacity)];

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("point10-latent-scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &gpu.scene_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // `scene.background = new THREE.Color(palette.background)`
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: background[0] as f64,
                            g: background[1] as f64,
                            b: background[2] as f64,
                            a: 1.0,
                        }),
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
            pass.set_bind_group(0, &gpu.bind_group, &[]);
            pass.set_vertex_buffer(1, gpu.instance_buffer.slice(..));
            if !opaque.is_empty() {
                gpu.mesh.draw_instanced(&mut pass, opaque.len() as u32);
            }
        }

        if !additive.is_empty() {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("point10-latent-additive"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &gpu.scene_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &gpu.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&gpu.additive_pipeline);
            pass.set_bind_group(0, &gpu.bind_group, &[]);
            pass.set_vertex_buffer(1, gpu.additive_buffer.slice(..));
            gpu.mesh.draw_instanced(&mut pass, additive.len() as u32);
        }

        // Afterimage -> bloom (the JS composer's pass order).
        gpu.afterimage.encode(encoder, &gpu.scene_view);
        gpu.bloom.set_input(gpu.afterimage.output_view());
        gpu.bloom.set_strength(bloom_strength);
        gpu.bloom.encode(encoder);

        // CRT grade + `afterimage + bloom` into the composer's scene target.
        gpu.grade_bind_group = create_grade_bind_group(&gpu.device, gpu);
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("point10-latent-grade"),
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
            pass.set_pipeline(&gpu.grade_pipeline);
            pass.set_bind_group(0, &gpu.grade_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        gpu.composer.render(encoder, target);
    }

    fn dispose(&mut self) {
        self.gpu = None;
    }

    fn props(&self) -> Vec<SceneProp> {
        scene_props_with_levers(LATENT_BACKROOMS_PROPS)
    }

    fn meta(&self) -> SceneMeta {
        SceneMeta::for_scene("Point10 Latent Backrooms", "3d-scenes")
    }
}

/// `_labelOpacity` (`:1866`).
fn _label_alpha(mid: f32) -> f32 {
    0.58 + mid * 0.16
}

// ---------------------------------------------------------------------------
// Room geometry
// ---------------------------------------------------------------------------

/// Colour helpers: a part's palette entries, in linear space.
struct Colours {
    wall: [f32; 3],
    ink: [f32; 3],
    light: [f32; 3],
    accent: [f32; 3],
    trim: [f32; 3],
}

impl Colours {
    fn new(palette: Palette) -> Self {
        Self {
            wall: srgb_hex_to_linear(palette.wall),
            ink: srgb_hex_to_linear(palette.ink),
            light: srgb_hex_to_linear(palette.light),
            accent: srgb_hex_to_linear(palette.accent),
            trim: srgb_hex_to_linear(palette.trim),
        }
    }
}

fn bx(x: f32, y: f32, z: f32, sx: f32, sy: f32, sz: f32) -> Mat4 {
    trs([x, y, z], [0.0, 0.0, 0.0], [sx, sy, sz])
}

/// Build one room's parts (`_makeRoom` + the per-kind builders).
fn build_room(plan: RoomPlan) -> Room {
    let colours = Colours::new(plan.palette());
    let mut parts: Vec<Part> = Vec::with_capacity(160);
    let width = plan.width;
    let height = plan.height;
    let depth = plan.depth;

    let mut push =
        |model: Mat4, colour: [f32; 3], mode: u32, emissive: f32, alpha: f32, dynamic: DynKind| {
            parts.push(Part {
                model,
                colour,
                mode,
                emissive,
                alpha,
                dynamic,
            });
        };

    // Shell (`:691-791`).
    push(
        bx(0.0, -0.02, 0.0, width * 0.5, 0.02, depth * 0.5),
        colours.wall,
        MODE_FLOOR,
        0.2,
        1.0,
        DynKind::Static,
    );
    push(
        bx(0.0, 0.012, 0.0, width * 0.49, 0.006, depth * 0.49),
        colours.wall,
        MODE_PANEL,
        1.0,
        0.05,
        DynKind::Static,
    );
    push(
        bx(0.0, height + 0.02, 0.0, width * 0.5, 0.02, depth * 0.5),
        colours.wall,
        MODE_WALL,
        0.22,
        1.0,
        DynKind::Static,
    );
    for side in [-1.0_f32, 1.0] {
        push(
            bx(
                side * width * 0.5,
                height * 0.5,
                0.0,
                0.1,
                height * 0.5,
                depth * 0.5,
            ),
            colours.wall,
            MODE_WALL,
            0.22,
            1.0,
            DynKind::Static,
        );
    }
    push(
        bx(0.0, 0.12, 0.0, width * 0.5, 0.12, depth * 0.5),
        colours.trim,
        MODE_TRIM,
        0.0,
        1.0,
        DynKind::Static,
    );
    for side in [-1.0_f32, 1.0] {
        push(
            bx(
                side * (width * 0.5 - 0.12),
                height * 0.5,
                0.0,
                0.12,
                height * 0.5,
                depth * 0.5,
            ),
            colours.trim,
            MODE_TRIM,
            0.0,
            1.0,
            DynKind::Static,
        );
    }

    // Seven ribs, panels, beams and ceiling fixtures (`:793-848`).
    for index in 0..7 {
        let z = -depth * 0.5 + ((index as f32 + 0.5) * depth) / 7.0;
        for side in [-1.0_f32, 1.0] {
            push(
                bx(
                    side * (width * 0.5 - 0.2),
                    height * 0.5,
                    z,
                    0.15,
                    height * 0.49,
                    0.11,
                ),
                colours.trim,
                MODE_TRIM,
                0.0,
                1.0,
                DynKind::Static,
            );
            push(
                bx(
                    side * (width * 0.5 - 0.36),
                    height * 0.52,
                    z,
                    0.0125,
                    height * 0.36,
                    depth * 0.0375,
                ),
                colours.light,
                MODE_PANEL,
                1.0,
                0.08,
                DynKind::Static,
            );
        }
        push(
            bx(0.0, height - 0.18, z, width * 0.47, 0.12, 0.11),
            colours.trim,
            MODE_TRIM,
            0.0,
            1.0,
            DynKind::Static,
        );
        push(
            bx(0.0, height - 0.32, z, width * 0.165, 0.0175, 0.035),
            colours.light,
            MODE_LIGHT,
            1.0,
            0.28,
            DynKind::Fixture { local_z: z },
        );
    }

    // End portals (`:850-935`).
    let side = width * 0.32;
    for end in [-1.0_f32, 1.0] {
        let portal_z = end * (depth * 0.5 - 0.28);
        for jamb in [-1.0_f32, 1.0] {
            push(
                bx(
                    jamb * side,
                    height * 0.4,
                    portal_z,
                    0.18,
                    height * 0.4,
                    0.21,
                ),
                colours.trim,
                MODE_TRIM,
                0.0,
                1.0,
                DynKind::Static,
            );
        }
        push(
            bx(0.0, height * 0.78, portal_z, side, 0.18, 0.21),
            colours.trim,
            MODE_TRIM,
            0.0,
            1.0,
            DynKind::Static,
        );
        let wing = (width * 0.5 - side).max(0.0);
        for wing_side in [-1.0_f32, 1.0] {
            push(
                bx(
                    wing_side * (side + wing * 0.5),
                    height * 0.4,
                    portal_z,
                    wing * 0.5,
                    height * 0.4,
                    0.09,
                ),
                colours.wall,
                MODE_WALL,
                0.22,
                1.0,
                DynKind::Static,
            );
        }
        push(
            bx(0.0, height * 0.89, portal_z, side, height * 0.11, 0.09),
            colours.wall,
            MODE_WALL,
            0.22,
            1.0,
            DynKind::Static,
        );
        push(
            bx(0.0, 0.08, portal_z, side, 0.08, 0.12),
            colours.trim,
            MODE_TRIM,
            0.0,
            1.0,
            DynKind::Static,
        );
        push(
            bx(
                0.0,
                height * 0.39,
                portal_z + end * 0.03,
                side * 0.85,
                height * 0.34,
                0.0125,
            ),
            colours.light,
            MODE_PANEL,
            1.0,
            0.08,
            DynKind::Static,
        );
    }

    // The room-code plaque (`_makePlaque`): a small emissive plate in place of
    // the JS's canvas-text sprite.
    let plaque_width = (width * 0.26).min(3.4);
    push(
        bx(
            -width * 0.34,
            2.25,
            -depth * 0.2,
            plaque_width * 0.5,
            plaque_width / 12.0,
            0.02,
        ),
        colours.light,
        MODE_DUST,
        1.0,
        0.58,
        DynKind::Plaque {
            phase: plan.phase,
            base_y: 2.25,
        },
    );

    // Per-kind detail (reduced counts, see the module docs).
    match plan.kind {
        "backrooms" => build_backrooms(&plan, &colours, &mut push),
        "library" => build_library(&plan, &colours, &mut push),
        "hall" => build_hall(&plan, &colours, &mut push),
        "server" => build_server(&plan, &colours, &mut push),
        "maze" => build_maze(&plan, &colours, &mut push),
        "archive" => build_archive(&plan, &colours, &mut push),
        "cathedral" => build_cathedral(&plan, &colours, &mut push),
        "garden" => build_garden(&plan, &colours, &mut push),
        "void" => build_void(&plan, &colours, &mut push),
        _ => build_chapel(&plan, &colours, &mut push),
    }
    drop(push);

    Room { plan, parts }
}

type PushPart<'a> = dyn FnMut(Mat4, [f32; 3], u32, f32, f32, DynKind) + 'a;

/// `_addBackrooms` (`:1026-1067`) — four dark alcoves per side.
fn build_backrooms(plan: &RoomPlan, colours: &Colours, push: &mut PushPart<'_>) {
    for index in 0..4 {
        let z = -plan.depth * 0.42 + index as f32 * plan.depth * 0.24;
        for side in [-1.0_f32, 1.0] {
            let x = side * plan.width * 0.42;
            push(
                bx(x, 1.2, z, 0.08, 1.2, 0.17),
                colours.trim,
                MODE_TRIM,
                0.0,
                1.0,
                DynKind::Static,
            );
            push(
                bx(x, 2.45, z, 0.75, 0.08, 0.17),
                colours.trim,
                MODE_TRIM,
                0.0,
                1.0,
                DynKind::Static,
            );
            push(
                bx(x - side * 0.1, 1.2, z, 0.015, 1.025, 0.09),
                colours.light,
                MODE_PANEL,
                1.0,
                0.08,
                DynKind::Static,
            );
        }
    }
}

/// `_addLibrary` (`:1069-1136`) — angled shelves with pulsing spines.
fn build_library(plan: &RoomPlan, colours: &Colours, push: &mut PushPart<'_>) {
    push(
        bx(0.0, 0.04, 0.0, plan.width * 0.17, 0.02, plan.depth * 0.45),
        colours.accent,
        MODE_ACCENT,
        0.12,
        1.0,
        DynKind::Static,
    );
    for side in [-1.0_f32, 1.0] {
        for row in 0..3 {
            let y = 0.8 + (row as f32 * (plan.height - 1.6)) / 3.0;
            let x = side * plan.width * 0.33;
            push(
                bx(x, y, 0.0, plan.width * 0.14, 0.06, plan.depth * 0.36),
                colours.trim,
                MODE_TRIM,
                0.0,
                1.0,
                DynKind::Static,
            );
            for col in 0..6 {
                let z = -plan.depth * 0.32 + col as f32 * plan.depth * 0.106;
                let h =
                    0.42 + hash(plan.index as f32 * 37.0 + row as f32 * 13.0 + col as f32) * 0.62;
                let colour = [colours.ink, colours.light, colours.accent][(row + col) % 3];
                let base_y = y + h * 0.5;
                push(
                    bx(x - side * 0.14, base_y, z, 0.06, h * 0.5, 0.17),
                    colour,
                    MODE_BOOK,
                    0.16 + (row % 3) as f32 * 0.06,
                    1.0,
                    DynKind::Bar {
                        phase: hash(plan.index as f32 * 61.0 + row as f32 * 7.0 + col as f32) * TAU,
                        pos: col as f32 / 5.0,
                        base_y,
                    },
                );
            }
        }
    }
    for index in 0..6 {
        let z = -plan.depth * 0.36 + index as f32 * plan.depth * 0.145;
        push(
            bx(0.0, plan.height * 0.82, z, 0.12, 0.12, 0.12),
            colours.light,
            MODE_LIGHT,
            1.0,
            0.28,
            DynKind::Core {
                phase: index as f32,
                position: [0.0, plan.height * 0.82, z],
                base_scale: [0.12, 0.12, 0.12],
            },
        );
    }
}

/// `_addHall` (`:1138-1227`) — columns, beams and hanging lights.
fn build_hall(plan: &RoomPlan, colours: &Colours, push: &mut PushPart<'_>) {
    push(
        bx(0.0, 0.05, 0.0, plan.width * 0.14, 0.025, plan.depth * 0.47),
        colours.accent,
        MODE_ACCENT,
        0.12,
        1.0,
        DynKind::Static,
    );
    for index in 0..4 {
        let z = -plan.depth * 0.39 + index as f32 * plan.depth * 0.234;
        for side in [-1.0_f32, 1.0] {
            let x = side * plan.width * 0.31;
            push(
                bx(x, plan.height * 0.43, z, 0.26, plan.height * 0.43, 0.26),
                colours.trim,
                MODE_TRIM,
                0.0,
                1.0,
                DynKind::Mover {
                    phase: index as f32 * 1.1,
                    amplitude: 0.025,
                    axis: 1,
                },
            );
            push(
                bx(x, 0.22, z, 0.55, 0.175, 0.55),
                colours.trim,
                MODE_TRIM,
                0.0,
                1.0,
                DynKind::Static,
            );
            push(
                bx(x, plan.height * 0.78, z, 0.09, plan.height * 0.18, 0.09),
                colours.accent,
                MODE_ACCENT,
                0.25,
                1.0,
                DynKind::Static,
            );
        }
        push(
            bx(0.0, plan.height * 0.8, z, plan.width * 0.36, 0.15, 0.17),
            colours.trim,
            MODE_TRIM,
            0.0,
            1.0,
            DynKind::Mover {
                phase: index as f32 * 0.8,
                amplitude: 0.03,
                axis: 1,
            },
        );
        push(
            bx(0.0, plan.height * 0.7, z, plan.width * 0.06, 0.025, 0.06),
            colours.light,
            MODE_LIGHT,
            1.0,
            0.28,
            DynKind::Static,
        );
    }
}

/// `_addServer` (`:1229-1282`) — rack rows with strip lights.
fn build_server(plan: &RoomPlan, colours: &Colours, push: &mut PushPart<'_>) {
    for side in [-1.0_f32, 1.0] {
        for row in 0..3 {
            let y = 0.75 + (row as f32 * (plan.height - 1.5)) / 3.0;
            let x = side * plan.width * 0.34;
            push(
                bx(
                    x,
                    y,
                    0.0,
                    plan.width * 0.11,
                    plan.height * 0.1,
                    plan.depth * 0.34,
                ),
                colours.trim,
                MODE_TRIM,
                0.0,
                1.0,
                DynKind::Static,
            );
            for col in 0..4 {
                let z = -plan.depth * 0.29 + col as f32 * plan.depth * 0.164;
                push(
                    bx(x - side * 0.13, y, z, 0.02, plan.height * 0.065, 0.04),
                    colours.accent,
                    MODE_ACCENT,
                    0.4,
                    1.0,
                    DynKind::Bar {
                        phase: col as f32 * 0.8,
                        pos: col as f32 / 3.0,
                        base_y: y,
                    },
                );
            }
        }
    }
    for index in 0..3 {
        let z = -plan.depth * 0.38 + index as f32 * plan.depth * 0.32;
        push(
            bx(0.0, plan.height * 0.76, z, plan.width * 0.12, 0.0175, 0.04),
            colours.accent,
            MODE_ACCENT,
            0.35,
            1.0,
            DynKind::Static,
        );
    }
}

/// `_addMaze` (`:1284-1321`).
fn build_maze(plan: &RoomPlan, colours: &Colours, push: &mut PushPart<'_>) {
    for index in 0..8 {
        let side = if index % 2 == 1 { 1.0 } else { -1.0 };
        let x = side * plan.width * (0.12 + hash(plan.index as f32 * 43.0 + index as f32) * 0.23);
        let z = -plan.depth * 0.4 + index as f32 * plan.depth * 0.11;
        let h = plan.height * (0.32 + hash(plan.index as f32 * 59.0 + index as f32) * 0.48);
        push(
            bx(x, h * 0.5, z, 0.12, h * 0.5, plan.depth * 0.1),
            colours.trim,
            MODE_TRIM,
            0.0,
            1.0,
            DynKind::Mover {
                phase: index as f32 * 1.3,
                amplitude: 0.12,
                axis: 0,
            },
        );
        push(
            bx(
                x - side * 0.14,
                h * 0.54,
                z,
                0.0125,
                h * 0.38,
                plan.depth * 0.07,
            ),
            colours.light,
            MODE_PANEL,
            1.0,
            0.08,
            DynKind::Static,
        );
    }
}

/// `_addArchive` (`:1323-1362`).
fn build_archive(plan: &RoomPlan, colours: &Colours, push: &mut PushPart<'_>) {
    for side in [-1.0_f32, 1.0] {
        for row in 0..4 {
            let y = 0.7 + (row as f32 * (plan.height - 1.4)) / 4.0;
            let x = side * plan.width * 0.36;
            push(
                bx(x, y, 0.0, plan.width * 0.095, 0.05, plan.depth * 0.42),
                colours.trim,
                MODE_TRIM,
                0.0,
                1.0,
                DynKind::Static,
            );
            for col in 0..4 {
                let z = -plan.depth * 0.37 + col as f32 * plan.depth * 0.246;
                let colour = [colours.ink, colours.light, colours.accent][col % 3];
                push(
                    bx(x - side * 0.1, y + 0.13, z, 0.0175, 0.12, 0.16),
                    colour,
                    MODE_BOOK,
                    0.2,
                    1.0,
                    DynKind::Bar {
                        phase: col as f32 * 0.7,
                        pos: col as f32 / 3.0,
                        base_y: y + 0.13,
                    },
                );
            }
        }
    }
}

/// `_addCathedral` (`:1364-1418`).
fn build_cathedral(plan: &RoomPlan, colours: &Colours, push: &mut PushPart<'_>) {
    for index in 0..3 {
        let z = -plan.depth * 0.35 + index as f32 * plan.depth * 0.34;
        let width = plan.width * (0.25 + index as f32 * 0.05);
        for side in [-1.0_f32, 1.0] {
            push(
                bx(
                    side * width,
                    plan.height * 0.29,
                    z,
                    0.17,
                    plan.height * 0.29,
                    0.19,
                ),
                colours.trim,
                MODE_TRIM,
                0.0,
                1.0,
                DynKind::Static,
            );
        }
        push(
            bx(0.0, plan.height * 0.58, z, width * 0.775, 0.025, 0.04),
            colours.light,
            MODE_LIGHT,
            1.0,
            0.28,
            DynKind::Static,
        );
    }
    push(
        bx(0.0, 1.35, -plan.depth * 0.08, 0.45, 0.6, 0.45),
        colours.accent,
        MODE_ACCENT,
        0.4,
        1.0,
        DynKind::Core {
            phase: 0.0,
            position: [0.0, 1.35, -plan.depth * 0.08],
            base_scale: [0.45, 0.6, 0.45],
        },
    );
}

/// `_addGarden` (`:1420-1463`).
fn build_garden(plan: &RoomPlan, colours: &Colours, push: &mut PushPart<'_>) {
    for index in 0..10 {
        let angle = index as f32 * 2.399 + plan.phase;
        let x = angle.cos() * plan.width * (0.18 + (index % 3) as f32 * 0.1);
        let z = angle.sin() * plan.depth * 0.3;
        push(
            bx(x, plan.height * 0.3, z, 0.03, plan.height * 0.25, 0.03),
            colours.trim,
            MODE_TRIM,
            0.0,
            1.0,
            DynKind::Mover {
                phase: angle,
                amplitude: 0.09,
                axis: 0,
            },
        );
        push(
            bx(
                x,
                plan.height * 0.6 + (index % 3) as f32 * 0.5,
                z,
                0.09,
                0.15,
                0.09,
            ),
            if index % 2 == 1 {
                colours.accent
            } else {
                colours.light
            },
            MODE_LIGHT,
            1.0,
            0.35,
            DynKind::Mover {
                phase: angle + 1.0,
                amplitude: 0.16,
                axis: 1,
            },
        );
    }
}

/// `_addVoid` (`:1465-1527`).
fn build_void(plan: &RoomPlan, colours: &Colours, push: &mut PushPart<'_>) {
    for index in 0..24 {
        let x = (hash(plan.index as f32 * 71.0 + index as f32 * 3.0) - 0.5) * plan.width * 1.8;
        let y = 0.5 + hash(plan.index as f32 * 73.0 + index as f32 * 5.0) * plan.height * 1.4;
        let z = (hash(plan.index as f32 * 79.0 + index as f32 * 7.0) - 0.5) * plan.depth * 1.8;
        push(
            bx(x, y, z, 0.055, 0.055, 0.055),
            colours.light,
            MODE_DUST,
            1.0,
            0.7,
            DynKind::Static,
        );
    }
    push(
        bx(
            0.0,
            plan.height * 0.48,
            -plan.depth * 0.12,
            0.65,
            0.65,
            0.65,
        ),
        colours.accent,
        MODE_ACCENT,
        0.45,
        1.0,
        DynKind::Core {
            phase: 0.0,
            position: [0.0, plan.height * 0.48, -plan.depth * 0.12],
            base_scale: [0.65, 0.65, 0.65],
        },
    );
    push(
        bx(
            0.0,
            plan.height * 0.48,
            -plan.depth * 0.12,
            plan.width * 0.15,
            0.02,
            0.02,
        ),
        colours.accent,
        MODE_ACCENT,
        0.3,
        1.0,
        DynKind::Static,
    );
    push(
        bx(
            0.0,
            plan.height * 0.48,
            -plan.depth * 0.12,
            0.02,
            plan.height * 0.15,
            0.02,
        ),
        colours.accent,
        MODE_ACCENT,
        0.3,
        1.0,
        DynKind::Static,
    );
}

/// `_addChapel` (`:1529-1597`).
fn build_chapel(plan: &RoomPlan, colours: &Colours, push: &mut PushPart<'_>) {
    push(
        bx(0.0, 0.05, 0.0, plan.width * 0.13, 0.025, plan.depth * 0.46),
        colours.accent,
        MODE_ACCENT,
        0.12,
        1.0,
        DynKind::Static,
    );
    for index in 0..4 {
        let z = -plan.depth * 0.35 + index as f32 * plan.depth * 0.225;
        push(
            bx(0.0, 0.65, z, plan.width * 0.3, 0.08, 0.18),
            colours.trim,
            MODE_TRIM,
            0.0,
            1.0,
            DynKind::Mover {
                phase: index as f32,
                amplitude: 0.025,
                axis: 1,
            },
        );
        for side in [-1.0_f32, 1.0] {
            push(
                bx(side * plan.width * 0.27, 1.8, z, 0.04, 1.1, 0.04),
                colours.accent,
                MODE_ACCENT,
                0.3,
                1.0,
                DynKind::Static,
            );
        }
    }
    push(
        bx(
            0.0,
            plan.height * 0.55,
            -plan.depth * 0.38,
            plan.width * 0.14,
            plan.height * 0.13,
            0.02,
        ),
        colours.light,
        MODE_LIGHT,
        1.0,
        0.28,
        DynKind::Static,
    );
}

// ---------------------------------------------------------------------------
// Shaders
// ---------------------------------------------------------------------------

fn latent_wgsl() -> String {
    LATENT_WGSL.to_string()
}

fn grade_wgsl() -> String {
    format!("{FULLSCREEN_TRIANGLE_WGSL}\n{GRADE_WGSL}")
}

fn create_grade_bind_group(device: &wgpu::Device, gpu: &LatentGpu) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("point10-latent-grade-bind-group"),
        layout: &gpu.grade_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: gpu.grade_uniforms.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(gpu.afterimage.output_view()),
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
                resource: wgpu::BindingResource::Sampler(&gpu.grade_sampler),
            },
        ],
    })
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

const LATENT_WGSL: &str = r#"
struct SceneUniforms {
  view_proj: mat4x4<f32>,
  camera_pos: vec4<f32>,
  // rgb = palette background, w = fog density
  background: vec4<f32>,
  // x = hemi, y = ambient, z = fill, w = line
  lights: vec4<f32>,
  // xyz = walk light position, w = intensity
  walk: vec4<f32>,
  walk_colour: vec4<f32>,
  // xyz = rim light position, w = intensity
  rim: vec4<f32>,
  rim_colour: vec4<f32>,
  // x = time, y = flash
  params: vec4<f32>,
}
@group(0) @binding(0) var<uniform> u: SceneUniforms;

fn hash21(p: vec2<f32>) -> f32 {
  return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

fn point_light(position: vec3<f32>, colour: vec3<f32>, intensity: f32, normal: vec3<f32>, world: vec3<f32>) -> vec3<f32> {
  let to_light = position - world;
  let distance = max(length(to_light), 0.001);
  let direction = to_light / distance;
  return colour * intensity * max(dot(normal, direction), 0.0) / (1.0 + distance * distance * 0.06);
}

struct VsOut {
  @builtin(position) position: vec4<f32>,
  @location(0) normal: vec3<f32>,
  @location(1) world: vec3<f32>,
  @location(2) colour: vec3<f32>,
  @location(3) alpha: f32,
  @location(4) emissive: f32,
  @location(5) mode: f32,
  // x = near, y = unused
  @location(6) extra: f32,
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
  @location(9) instance_params: vec4<f32>,
) -> VsOut {
  let model = mat4x4<f32>(model_0, model_1, model_2, model_3);
  let world = model * vec4<f32>(position, 1.0);
  var out: VsOut;
  out.position = u.view_proj * world;
  out.normal = normalize((model * vec4<f32>(normal, 0.0)).xyz);
  out.world = world.xyz;
  out.colour = instance_colour.rgb;
  out.alpha = instance_colour.a;
  out.emissive = instance_params.x;
  out.mode = instance_params.y;
  out.extra = instance_params.z;
  return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
  let mode = u32(in.mode + 0.5);

  // Additive materials (`MODE_PANEL`, `MODE_LIGHT`, `MODE_DUST`) are unlit.
  if (mode >= 3u) {
    return vec4<f32>(in.colour * in.emissive, in.alpha);
  }

  var albedo = in.colour;
  // The glyph wall/floor pattern stands in for the JS canvas textures: a
  // hashed glyph grid plus faint run-length scratches.
  if (mode == 1u) {
    let cell = floor(vec2<f32>(in.world.y * 2.2, in.world.z * 1.1));
    let glyph = step(0.55, hash21(cell));
    let streak = hash21(floor(vec2<f32>(in.world.y * 14.0, in.world.z * 0.6)));
    albedo = albedo * (0.72 + 0.28 * streak) + in.colour * glyph * 0.35;
  } else if (mode == 2u) {
    let cell = floor(vec2<f32>(in.world.x * 1.6, in.world.z * 1.6));
    let glyph = step(0.62, hash21(cell));
    let grid_x = abs(fract(in.world.x / 2.8) - 0.5);
    let grid_z = abs(fract(in.world.z / 2.8) - 0.5);
    let line = 1.0 - smoothstep(0.0, 0.04, min(grid_x, grid_z));
    albedo = albedo * (0.85 + 0.15 * glyph) + in.colour * line * u.lights.w;
  } else if (mode == 5u) {
    albedo = albedo * (0.8 + 0.2 * hash21(floor(in.world.xz * 3.0)));
  }

  let normal = normalize(in.normal);
  let hemi = mix(vec3<f32>(0.02, 0.018, 0.015), u.walk_colour.rgb, normal.y * 0.5 + 0.5)
    * u.lights.x;
  let fill = max(dot(normal, normalize(vec3<f32>(2.0, 8.0, 4.0))), 0.0) * u.lights.z;
  var lit = albedo * (hemi + vec3<f32>(1.0) * fill);
  lit = lit + albedo * point_light(u.walk.xyz, u.walk_colour.rgb, u.walk.w, normal, in.world);
  lit = lit + albedo * point_light(u.rim.xyz, u.rim_colour.rgb, u.rim.w, normal, in.world);
  lit = lit + albedo * u.lights.y;

  // `MeshStandardMaterial.emissive * emissiveIntensity`.
  lit = lit + in.colour * in.emissive;

  // FogExp2: `1 - exp(-density² * distance²)`.
  let distance = length(u.camera_pos.xyz - in.world);
  let fog = 1.0 - exp(-u.background.w * u.background.w * distance * distance);
  let colour = mix(lit, u.background.rgb, clamp(fog, 0.0, 1.0));
  return vec4<f32>(colour, in.alpha);
}
"#;

const GRADE_WGSL: &str = r#"
struct GradeUniforms {
  resolution: vec2<f32>,
  time: f32,
  intensity: f32,
  // x = exposure (the `brightness` lever)
  params: vec4<f32>,
}
@group(0) @binding(0) var<uniform> u: GradeUniforms;
@group(0) @binding(1) var base_texture: texture_2d<f32>;
@group(0) @binding(2) var bloom_texture: texture_2d<f32>;
@group(0) @binding(3) var source_sampler: sampler;

fn hash12(p: vec2<f32>) -> f32 {
  return fract(sin(dot(p, vec2<f32>(12.9898, 78.233))) * 43758.5453);
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
  let uv = position.xy / u.resolution;
  let base = textureSample(base_texture, source_sampler, uv).rgb;
  // The JS composes `afterimage + bloom` before the CRT pass.
  var colour = base + textureSample(bloom_texture, source_sampler, uv).rgb;

  // CRT: scanline, vignette and grain (`:1631-1640`).
  let scan = 0.91 + 0.09 * sin(uv.y * u.resolution.y * 3.14159265);
  let centred = (uv - vec2<f32>(0.5)) * vec2<f32>(u.resolution.x / max(u.resolution.y, 1.0), 1.0);
  let vignette = 1.0 - smoothstep(0.32, 0.92, length(centred));
  let grain = hash12(uv * u.resolution + vec2<f32>(u.time));
  colour = colour * scan * mix(0.78, 1.0, vignette);
  colour = colour + vec3<f32>((grain - 0.5) * 0.018 * u.intensity);
  // `gl_FragColor = vec4(max(color * exposure, 0.0), 1.0)` — the module is
  // opaque.
  return vec4<f32>(max(colour * u.params.x, vec3<f32>(0.0)), 1.0);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32, tolerance: f32) {
        assert!((a - b).abs() < tolerance, "{a} != {b}");
    }

    #[test]
    fn room_plan_builds_the_twelve_rooms() {
        let plans: Vec<RoomPlan> = (0..ROOM_COUNT).map(create_room_plan).collect();
        assert_eq!(plans.len(), 12);
        assert_eq!(plans[0].name, ROOM_NAMES[0]);
        assert_eq!(plans[0].kind, "backrooms");
        assert_eq!(plans[3].kind, "server");
        assert_eq!(plans[9].kind, "void");
        assert_eq!(plans[11].kind, ROOM_KINDS[11]);

        for plan in &plans {
            assert!(plan.depth >= 40.0 && plan.depth < LATENT_ROOM_SPACING);
            assert!(plan.width > 14.0);
            assert!(plan.height > 7.0);
            assert!((2..7).contains(&plan.complexity));
            assert!(plan.palette < PALETTES.len());
            assert!(plan.phase.is_finite());
        }
        // clamps negative indexes and wraps the tables
        assert_eq!(create_room_plan(0).index, 0);
        assert_eq!(create_room_plan(12).name, ROOM_NAMES[0]);
        assert_eq!(create_room_plan(13).kind, ROOM_KINDS[1]);
    }

    #[test]
    fn room_words_and_palettes_match_the_js() {
        assert!(ROOM_WORDS.contains(&"LATENT"));
        assert!(ROOM_WORDS.contains(&"MEMORY"));
        assert_eq!(PALETTES[0].background, 0x0d0b07);
        assert_eq!(PALETTES[1].light, 0xa9eaff);
        assert_eq!(PALETTES[3].accent, 0xffb45e);
        for palette in PALETTES {
            assert_eq!(srgb_hex_to_linear(palette.ink).len(), 3);
        }
    }

    #[test]
    fn the_corridor_treadmill_cycles_every_spacing() {
        assert_eq!(room_cycle(0.0), 0);
        assert_eq!(room_cycle(27.0), 0);
        assert_eq!(room_cycle(28.0), 1);
        assert_eq!(room_cycle(84.0), 2);

        let first = latent_room_layout(0.0);
        assert_eq!(first.cycle, 0);
        assert_eq!(first.current, 0);
        assert_eq!(first.camera_z, START_Z);
        assert_eq!(first.rooms.len(), ROOM_COUNT);
        assert_eq!(first.rooms[0].index, 0);
        assert_eq!(first.rooms[0].z, START_Z);
        assert_eq!(first.rooms[1].z, START_Z - LATENT_ROOM_SPACING);

        let later = latent_room_layout(LATENT_ROOM_SPACING);
        assert_eq!(later.cycle, 1);
        assert_eq!(later.current, 1);
        assert_eq!(later.rooms[0].absolute, 1);
        assert_eq!(later.rooms[0].index, 1);
        assert_eq!(later.rooms[0].z, START_Z - LATENT_ROOM_SPACING);
    }

    #[test]
    fn the_drive_creeps_forward_and_clamps_long_frames() {
        let drive = latent_backrooms_drive(0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0);
        close(drive.speed, 6.2, 1e-6);
        assert_eq!(drive.travel, 0.0);
        assert_eq!(drive.ripple_phase, 0.0);
        assert_eq!(drive.camera_z, START_Z);

        let long = latent_backrooms_drive(0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0);
        close(long.step, MAX_DRIVE_DT, 1e-6);
        close(long.travel, 0.62, 1e-6);
        close(long.camera_z, START_Z - 0.62, 1e-6);

        let scaled = latent_backrooms_drive(0.0, 0.0, 0.1, 0.0, 0.0, 0.0, 2.0);
        close(scaled.speed, 12.4, 1e-6);
        close(scaled.travel, 1.24, 1e-6);

        let loud = latent_backrooms_drive(0.0, 0.0, 0.1, 0.9, 0.8, 0.9, 1.0);
        close(loud.speed, walk_speed(0.9, 0.8, 0.9), 1e-6);
        close(loud.travel, 0.1 * loud.speed, 1e-5);
        close(
            loud.ripple_phase,
            0.1 * ripple_speed(0.9, 0.8, 0.9) * 0.32,
            1e-5,
        );

        let wrapped = latent_backrooms_drive(0.0, TAU - 0.01, 0.1, 1.0, 0.0, 0.0, 1.0);
        assert!(wrapped.ripple_phase >= 0.0 && wrapped.ripple_phase < TAU);

        let bad = latent_backrooms_drive(
            f32::NAN,
            f32::NAN,
            f32::NAN,
            f32::NAN,
            f32::NAN,
            f32::NAN,
            f32::NAN,
        );
        assert_eq!(bad.step, 0.0);
        close(bad.speed, 6.2, 1e-6);
        assert_eq!(bad.travel, 0.0);
        assert_eq!(bad.ripple_phase, 0.0);
    }

    #[test]
    fn the_audio_mapping_scales_the_lights() {
        assert!(walk_speed(0.9, 0.8, 0.9) > walk_speed(0.0, 0.0, 0.0) + 8.0);
        assert_eq!(walk_speed(2.0, 2.0, 2.0), walk_speed(1.0, 1.0, 1.0));
        assert_eq!(walk_speed(-1.0, -1.0, -1.0), walk_speed(0.0, 0.0, 0.0));

        let quiet = lighting_drive(0.0, 0.0, 0.0, 0.0);
        let loud = lighting_drive(0.9, 0.8, 0.7, 1.0);
        assert!(quiet.wall < 0.3);
        assert!(loud.walk > quiet.walk * 3.0);
        assert!(loud.bloom > quiet.bloom * 2.0);
        assert!(ripple_speed(0.9, 0.8, 1.0) > walk_speed(0.9, 0.8, 1.0) * 2.0);
    }

    #[test]
    fn the_head_motion_and_gaze_are_finite() {
        assert!(run_bob(0.0, 0.0).is_finite());
        assert!(run_sway(0.0, 0.0).is_finite());
        assert_ne!(run_bob(0.0, 0.0), run_bob(1.0, 0.0));
        close(hallway_ripple(std::f32::consts::FRAC_PI_2, 0.0), 1.0, 1e-5);
        assert_eq!(hallway_ripple(0.0, 0.0), 0.0);
        assert_eq!(hallway_ripple(-std::f32::consts::FRAC_PI_2, 0.0), 0.0);
        assert_ne!(chevron_grid_z(0.0, 0, 3.4), chevron_grid_z(-10.0, 0, 3.4));
    }

    #[test]
    fn the_camera_path_stays_inside_the_room() {
        let plan = create_room_plan(0);
        let quiet = walk_vector(0.0, &plan, 0.0, 0.0, 0.0, 0.0, 0.0);
        let musical = walk_vector(0.0, &plan, 0.9, 0.8, 0.7, 0.9, 1.4);
        assert!(quiet.x.is_finite() && quiet.y.is_finite());
        assert!((musical.x - quiet.x).abs() > 0.001);
        assert!(musical.x.abs() <= plan.width * 0.22 + 1e-5);
        assert!(musical.y > 1.6);
    }

    #[test]
    fn preset_constants_match_the_js() {
        assert_eq!(ROOM_COUNT, 12);
        assert_eq!(LATENT_ROOM_SPACING, 56.0);
        assert_eq!(START_Z, 10.0);
        assert_eq!(MAX_DRIVE_DT, 0.1);
        assert_eq!(ROOM_NAMES.len(), ROOM_COUNT);
        assert_eq!(ROOM_KINDS.len(), ROOM_COUNT);
        assert_eq!(ROOM_CODES.len(), ROOM_COUNT);
        close(TAU, std::f32::consts::TAU, 1e-6);
    }

    #[test]
    fn uniforms_match_the_documented_sizes() {
        assert_eq!(std::mem::size_of::<SceneUniforms>(), 192);
        assert_eq!(std::mem::size_of::<GradeUniforms>(), 32);
        assert_eq!(std::mem::size_of::<Instance>(), 96);
    }

    #[test]
    fn every_room_has_a_shell_and_details() {
        let scene = LatentBackrooms::new();
        assert_eq!(scene.room_count(), ROOM_COUNT);
        for room in &scene.rooms {
            assert!(room.parts.len() > 30, "{} parts", room.parts.len());
            for part in &room.parts {
                assert!(part.model[3][3].is_finite());
                assert!(part.colour.iter().all(|value| value.is_finite()));
            }
        }
        // Every room kind is represented by a distinct build path.
        let kinds: Vec<&str> = scene.rooms.iter().map(|room| room.plan.kind).collect();
        assert!(kinds.contains(&"library"));
        assert!(kinds.contains(&"void"));
        assert!(kinds.contains(&"garden"));
    }

    #[test]
    fn levers_default_to_one_and_the_damp_clamps() {
        let scene = LatentBackrooms::new();
        assert_eq!(scene.speed_scale, 1.0);
        assert_eq!(scene.bloom_scale, 1.0);
        assert_eq!(scene.trail_scale, 1.0);
        assert_eq!(scene.brightness_scale, 1.0);
        // 0.48 at silence.
        close(scene.afterimage_damp(), 0.48, 1e-6);
        // The bloom floor is 0.35.
        close(scene.bloom_strength(), 0.45, 1e-6);
        assert!(scene.camera_fov() >= 68.0);
    }

    #[test]
    fn props_and_meta_match_the_js_registry() {
        let scene = LatentBackrooms::new();
        assert_eq!(scene.meta().name, "Point10 Latent Backrooms");
        assert_eq!(scene.meta().category, "3d-scenes");
        let names: Vec<&str> = scene.props().iter().map(|prop| prop.name).collect();
        assert_eq!(
            &names[names.len() - 4..],
            &["speed", "bloom", "trail", "brightness"]
        );
        assert_eq!(names[0], "gain");
    }

    #[test]
    fn props_clamp_and_trails_scale_the_damp() {
        let mut scene = LatentBackrooms::new();
        scene.apply_props(|key| match key {
            "speed" => Some(9.0),
            "bloom" => Some(-1.0),
            "trail" => Some(2.0),
            "brightness" => Some(1.5),
            _ => None,
        });
        assert_eq!(scene.speed_scale, 4.0);
        assert_eq!(scene.bloom_scale, 0.0);
        assert_eq!(scene.trail_scale, 2.0);
        assert_eq!(scene.brightness_scale, 1.5);
        // `clamp((0.48 + …) * 2, 0, 0.95)`.
        close(scene.afterimage_damp(), 0.95, 1e-6);
    }

    #[test]
    fn update_walks_the_corridor() {
        use crate::point10::audio_bridge::BandPayload;
        let mut scene = LatentBackrooms::new();
        let mut payload = BandPayload::silent(44_100.0);
        // The scene reads `AudioBridge::bands` (lane 0 of each band), so the
        // payload carries lanes, not just FFT magnitudes.
        payload.lanes = [0.5; 12];
        payload.cells = [0.5; 16];
        payload.freq_mag = [0.5; crate::point10::audio_bridge::AUDIO_SAMPLES];
        payload.byte_frequency_data = vec![0; 1024];
        let mut bridge = AudioBridge::new();
        bridge.update(0.1, Some(&payload), &Default::default());

        for _ in 0..600 {
            scene.update(1.0 / 60.0, &bridge);
        }
        assert!(scene.travel() > 0.0);
        assert!(scene.speed() > 6.2);
        let camera = scene.camera_position();
        assert!(camera[2] < START_Z);
        assert!(camera[1] > 1.0);
        assert_eq!(
            scene.current_room(),
            room_cycle(scene.travel()) as usize % ROOM_COUNT
        );
    }

    #[test]
    fn wgsl_parses_and_validates() {
        for (label, program) in [("scene", latent_wgsl()), ("grade", grade_wgsl())] {
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
    fn shaders_keep_the_port_order_and_grade() {
        let scene = latent_wgsl();
        for needle in [
            "fn hash21(p: vec2<f32>) -> f32",
            "let fog = 1.0 - exp(-u.background.w * u.background.w * distance * distance);",
            "return vec4<f32>(in.colour * in.emissive, in.alpha);",
        ] {
            assert!(scene.contains(needle), "scene shader lost {needle}");
        }
        let grade = grade_wgsl();
        for needle in [
            "let base = textureSample(base_texture, source_sampler, uv).rgb;",
            "0.91 + 0.09 * sin(uv.y * u.resolution.y * 3.14159265)",
            "1.0 - smoothstep(0.32, 0.92, length(centred))",
            "max(colour * u.params.x, vec3<f32>(0.0))",
        ] {
            assert!(grade.contains(needle), "grade shader lost {needle}");
        }
    }
}
