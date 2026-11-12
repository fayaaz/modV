//! Shader-renderer uniform contract — the exact set of uniforms every
//! built-in and user-supplied fragment module receives, ported from
//! `src/renderer/src/application/renderers/shader/index.js:90-126`
//! (`generateUniforms`) and `default-shader.js` (the `f` / `fWrap` flavours).
//!
//! # The two buffer model
//!
//! Uniforms are split across **two** uniform buffers, one binding each:
//!
//! * **Binding 0 — shared buffer** ([`ShaderUniforms`], 160 bytes, std140-ish,
//!   16-byte slots). Written once per frame; identical for every module.
//! * **Binding 7 — prop buffer** ([`PropLayout`]). One buffer per module,
//!   holding exactly the module's own props in declaration order, each prop
//!   in its own 16-byte slot. Modules without props bind a 16-byte zeroed
//!   dummy instead (a 0-sized uniform buffer is invalid).
//!
//! The bindings in between are the textures and samplers (see
//! `super::pipeline`):
//!
//! | Binding | Name            | Type                  | Notes                        |
//! |---------|-----------------|-----------------------|------------------------------|
//! | 0       | *(uniform)*     | [`ShaderUniforms`]    | this file, 160 bytes          |
//! | 1..=4   | `iChannel0..3`  | `texture_2d<f32>`     | all alias the group texture   |
//! | 5       | `u_modVCanvas`  | `texture_2d<f32>`     | same view again               |
//! | 6       | `u_fft`         | `texture_2d<f32>`     | 512×1 RGBA8, 1×1 dummy if absent |
//! | 7       | *(uniform)*     | module props / dummy  | this file, [`PropLayout`]     |
//! | 8       | group sampler   | `sampler`             | Linear, Repeat/Repeat         |
//! | 9       | fft sampler     | `sampler`             | Linear, Repeat/Repeat         |
//!
//! # Faithful JavaScript drift items (ported, not fixed)
//!
//! The legacy code has a handful of quirks that are **kept verbatim** so the
//! port matches the reference renderer bit-for-bit in behaviour, not just in
//! unit semantics:
//!
//! * `iTimeDelta` is **not** a delta — `index.js:118` feeds it
//!   `time / 1000`, i.e. the same value as `iTime`. Same for `u_delta`.
//! * `u_time` is absolute **milliseconds** (`performance.now()`), while
//!   `iGlobalTime`/`iTime`/`u_delta` are seconds. The caller passes seconds
//!   and the port multiplies by 1000 (`super::pipeline`).
//! * `u_fftResolution` is a scalar `f32` (the FFT texture **width**, e.g.
//!   `512.0`), matching `default-shader.js`'s `uniform float u_fftResolution`
//!   and `index.js:112` (`fftTexture ? fftTexture.width : 1`). The doc table
//!   in docs/rust-hybrid.md says `vec2`; that table is wrong here and the JS
//!   is the source of truth. A 512×1 texture still has width 512.
//! * `u_kick` is declared as a GLSL `bool`, but WGSL uniform buffers carry no
//!   boolean type, so it travels as `u32` 0/1. In WGSL the member reads as
//!   `u.u_kick != 0u`; in GLSL the cast `float(u_kick)` becomes
//!   `f32(u.u_kick)`.
//! * `iChannelTime` is declared by `default-shader.js` **and** by the WGSL
//!   struct, but nothing ever uploads it (search `index.js` — there is no
//!   `iChannelTime` write anywhere). It stays zero-initialised. The GLSL
//!   `float[4]` is flattened to `vec4<f32>` in WGSL because the uniform
//!   address space requires array strides to be multiples of 16 (naga
//!   rejects `array<f32, 4>` there); nothing indexes the elements.
//! * `iChannelResolution` is per-channel `vec3` in GLSL, which in WGSL is a
//!   16-byte-aligned `vec3` that cannot share a 16-byte slot with anything.
//!   The WGSL declares `array<vec4<f32>, 4>` and uses only `.xyz`; the Rust
//!   struct matches with `[[f32; 4]; 4]` (the `w` lane is padding).
//! * `iFrame` is `frames()` from `worker/frame-counter.js` — monotonically
//!   increasing frames since boot, **never** reset. `i32` here matches the
//!   GLSL `uniform int iFrame`.
//! * `iDate` is `[year, month (0-based), weekday (0 = Sunday), seconds]`
//!   from the JS `Date` getters (`getFullYear`/`getMonth`/`getDay` plus a
//!   monotonic seconds stamp) — not a unix time. The caller builds it.
//!
//! # Byte layout of [`ShaderUniforms`] (160 bytes, `#[repr(C)]`)
//!
//! ```text
//! offset  member                     size  notes
//! 0       i_global_time              4
//! 4       i_frame                    4
//! 8       i_time                     4
//! 12      i_time_delta               4
//! 16      i_date                     16   vec4<f32>
//! 32      i_resolution               12   vec3<f32>
//! 44      _pad0                      4
//! 48      i_channel_time             16   vec4<f32> (GLSL float[4])
//! 64      i_channel_resolution       64   array<vec4<f32>, 4> (xyz used)
//! 128     u_delta                    4
//! 132     u_time                     4
//! 136     u_fft_resolution           4
//! 140     u_kick                     4    u32 0/1
//! 144     u_bass                     4
//! 148     u_mid                      4
//! 152     u_treble                   4
//! 156     _pad1                      4
//! 160     total
//! ```
//!
//! The WGSL [`SHARED_UNIFORM_WGSL`] struct members sit at exactly these
//! offsets (WGSL `vec4` is 16-byte aligned, `vec3` is 16-byte aligned with a
//! 12-byte span, everything else is 4-byte aligned — no implicit padding
//! anywhere except the explicit `_pad0`/`_pad1` members).

#![forbid(unsafe_code)]

use std::fmt;

/// Monotonic seconds (`iGlobalTime` family) — `performance.now() / 1000`.
pub const SECONDS_TO_MILLISECONDS: f32 = 1000.0;

/// Byte size of [`ShaderUniforms`]; must equal the WGSL struct span.
pub const SHADER_UNIFORMS_BYTES: usize = 160;

/// The shared, built-in uniform block. Port of the `defaults` object created
/// by `generateUniforms` in `index.js:94-113`.
///
/// `#[repr(C)]` + `bytemuck::Pod` makes the in-memory layout exact; the WGSL
/// mirror lives in [`SHARED_UNIFORM_WGSL`] and is kept byte-identical by the
/// `wgsl_struct_span_is_160_bytes` test.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ShaderUniforms {
    /// Seconds since engine start (monotonic). `iGlobalTime`.
    pub i_global_time: f32,
    /// Frames since boot (`worker/frame-counter.js`). `iFrame`.
    pub i_frame: i32,
    /// Same value as `i_global_time` (JS drift: `iTime` is *not* a separate
    /// clock). `iTime`.
    pub i_time: f32,
    /// **Not a delta** — JS passes the same value as `i_time`
    /// (`index.js:118`). `iTimeDelta`.
    pub i_time_delta: f32,
    /// `[year, month (0-based), weekday (0 = Sunday), monotonic seconds]`.
    /// `iDate`.
    pub i_date: [f32; 4],
    /// Group-texture `[width, height, dpr]`. `iResolution`.
    pub i_resolution: [f32; 3],
    /// Padding to the 16-byte slot boundary. Carries no data.
    pub _pad0: f32,
    /// Declared in GLSL (`default-shader.js`) and WGSL; **never uploaded**.
    /// Always `[0.0; 4]`. `iChannelTime`.
    pub i_channel_time: [f32; 4],
    /// Per-channel resolutions; every element aliases the same
    /// `[width, height, dpr]`. `.w` is unused padding (WGSL `vec4` alignment).
    /// `iChannelResolution`.
    pub i_channel_resolution: [[f32; 4]; 4],
    /// Same value as `i_global_time` (JS drift). `u_delta`.
    pub u_delta: f32,
    /// Absolute **milliseconds** (`performance.now()`), unlike the
    /// seconds-based fields above. `u_time`.
    pub u_time: f32,
    /// Width of the 512×1 FFT texture — scalar `f32`, e.g. `512.0`.
    /// `u_fftResolution`.
    pub u_fft_resolution: f32,
    /// Beat flag as `u32` 0/1 (WGSL uniform buffers cannot store `bool`).
    /// `u_kick`.
    pub u_kick: u32,
    /// AGC-normalised bass band (0..1). `u_bass`.
    pub u_bass: f32,
    /// AGC-normalised mid band (0..1). `u_mid`.
    pub u_mid: f32,
    /// AGC-normalised treble band (0..1). `u_treble`.
    pub u_treble: f32,
    /// Padding to the 16-byte slot boundary. Carries no data.
    pub _pad1: f32,
}

impl ShaderUniforms {
    /// The width used when no FFT texture is bound this frame; mirrors
    /// `index.js:112` (`fftTexture ? fftTexture.width : 1`).
    pub const NO_FFT_RESOLUTION: f32 = 1.0;
    /// Width of the real FFT texture (`constants.js`: `AUDIO_BUFFER_SIZE`).
    pub const FFT_WIDTH: f32 = 512.0;
}

/// The shared WGSL preamble: the [`ShaderUniforms`] mirror struct, every
/// texture/sampler binding, and the `modv_uv` helper every module uses in
/// place of the GLSL `vUv` varying.
///
/// Kept as one constant so the vertex shader can reuse declarations, and so
/// the `all_shader_modules_parse_with_naga` test exercises exactly what
/// ships to `wgpu::create_shader_module`.
///
/// # Texture binding layout (fixed for every module)
///
/// All of `iChannel0..3` and `u_modVCanvas` are bound to **the same** view —
/// the group texture — mirroring `index.js:103-111` where every one of those
/// uniforms is assigned the same `canvasTexture`. wgpu permits the same
/// texture in multiple bindings of one layout, so the bind group simply
/// repeats the view at 1–5. `u_fft` is the 512×1 RGBA8 FFT texture, or a
/// 1×1 zero texture when no FFT input is available this frame.
///
/// # vUv vs `modv_uv`
///
/// GLSL computes `vUv = vec2(1.0 - x, y)` in *clip* space, which after the
/// `gl_Position = 1.0 - 2.0 * position` mapping works out to a plain
/// normalised `(px / w, py / h)` with the origin at the **top-left** — the
/// same orientation as `@builtin(position)` in wgpu. `modv_uv` therefore does
/// **no** y-flip; an earlier draft flipped y and would have presented every
/// module upside down. `fragCoord` passed to `mainImage` is the wgpu
/// fragment position (top-left origin) rather than the GL bottom-left one;
/// only pixel-grid effects notice, and none of them care about parity.
///
/// # iChannelResolution element type
///
/// GLSL `vec3 iChannelResolution[4]` becomes `array<vec4<f32>, 4>` — WGSL
/// `vec3` elements in an array still take a 16-byte stride, so the fourth
/// lane carries explicit padding and the `xyz` lanes are the value.
pub const SHARED_UNIFORM_WGSL: &str = r#"// modV shader renderer — shared uniform contract.
// Mirrors crates/modv-renderers/src/shader/uniforms.rs (ShaderUniforms).
// Source of truth: src/renderer/src/application/renderers/shader/index.js:90-126
// and default-shader.js (f / fWrap).
struct ShaderUniforms {
  iGlobalTime: f32,
  iFrame: i32,
  iTime: f32,
  iTimeDelta: f32,
  iDate: vec4<f32>,
  iResolution: vec3<f32>,
  _pad0: f32,
  iChannelTime: vec4<f32>,
  iChannelResolution: array<vec4<f32>, 4>,
  u_delta: f32,
  u_time: f32,
  u_fftResolution: f32,
  u_kick: u32,
  u_bass: f32,
  u_mid: f32,
  u_treble: f32,
  _pad1: f32,
}

@group(0) @binding(0) var<uniform> u: ShaderUniforms;
@group(0) @binding(1) var iChannel0: texture_2d<f32>;
@group(0) @binding(2) var iChannel1: texture_2d<f32>;
@group(0) @binding(3) var iChannel2: texture_2d<f32>;
@group(0) @binding(4) var iChannel3: texture_2d<f32>;
@group(0) @binding(5) var u_modVCanvas: texture_2d<f32>;
@group(0) @binding(6) var u_fft: texture_2d<f32>;
@group(0) @binding(8) var modvGroupSampler: sampler;
@group(0) @binding(9) var modvFftSampler: sampler;

// GLSL vUv in clip space folds to a plain normalised top-left-origin uv;
// see the `modv_uv` doc note in uniforms.rs.
fn modv_uv(fragCoord: vec2<f32>) -> vec2<f32> {
  let res = u.iResolution;
  return vec2<f32>(fragCoord.x / res.x, fragCoord.y / res.y);
}
"#;

/// GLSL `mod` semantics: floor-based, sign follows the divisor — implemented
/// as `x - y * floor(x / y)`. WGSL has no built-in `mod`. Modules that need
/// it splice the relevant const into their body instead of bloating the
/// shared preamble.
pub(crate) const GLSL_MOD_F_WGSL: &str = r#"fn glsl_mod_f(x: f32, y: f32) -> f32 {
  return x - y * floor(x / y);
}
"#;

/// Vector form of [`GLSL_MOD_F_WGSL`].
pub(crate) const GLSL_MOD_V2_WGSL: &str = r#"fn glsl_mod_v2(x: vec2<f32>, y: vec2<f32>) -> vec2<f32> {
  return x - y * floor(x / y);
}
"#;

/// The prop-buffer binding: GLSL props become module-level `vec4<f32>`
/// members (one 16-byte slot each) in a `ModuleProps` struct at binding 7.
/// Only the modules whose WGSL declares props introduce this binding; the
/// fixed bind group layout always provides it (see `super::pipeline`).
pub(crate) const MODULE_PROPS_BINDING_WGSL: &str =
    "@group(0) @binding(7) var<uniform> props: ModuleProps;\n";

/// Byte size of one [`PropLayout`] slot (std140-style 16-byte slots).
pub const PROP_SLOT_BYTES: usize = 16;

/// The encoded prop payload the engine hands to [`crate::shader::pipeline`].
///
/// A flat list of raw floats **in the module's declaration order** — the
/// caller (the WS-2 engine wiring) is responsible for JSON-coercing and
/// clamping prop values (see `store/modules/modules.js:481-562`) and for
/// expanding `vecN`/`color` props into their consecutive float lanes. `int` /
/// `bool` / `enum` values arrive as their numeric value; the encoder re-reads
/// the low bits as `i32`/`u32`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PropSlots {
    /// Raw prop values, flattened in declaration order (`vecN` expands to N
    /// consecutive entries).
    pub floats: Vec<f32>,
}

impl PropSlots {
    /// Empty payload — for modules with no props.
    pub const EMPTY: PropSlots = PropSlots { floats: Vec::new() };
}

/// The kind of a module prop; determines the slot's WGSL member type and how
/// [`PropLayout::encode`] writes the bytes.
///
/// In-tree modules only declare [`PropKind::Float`] props; the remaining
/// kinds exist so the encoding and the generated WGSL are total functions
/// over the JS prop-type space (`dataTypes.js`), not because anything in
/// this crate uses them yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PropKind {
    /// 32-bit float — `vec4<f32>` slot, `.x` used.
    Float,
    /// Signed 32-bit int — `vec4<i32>` slot, `.x` used.
    Int,
    /// Boolean — `vec4<u32>` slot, `.x` is 0/1.
    Bool,
    /// Enum (long) — `vec4<u32>` slot, `.x` is the option index.
    Enum,
    /// `vec2` — `vec4<f32>` slot, `.xy` used.
    Vec2,
    /// `vec3` — `vec4<f32>` slot, `.xyz` used.
    Vec3,
    /// `vec4` — `vec4<f32>` slot, all lanes used.
    Vec4,
    /// `color` — `vec4<f32>` slot, all lanes used (r, g, b, a).
    Color,
}

impl PropKind {
    /// Number of `f32` lanes the caller must supply for one prop.
    pub const fn float_lanes(self) -> usize {
        match self {
            PropKind::Float | PropKind::Int | PropKind::Bool | PropKind::Enum => 1,
            PropKind::Vec2 => 2,
            PropKind::Vec3 => 3,
            PropKind::Vec4 | PropKind::Color => 4,
        }
    }

    /// WGSL member type of the prop's slot.
    pub const fn wgsl_member_type(self) -> &'static str {
        match self {
            PropKind::Float
            | PropKind::Vec2
            | PropKind::Vec3
            | PropKind::Vec4
            | PropKind::Color => "vec4<f32>",
            PropKind::Int => "vec4<i32>",
            PropKind::Bool | PropKind::Enum => "vec4<u32>",
        }
    }
}

/// A module prop as declared in the JS module definition — the order of this
/// list **is** the prop-buffer slot order and the WGSL `ModuleProps` member
/// order.
///
/// The UI metadata (`label`/`min`/`max`/`step`/`default`) travels along so
/// the registry is self-describing; the encoder ignores it. All in-tree
/// props are floats, so the numeric metadata is `f64` unconditionally —
/// vector/color props would need richer defaults and can be added when a
/// module needs them.
#[derive(Clone, Debug, PartialEq)]
pub struct ShaderProp {
    /// Prop name as it appears in the shader and the preset JSON
    /// (e.g. `u_ringWidth`).
    pub name: &'static str,
    /// Prop kind; drives the slot's WGSL member type and encoder.
    pub kind: PropKind,
    /// Gallery label (`props.x.label` in the JS module).
    pub label: &'static str,
    /// Inclusive minimum for `strict` float props (`props.x.min`).
    pub min: f64,
    /// Inclusive maximum for `strict` float props (`props.x.max`).
    pub max: f64,
    /// UI step (`props.x.step`).
    pub step: f64,
    /// Default value (`props.x.default`).
    pub default: f64,
}

/// One slot in a [`PropLayout`]: where the prop lives in the prop buffer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PropSlot {
    /// Prop name; matches the `ModuleProps` member name.
    pub name: &'static str,
    /// Prop kind.
    pub kind: PropKind,
    /// Byte offset of the 16-byte slot inside the prop buffer.
    pub offset: usize,
}

/// The per-module prop buffer layout, derived from the module's prop
/// declaration order.
///
/// Every prop occupies a 16-byte slot (`vec4`), so the offsets are simply
/// `index * 16`. An empty prop list still produces a 16-byte dummy layout so
/// binding 7 is never a 0-sized buffer.
#[derive(Clone, Debug, PartialEq)]
pub struct PropLayout {
    /// Slots in declaration order.
    pub slots: Vec<PropSlot>,
    /// Total prop buffer size in bytes; always ≥ 16 and a multiple of 16.
    pub buffer_size: usize,
}

impl PropLayout {
    /// Derive the layout from a module's declared props.
    pub fn from_props(props: &[ShaderProp]) -> Self {
        let slots = props
            .iter()
            .enumerate()
            .map(|(index, prop)| PropSlot {
                name: prop.name,
                kind: prop.kind,
                offset: index * PROP_SLOT_BYTES,
            })
            .collect::<Vec<_>>();
        let buffer_size = (slots.len() * PROP_SLOT_BYTES).max(PROP_SLOT_BYTES);
        Self { slots, buffer_size }
    }

    /// The total number of `f32` lanes encoded in the buffer.
    pub fn expected_floats(&self) -> usize {
        self.slots.iter().map(|slot| slot.kind.float_lanes()).sum()
    }

    /// Encode a caller-provided [`PropSlots`] payload into prop-buffer bytes.
    ///
    /// The payload length must equal [`PropLayout::expected_floats`]; the
    /// check is the only thing standing between a malformed preset and a
    /// silently misaligned uniform block, so it is enforced here rather than
    /// at the call site.
    pub fn encode(&self, slots: &PropSlots) -> Result<Vec<u8>, PropEncodeError> {
        let expected = self.expected_floats();
        if slots.floats.len() != expected {
            return Err(PropEncodeError::LengthMismatch {
                expected,
                got: slots.floats.len(),
            });
        }

        let mut bytes = vec![0_u8; self.buffer_size];
        let mut cursor = 0;
        for slot in &self.slots {
            let lane_count = slot.kind.float_lanes();
            let lanes = &slots.floats[cursor..cursor + lane_count];
            match slot.kind {
                PropKind::Float
                | PropKind::Vec2
                | PropKind::Vec3
                | PropKind::Vec4
                | PropKind::Color => {
                    for (lane, value) in lanes.iter().enumerate() {
                        let start = slot.offset + lane * 4;
                        bytes[start..start + 4].copy_from_slice(&value.to_le_bytes());
                    }
                }
                PropKind::Int => {
                    let value = lanes[0] as i32;
                    bytes[slot.offset..slot.offset + 4].copy_from_slice(&value.to_le_bytes());
                }
                PropKind::Bool => {
                    let value: u32 = if lanes[0] != 0.0 { 1 } else { 0 };
                    bytes[slot.offset..slot.offset + 4].copy_from_slice(&value.to_le_bytes());
                }
                PropKind::Enum => {
                    let value = lanes[0] as u32;
                    bytes[slot.offset..slot.offset + 4].copy_from_slice(&value.to_le_bytes());
                }
            }
            cursor += lane_count;
        }
        Ok(bytes)
    }
}

/// Errors produced while encoding the prop buffer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PropEncodeError {
    /// The caller supplied the wrong number of float lanes for the module's
    /// declared props.
    LengthMismatch {
        /// Expected lane count (sum of the prop kinds' lane widths).
        expected: usize,
        /// Lane count actually supplied.
        got: usize,
    },
}

impl fmt::Display for PropEncodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PropEncodeError::LengthMismatch { expected, got } => {
                write!(
                    f,
                    "prop payload length mismatch: expected {expected} float lanes, got {got}"
                )
            }
        }
    }
}

impl std::error::Error for PropEncodeError {}

/// Generate the WGSL `ModuleProps` declaration for a module, or `None` when
/// the module declares no props (its WGSL then simply does not declare
/// binding 7 — the fixed bind group layout always provides a binding for it,
/// bound to a dummy).
///
/// Members are one `vec4` slot per prop in declaration order. The member
/// types mirror [`PropKind::wgsl_member_type`] so the bytes written by
/// [`PropLayout::encode`] decode to exactly the value the shader reads.
pub(crate) fn module_props_wgsl(props: &[ShaderProp]) -> Option<String> {
    if props.is_empty() {
        return None;
    }
    let mut source = String::from("struct ModuleProps {\n");
    for prop in props {
        source.push_str("  ");
        source.push_str(prop.name);
        source.push_str(": ");
        source.push_str(prop.kind.wgsl_member_type());
        source.push_str(",\n");
    }
    source.push_str("}\n");
    source.push_str(MODULE_PROPS_BINDING_WGSL);
    Some(source)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_buffer_layout_is_160_bytes() {
        assert_eq!(
            std::mem::size_of::<ShaderUniforms>(),
            SHADER_UNIFORMS_BYTES,
            "ShaderUniforms must stay 160 bytes — the WGSL mirror is hand-aligned to it"
        );
        assert_eq!(std::mem::align_of::<ShaderUniforms>(), 4);
    }

    #[test]
    fn wgsl_struct_span_is_160_bytes() {
        // naga's layout calculator gives the authoritative WGSL struct span,
        // which must equal the Rust struct size — wgpu rejects draws whose
        // bound buffer is smaller than what the shader expects.
        let module = naga::front::wgsl::parse_str(SHARED_UNIFORM_WGSL)
            .expect("SHARED_UNIFORM_WGSL must parse");
        let struct_type = module
            .types
            .iter()
            .find_map(|(_, ty)| match &ty.inner {
                naga::TypeInner::Struct { span, .. }
                    if ty.name.as_deref() == Some("ShaderUniforms") =>
                {
                    Some(*span)
                }
                _ => None,
            })
            .expect("ShaderUniforms struct must exist in the WGSL module");
        assert_eq!(struct_type as usize, SHADER_UNIFORMS_BYTES);
    }

    #[test]
    fn prop_layout_matches_wgsl_member_order() {
        let props = vec![
            ShaderProp {
                name: "u_ringWidth",
                kind: PropKind::Float,
                label: "Ring Width",
                min: 0.01,
                max: 0.5,
                step: 0.01,
                default: 0.05,
            },
            ShaderProp {
                name: "strength",
                kind: PropKind::Float,
                label: "Strength",
                min: 0.0,
                max: 0.05,
                step: 0.001,
                default: 0.001,
            },
        ];
        let layout = PropLayout::from_props(&props);
        assert_eq!(layout.slots[0].offset, 0);
        assert_eq!(layout.slots[1].offset, PROP_SLOT_BYTES);
        assert_eq!(layout.buffer_size, 2 * PROP_SLOT_BYTES);
        assert_eq!(layout.expected_floats(), 2);

        let bytes = layout
            .encode(&PropSlots {
                floats: vec![0.25, 1.5],
            })
            .expect("encode");
        assert_eq!(bytes.len(), 32);
        assert_eq!(&bytes[0..4], &0.25_f32.to_le_bytes());
        assert_eq!(&bytes[16..20], &1.5_f32.to_le_bytes());
        assert!(bytes[4..16].iter().all(|b| *b == 0));

        // The WGSL member order must match the layout order.
        let wgsl = module_props_wgsl(&props).expect("two props generate a ModuleProps struct");
        let first = wgsl.find("u_ringWidth").expect("member order");
        let second = wgsl.find("strength").expect("member order");
        assert!(first < second);
    }

    #[test]
    fn encode_rejects_wrong_lane_count() {
        let props = vec![ShaderProp {
            name: "u_ringWidth",
            kind: PropKind::Float,
            label: "Ring Width",
            min: 0.01,
            max: 0.5,
            step: 0.01,
            default: 0.05,
        }];
        let layout = PropLayout::from_props(&props);
        let err = layout
            .encode(&PropSlots { floats: vec![] })
            .expect_err("empty payload must fail");
        assert_eq!(
            err,
            PropEncodeError::LengthMismatch {
                expected: 1,
                got: 0
            }
        );
    }

    #[test]
    fn empty_layout_encodes_a_16_byte_dummy() {
        let layout = PropLayout::from_props(&[]);
        assert_eq!(layout.buffer_size, PROP_SLOT_BYTES);
        let bytes = layout.encode(&PropSlots::EMPTY).expect("encode");
        assert_eq!(bytes.len(), PROP_SLOT_BYTES);
        assert!(bytes.iter().all(|b| *b == 0));
        assert!(module_props_wgsl(&[]).is_none());
    }
}
