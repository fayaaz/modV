//! ES → desktop-GLSL 450 preprocessor and the ISF runtime macro set.
//!
//! This is the Rust port of `ISFParser.generateShaders` /
//! `buildFragmentShader` / `buildVertexShader` / `replaceSpecialFunctions`
//! (`ISFParser.js:78-260`) plus the ES-isms rewrite that naga's GLSL front-end
//! needs (naga only accepts `#version 440/450/460` core — see
//! `docs/isf-translation-notes.md`).
//!
//! # What gets emitted
//!
//! For every fragment pass ([`build_fragment_glsl`]):
//!
//! ```glsl
//! #version 450
//! #define PI 3.14159265358979323846          // only when the shader does not
//! #define ISF_PASS_INDEX 0
//! layout(binding = 0) uniform IsfUniforms {   // one block, no instance name
//!   vec4 DATE;                                // so members are global
//!   vec2 RENDERSIZE;
//!   int PASSINDEX;
//!   int FRAMEINDEX;
//!   float TIME;
//!   float TIMEDELTA;
//!   float bright;                             // ISF inputs, declaration order
//!   vec2 _inputImage_imgSize;
//!   vec4 _inputImage_imgRect;
//!   int _inputImage_flip;
//! };
//! in vec2 isf_FragNormCoord;
//! #define isf_FragCoord (floor(gl_FragCoord.xy))
//! layout(location = 0) out vec4 isf_FragColor;
//! layout(binding = 1) uniform texture2D isf_tex_inputImage;
//! layout(binding = 2) uniform sampler isf_smp_inputImage;
//! // VVSAMPLER_2DBYPIXEL / VVSAMPLER_2DBYNORM macros
//! // … the shader body, macro-expanded and rewritten …
//! ```
//!
//! The vendored runtime declares the same values as *loose* uniforms
//! (`ISFParser.fragmentShaderSkeleton`). naga's GLSL front-end requires every
//! uniform to carry a `layout(binding)`, which loose scalars cannot express,
//! so the port packs them into one uniform block. The member set, order and
//! WGSL-layout-compatible offsets are produced by [`uniform_members`] and
//! [`uniform_layout`] so the runtime can byte-encode the same block.
//!
//! # Runtime uniforms (exact names, `ISFParser.fragmentShaderSkeleton`)
//!
//! | Name | GLSL type | Meaning |
//! |------|-----------|---------|
//! | `PASSINDEX` | `int` | index of the pass currently rendering |
//! | `RENDERSIZE` | `vec2` | size of the pass target in pixels |
//! | `TIME` | `float` | seconds since engine start (`setDateUniforms`) |
//! | `TIMEDELTA` | `float` | seconds since the previous frame |
//! | `FRAMEINDEX` | `int` | frames rendered (starts at 1 in the JS) |
//! | `DATE` | `vec4` | `[year, month 1-based, day, secs since midnight]` |
//! | `PI` | `float` | injected define when the shader has none |
//!
//! 65 corpus shaders use `TIME` without declaring it; the block always
//! provides it, exactly like the JavaScript skeleton.
//!
//! # Input uniform mapping (`ISFParser.typeUniformMap`)
//!
//! | `TYPE` | GLSL member | Notes |
//! |--------|-------------|-------|
//! | `float` | `float NAME` | |
//! | `bool`, `event` | `int isf_bool_NAME` + a `bool NAME` global | WGSL forbids `bool` in a uniform block, so the value travels as `i32` and is copied into a private global at the top of the entry point |
//! | `long` | `int NAME` | |
//! | `color` | `vec4 NAME` | |
//! | `point2D` | `vec2 NAME` | |
//! | `image`, `audio`, `audioFFT` | `texture2D` + `sampler` pair | bound from `ISFTexture` inputs |
//!
//! The three texture metadata uniforms per sampler are the vendored contract:
//! `_<name>_imgSize` (`vec2`), `_<name>_imgRect` (`vec4`, always
//! `[0, 0, 1, 1]`) and `_<name>_flip` (`bool` in the JS, `int` here for the
//! same WGSL reason).
//!
//! # Macro expansion (`ISFParser.replaceSpecialFunctions`, lines 144-260)
//!
//! The expansion order and the replacement text are the JavaScript's:
//!
//! | Source | Expansion |
//! |--------|-----------|
//! | `IMG_THIS_PIXEL(x)` | `texture(x, isf_FragNormCoord)` (the JS bug: it uses normalised coordinates for both `IMG_THIS_*` macros, kept verbatim) |
//! | `IMG_THIS_NORM_PIXEL(x)` | `texture(x, isf_FragNormCoord)` |
//! | `IMG_PIXEL(s, c)` | `texture(s, (c) / RENDERSIZE)` |
//! | `IMG_NORM_PIXEL(s, c)` | `VVSAMPLER_2DBYNORM(s, _s_imgRect, _s_imgSize, _s_flip, c)` |
//! | `IMG_SIZE(s)` | `_s_imgSize` |
//!
//! `VVSAMPLER_2DBYPIXEL`/`BYNORM` are emitted as GLSL *macros* instead of the
//! JavaScript functions: naga's GLSL front-end cannot take a combined
//! `sampler2D` as a function parameter, while a macro composes fine.
//!
//! # ES → 450 rewrite
//!
//! 1. `#version …` lines are dropped (the prelude carries `#version 450`).
//! 2. `precision …` lines are dropped (naga infers precision).
//! 3. `texture2D` → `texture`, `texture2DProj` → `textureProj`,
//!    `texture2DLod` → `textureLod`, `texture2DGrad` → `textureGrad`,
//!    `textureCube` → `texture` (identifier-aware, so `my_texture2D` is safe).
//!    `texture2DRect`/`sampler2DRect` are **not** rewritten: they sample in
//!    pixel coordinates, so mapping them to normalised `texture()` would be
//!    silently wrong. Such shaders are rejected with
//!    [`super::translate::TranslateError::Unsupported`].
//! 4. `gl_FragColor` → the declared `isf_FragColor` output.
//! 5. In the fragment, a shader-declared `in`/`varying`/`uniform` line loses
//!    its storage qualifier and becomes a plain global (`vec2 x;`), which
//!    naga lowers to a zero-initialised `var<private>`. This reproduces the
//!    GLSL compiler's treatment of the 37 corpus shaders that carry template
//!    leftover varyings (`left_coord`, `texOffsets`, …): they are either
//!    dead-code eliminated or read as zero. A `sampler2D` cannot be an entry
//!    point input in WGSL without a matching vertex output, and the runtime
//!    draws every ISF pass with the built-in full-screen triangle, so the
//!    qualifier is removed rather than forwarded.
//! 6. In the vertex stage `attribute` → `in`, `varying` → `out`.
//! 7. A non-`const` global with an initialiser is split into the declaration
//!    (a private, zero-initialised global) plus an assignment in a generated
//!    `isf_init_globals()` called before the shader's `main`:
//!    WGSL has no module-scope initialiser that can read a uniform
//!    (`Pixelize`'s `float d = min(progress, 1.0 - progress);`).
//! 8. Functions with `sampler2D` parameters are cloned per actual texture at
//!    their call sites (the parameter is replaced by the texture's
//!    `sampler2D(texture, sampler)` pair); naga cannot take a sampler as a
//!    function argument (`Point10 *`).
//! 9. A non-void function whose last top-level statement is not a `return`
//!    gets a deterministic `return <zero>;` appended; WGSL needs a value on
//!    every path (`Shape Morph`'s `shapeForType`).
//! 10. `in`/`out`/`varying mat2|mat3|mat4` declarations are split into one
//!     vector varying per column, matrix assignments write the columns and
//!     matrix reads rebuild the value; WGSL entry-point IO cannot carry a
//!     matrix (`v002 Light Leak`).
//!
//! # Deliberate coordinate deviation (documented, not hidden)
//!
//! wgpu's `@builtin(position)` is top-left origin; GL's `gl_FragCoord` is
//! bottom-left. `isf_FragNormCoord` is therefore top-left origin here, which
//! keeps texture sampling visually identical to the Electron renderer (in
//! wgpu, texture `v = 0` is also the first row) while mirroring shaders that
//! treat `isf_FragNormCoord` as a Cartesian coordinate. This matches the
//! convention already documented for the shader renderer (`shader/mod.rs`).
//! `isf_FragCoord` is a macro over `gl_FragCoord` rather than a varying: the
//! vendored vertex skeleton assigns a different case (`isf_fragCoord`) and
//! never declares it as an output, so the original is an unbound varying in
//! JavaScript and would be a link error whenever a shader actually reads it.

#![forbid(unsafe_code)]

use std::fmt;

use super::parser::{IsfDocument, IsfInput};

/// GLSL definitions for the two texture-sampling helpers.
///
/// Ported line for line from `ISFParser.fragmentShaderSkeleton`
/// (`ISFParser.js:300-313`). `flip` is an `int` (WGSL uniform rules) and is
/// compared against zero.
pub const VVSAMPLER_MACROS: &str = r#"#define VVSAMPLER_2DBYPIXEL(isf_sampler, isf_rect, isf_size, isf_flip, isf_loc) \
  (((isf_flip) != 0) ? texture(isf_sampler, vec2((((isf_loc).x / (isf_size).x) * (isf_rect).z) + (isf_rect).x, ((isf_rect).w - (((isf_loc).y / (isf_size).y) * (isf_rect).w)) + (isf_rect).y)) : texture(isf_sampler, vec2((((isf_loc).x / (isf_size).x) * (isf_rect).z) + (isf_rect).x, (((isf_loc).y / (isf_size).y) * (isf_rect).w) + (isf_rect).y)))
#define VVSAMPLER_2DBYNORM(isf_sampler, isf_rect, isf_size, isf_flip, isf_norm) \
  (VVSAMPLER_2DBYPIXEL(isf_sampler, isf_rect, isf_size, isf_flip, vec2((isf_norm).x * (isf_size).x, (isf_norm).y * (isf_size).y)))
"#;

/// The default vertex body (`ISFParser.vertexShaderDefault`).
pub const DEFAULT_VERTEX_BODY: &str = "void main() { isf_vertShaderInit(); }\n";

/// The type of a uniform-block member. Only the scalar/vector types the ISF
/// uniform contract uses are represented; textures are separate bindings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UniformKind {
    /// `float` — 4-byte aligned, 4-byte size.
    Float,
    /// `int` — also `bool`/`event`/`long` inputs (WGSL forbids `bool` in a
    /// uniform buffer).
    Int,
    /// `vec2` — 8-byte aligned, 8-byte size.
    Vec2,
    /// `vec4` — 16-byte aligned, 16-byte size.
    Vec4,
}

impl UniformKind {
    /// GLSL spelling inside the block.
    pub fn glsl_type(self) -> &'static str {
        match self {
            UniformKind::Float => "float",
            UniformKind::Int => "int",
            UniformKind::Vec2 => "vec2",
            UniformKind::Vec4 => "vec4",
        }
    }

    /// WGSL/WGSL-uniform `(align, size)` in bytes.
    pub fn layout(self) -> (usize, usize) {
        match self {
            UniformKind::Float | UniformKind::Int => (4, 4),
            UniformKind::Vec2 => (8, 8),
            UniformKind::Vec4 => (16, 16),
        }
    }
}

/// What a block member is filled from every frame.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum UniformSource {
    /// `PASSINDEX`.
    PassIndex,
    /// `RENDERSIZE`.
    RenderSize,
    /// `TIME`.
    Time,
    /// `TIMEDELTA`.
    TimeDelta,
    /// `FRAMEINDEX`.
    FrameIndex,
    /// `DATE`.
    Date,
    /// An ISF `float`/`long`/`bool`/`color`/`point2D` input, by name.
    Input(String),
    /// `_<texture>_imgSize`.
    ImageSize(String),
    /// `_<texture>_imgRect`.
    ImageRect(String),
    /// `_<texture>_flip`.
    ImageFlip(String),
}

/// One member of the generated `IsfUniforms` block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UniformMember {
    /// Member name in GLSL/WGSL.
    pub name: String,
    /// Member type.
    pub kind: UniformKind,
    /// Per-frame value source.
    pub source: UniformSource,
}

/// A texture bound to the fragment stage, with its WGSL binding numbers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextureBinding {
    /// ISF input / pass-target name (the macro the shader uses).
    pub name: String,
    /// `@binding` of the `texture_2d<f32>`.
    pub texture_binding: u32,
    /// `@binding` of the `sampler`.
    pub sampler_binding: u32,
}

/// The uniform-block layout the WGSL backend/WGPU will use.
///
/// WGSL uniform address-space rules for the types this module emits: scalars
/// align to 4, `vec2` to 8, `vec4` to 16; the struct itself is 16-byte
/// aligned and its size is rounded up to a multiple of 16. The Rust encoder
/// writes each member at [`MemberLayout::offset`], so the byte block and the
/// shader agree by construction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UniformLayout {
    /// Total buffer size in bytes (multiple of 16).
    pub size: usize,
    /// Members with their byte offsets, in declaration order.
    pub members: Vec<MemberLayout>,
}

/// A laid-out uniform member.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemberLayout {
    /// Member name.
    pub name: String,
    /// Member type.
    pub kind: UniformKind,
    /// Per-frame source.
    pub source: UniformSource,
    /// Byte offset from the start of the block.
    pub offset: usize,
}

/// Options a size formula is evaluated against.
///
/// The scalar values mirror the uniforms the JavaScript `evaluateSize`
/// substitutes: `$WIDTH`, `$HEIGHT` and `$<input>`, plus the runtime scalars
/// `$TIME`, `$TIMEDELTA`, `$FRAMEINDEX` and `$PASSINDEX`. Vector values
/// (`RENDERSIZE`, `DATE`, `_<texture>_imgSize`) have no scalar spelling and
/// are rejected as unknown variables, which becomes a per-pass
/// [`SizeEvalError`] rather than a dead module.
#[derive(Clone, Debug, PartialEq)]
pub struct SizeVars {
    /// `$WIDTH` — the destination width in pixels.
    pub width: f64,
    /// `$HEIGHT` — the destination height in pixels.
    pub height: f64,
    /// `$TIME` — seconds since the engine started.
    pub time: f64,
    /// `$TIMEDELTA` — seconds since the previous frame.
    pub time_delta: f64,
    /// `$FRAMEINDEX` — the 1-based frame counter.
    pub frame_index: f64,
    /// `$PASSINDEX` — the pass being sized.
    pub pass_index: f64,
    /// `$<inputName>` values (the JavaScript substitutes every uniform's
    /// current value into the formula).
    pub inputs: Vec<(String, f64)>,
    /// Upper clamp, normally the device's `max_texture_dimension_2d`.
    pub max_dimension: u32,
}

/// Conservative upper bound used when no device limit is at hand
/// (`wgpu::Limits::default().max_texture_dimension_2d`).
pub const DEFAULT_MAX_TEXTURE_DIMENSION: u32 = 8192;

impl Default for SizeVars {
    fn default() -> Self {
        Self {
            width: 0.0,
            height: 0.0,
            time: 0.0,
            time_delta: 0.0,
            frame_index: 0.0,
            pass_index: 0.0,
            inputs: Vec::new(),
            max_dimension: DEFAULT_MAX_TEXTURE_DIMENSION,
        }
    }
}

/// The textures a document samples: `image`/`audio`/`audioFFT` inputs in
/// declaration order, then pass targets, then `IMPORTED` keys
/// (`ISFParser.generateShaders`, lines 84-105).
pub fn texture_inputs(doc: &IsfDocument) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let mut push = |name: &str| {
        if !names.iter().any(|existing| existing == name) {
            names.push(name.to_string());
        }
    };
    for input in &doc.inputs {
        if is_texture_type(&input.type_) {
            push(&input.name);
        }
    }
    for pass in &doc.passes {
        if let Some(target) = pass.target.as_deref() {
            push(target);
        }
    }
    if let Some(imported) = doc
        .metadata
        .get("IMPORTED")
        .and_then(|value| value.as_object())
    {
        let mut keys: Vec<&String> = imported.keys().collect();
        keys.sort();
        for key in keys {
            push(key);
        }
    }
    names
}

/// Whether an ISF input type is sampled as a texture.
pub fn is_texture_type(type_: &str) -> bool {
    matches!(type_, "image" | "audio" | "audioFFT")
}

/// Bindings for [`texture_inputs`]: binding 0 is the uniform block, then each
/// texture gets a `(texture, sampler)` pair in order — texture at `2*i + 1`,
/// sampler at `2*i + 2`.
pub fn texture_bindings(doc: &IsfDocument) -> Vec<TextureBinding> {
    texture_inputs(doc)
        .into_iter()
        .enumerate()
        .map(|(index, name)| TextureBinding {
            name,
            texture_binding: (index as u32) * 2 + 1,
            sampler_binding: (index as u32) * 2 + 2,
        })
        .collect()
}

/// The uniform block members for a document, in the fixed order the
/// preprocessor emits them.
pub fn uniform_members(doc: &IsfDocument) -> Vec<UniformMember> {
    let mut members = vec![
        UniformMember {
            name: "DATE".into(),
            kind: UniformKind::Vec4,
            source: UniformSource::Date,
        },
        UniformMember {
            name: "RENDERSIZE".into(),
            kind: UniformKind::Vec2,
            source: UniformSource::RenderSize,
        },
        UniformMember {
            name: "PASSINDEX".into(),
            kind: UniformKind::Int,
            source: UniformSource::PassIndex,
        },
        UniformMember {
            name: "FRAMEINDEX".into(),
            kind: UniformKind::Int,
            source: UniformSource::FrameIndex,
        },
        UniformMember {
            name: "TIME".into(),
            kind: UniformKind::Float,
            source: UniformSource::Time,
        },
        UniformMember {
            name: "TIMEDELTA".into(),
            kind: UniformKind::Float,
            source: UniformSource::TimeDelta,
        },
    ];
    for input in &doc.inputs {
        if let Some((name, kind)) = input_member(input) {
            members.push(UniformMember {
                name,
                kind,
                source: UniformSource::Input(input.name.clone()),
            });
        }
    }
    for texture in texture_inputs(doc) {
        members.push(UniformMember {
            name: format!("_{texture}_imgSize"),
            kind: UniformKind::Vec2,
            source: UniformSource::ImageSize(texture.clone()),
        });
        members.push(UniformMember {
            name: format!("_{texture}_imgRect"),
            kind: UniformKind::Vec4,
            source: UniformSource::ImageRect(texture.clone()),
        });
        members.push(UniformMember {
            name: format!("_{texture}_flip"),
            kind: UniformKind::Int,
            source: UniformSource::ImageFlip(texture),
        });
    }
    members
}

/// The block member for a scalar/vector input, or `None` for textures and
/// unknown types (`ISFParser.addUniform` → `typeUniformMap`).
fn input_member(input: &IsfInput) -> Option<(String, UniformKind)> {
    let member = match input.type_.as_str() {
        "float" => (sanitised_input_name(&input.name), UniformKind::Float),
        "bool" | "event" => (
            format!("isf_bool_{}", sanitised_input_name(&input.name)),
            UniformKind::Int,
        ),
        "long" => (sanitised_input_name(&input.name), UniformKind::Int),
        "color" => (sanitised_input_name(&input.name), UniformKind::Vec4),
        "point2D" => (sanitised_input_name(&input.name), UniformKind::Vec2),
        _ => return None,
    };
    Some(member)
}

/// Words that cannot be used as a GLSL uniform-block member name.
///
/// Every name here is prefixed with `isf_input_` in the generated block and in
/// the shader body — see [`sanitised_input_name`] and [`body_input_renames`].
/// The *prop* keeps the original name, so presets are unaffected.
///
/// `UltimateSpiral.fs` has an input named `smooth`, which naga tokenises as an
/// interpolation qualifier, so a block member called `smooth` cannot parse.
pub const RESERVED_INPUT_NAMES: &[&str] = &[
    "layout",
    "in",
    "out",
    "uniform",
    "buffer",
    "shared",
    "invariant",
    "flat",
    "noperspective",
    "smooth",
    "centroid",
    "sample",
    "const",
    "inout",
    "precision",
    "highp",
    "mediump",
    "lowp",
    "restrict",
    "readonly",
    "writeonly",
    "if",
    "else",
    "switch",
    "case",
    "default",
    "while",
    "do",
    "for",
    "return",
    "break",
    "continue",
    "discard",
    "struct",
    "void",
    "true",
    "false",
];

/// GLSL statement/declaration keywords that a shader body cannot reference as
/// an identifier.
///
/// An input with one of these names still needs a prefixed *declaration*
/// (`float isf_input_return;`), but its body occurrences are the keywords
/// themselves: rewriting `return`/`for`/`if` would corrupt every shader that
/// has a legal-looking input name plus control flow (R-26). Such an input is
/// unreachable from GLSL source — exactly as it is in the vendored renderer,
/// where the GLSL compiler rejects the identifier.
pub const GLSL_STATEMENT_KEYWORDS: &[&str] = &[
    "if", "else", "switch", "case", "default", "while", "do", "for", "return", "break", "continue",
    "discard", "struct", "void", "true", "false",
];

/// The GLSL identifier an input is declared as.
pub fn sanitised_input_name(name: &str) -> String {
    if RESERVED_INPUT_NAMES.contains(&name) {
        format!("isf_input_{name}")
    } else {
        name.to_string()
    }
}

/// `(original, sanitised)` pairs for inputs whose name is a reserved word and
/// can safely be rewritten in the body.
///
/// Statement keywords are excluded: only *declarations* are renamed for those
/// (see [`GLSL_STATEMENT_KEYWORDS`]).
fn body_input_renames(doc: &IsfDocument) -> Vec<(String, String)> {
    doc.inputs
        .iter()
        .filter_map(|input| {
            if GLSL_STATEMENT_KEYWORDS.contains(&input.name.as_str()) {
                return None;
            }
            let sanitised = sanitised_input_name(&input.name);
            (sanitised != input.name).then(|| (input.name.clone(), sanitised))
        })
        .collect()
}

/// Compute the WGSL uniform layout of a member list.
pub fn uniform_layout(members: &[UniformMember]) -> UniformLayout {
    let mut offset = 0usize;
    let mut laid_out = Vec::with_capacity(members.len());
    for member in members {
        let (align, size) = member.kind.layout();
        offset = round_up(offset, align);
        laid_out.push(MemberLayout {
            name: member.name.clone(),
            kind: member.kind,
            source: member.source.clone(),
            offset,
        });
        offset += size;
    }
    UniformLayout {
        size: round_up(offset, 16).max(16),
        members: laid_out,
    }
}

fn round_up(value: usize, align: usize) -> usize {
    value.div_ceil(align) * align
}

/// Build the complete per-pass fragment GLSL for naga.
///
/// `pass_index` is only used for the `ISF_PASS_INDEX` define; the shader body
/// is identical for every pass (as in the vendored renderer, which compiles
/// one program for all passes and uploads `PASSINDEX` per pass).
pub fn build_fragment_glsl(doc: &IsfDocument, pass_index: usize) -> String {
    let body = rewrite_fragment_body(doc);
    let members = uniform_members(doc);
    let bindings = texture_bindings(doc);

    let mut out = String::with_capacity(body.source.len() + 4096);
    out.push_str("#version 450\n\n");
    if !declares_identifier(&doc.fragment_source, "PI") {
        out.push_str("#define PI 3.14159265358979323846\n");
    }
    out.push_str(&format!("#define ISF_PASS_INDEX {pass_index}\n\n"));

    out.push_str("layout(binding = 0) uniform IsfUniforms {\n");
    for member in &members {
        out.push_str("  ");
        out.push_str(member.kind.glsl_type());
        out.push(' ');
        out.push_str(&member.name);
        out.push_str(";\n");
    }
    out.push_str("};\n\n");

    out.push_str("in vec2 isf_FragNormCoord;\n");
    out.push_str("#define isf_FragCoord (floor(gl_FragCoord.xy))\n\n");
    out.push_str("layout(location = 0) out vec4 isf_FragColor;\n\n");

    out.push_str(&bool_globals(doc));
    out.push_str(&texture_declarations(&bindings));
    out.push_str(VVSAMPLER_MACROS);
    out.push('\n');
    out.push_str(&body.source);
    out.push_str(&global_init_function(&body.global_initialisers));
    out.push_str(&entry_wrapper(doc, !body.global_initialisers.is_empty()));
    out
}

/// Declarations of the sampled textures and samplers.
///
/// The samplers are *not* given a `#define <name> sampler2D(...)` shortcut:
/// corpus shaders such as `Interlace.fs` declare a local with the same name as
/// a pass target (`vec4 lastRow = IMG_PIXEL(lastRow, vec2(0.5));`), and a
/// macro would rewrite the local's declarator too. Instead the sampler is
/// substituted into the call site by [`substitute_sampler_arguments`].
fn texture_declarations(bindings: &[TextureBinding]) -> String {
    let mut out = String::new();
    for binding in bindings {
        out.push_str(&format!(
            "layout(binding = {}) uniform texture2D isf_tex_{};\n",
            binding.texture_binding, binding.name
        ));
        out.push_str(&format!(
            "layout(binding = {}) uniform sampler isf_smp_{};\n",
            binding.sampler_binding, binding.name
        ));
    }
    out.push('\n');
    out
}

/// Private `bool` globals for `bool`/`event` inputs.
///
/// WGSL forbids `bool` in a uniform block, so the value travels as
/// `int isf_bool_<name>` and is copied into a `bool` global at the top of the
/// entry point. A macro (`#define r bool(isf_bool_r)`) is not an option: the
/// corpus has inputs called `r`, and the macro would rewrite every `.r`
/// swizzle. A private global is shadowed correctly by locals and leaves field
/// accesses alone.
fn bool_globals(doc: &IsfDocument) -> String {
    let mut out = String::new();
    for input in &doc.inputs {
        if input.type_ == "bool" || input.type_ == "event" {
            out.push_str(&format!("bool {};\n", sanitised_input_name(&input.name)));
        }
    }
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

/// The bridge between naga's entry point and the shader's `main` when bool
/// inputs or hoisted global initialisers need running first.
fn entry_wrapper(doc: &IsfDocument, initialises_globals: bool) -> String {
    let bools: Vec<String> = doc
        .inputs
        .iter()
        .filter(|input| input.type_ == "bool" || input.type_ == "event")
        .map(|input| sanitised_input_name(&input.name))
        .collect();
    if bools.is_empty() && !initialises_globals {
        return String::new();
    }
    let mut out = String::new();
    if !bools.is_empty() {
        out.push_str("\nvoid isf_init_inputs() {\n");
        for name in &bools {
            out.push_str(&format!("  {name} = (isf_bool_{name} != 0);\n"));
        }
        out.push_str("}\n");
    }
    out.push_str("void main() {\n");
    if !bools.is_empty() {
        out.push_str("  isf_init_inputs();\n");
    }
    if initialises_globals {
        out.push_str("  isf_init_globals();\n");
    }
    out.push_str("  isf_user_main();\n}\n");
    out
}

/// The function holding the assignments moved out of global declarations.
///
/// WGSL has no module-scope initialiser that can reference a uniform, so a
/// GLSL global such as `float d = min(progress, 1.0);` becomes a private
/// (zero-initialised) global plus an assignment executed at the top of the
/// entry point, before the shader's own `main`.
fn global_init_function(initialisers: &[String]) -> String {
    if initialisers.is_empty() {
        return String::new();
    }
    let mut out = String::from("\nvoid isf_init_globals() {\n");
    for statement in initialisers {
        out.push_str("  ");
        out.push_str(statement);
        out.push('\n');
    }
    out.push_str("}\n");
    out
}

/// Rename the shader's `main` so the generated wrapper can be the entry
/// point, keeping `void main()` at the end of the source for naga.
fn rename_main(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut rest = source;
    while let Some(position) = rest.find("main") {
        let (before, after_start) = rest.split_at(position);
        out.push_str(before);
        let after = &after_start[4..];
        let before_ok = before
            .chars()
            .next_back()
            .is_none_or(|character| !is_ident_char(character));
        let after_ok = after.chars().find(|character| !character.is_whitespace()) == Some('(');
        if before_ok && after_ok {
            out.push_str("isf_user_main");
        } else {
            out.push_str("main");
        }
        rest = after;
    }
    out.push_str(rest);
    out
}

/// Build the desktop-450 vertex GLSL for the (optional) `.vs` sidecar.
///
/// Port of `ISFParser.buildVertexShader` / `vertexShaderSkeleton`
/// (`ISFParser.js:196-330`), with `attribute` → `in`, `varying` → `out`,
/// `precision` dropped and the skeleton's own `isf_vertShaderInit` kept.
/// The `[[functions]]` block that fills `_<image>_texCoord` is not emitted:
/// the translated fragment never declares those inputs (the runtime uses the
/// built-in WGSL vertex triangle), so generating them would only add dead
/// varyings to the report.
pub fn build_vertex_glsl(doc: &IsfDocument) -> String {
    let body = rewrite_vertex_body(doc);
    let members = uniform_members(doc);
    let bindings = texture_bindings(doc);

    let mut out = String::with_capacity(body.source.len() + 4096);
    out.push_str("#version 450\n\n");
    if !declares_identifier(doc.vertex_source.as_deref().unwrap_or(""), "PI") {
        out.push_str("#define PI 3.14159265358979323846\n");
    }
    out.push_str("layout(location = 0) in vec2 isf_position;\n");
    out.push_str("layout(location = 0) out vec2 isf_FragNormCoord;\n\n");

    out.push_str("layout(binding = 0) uniform IsfUniforms {\n");
    for member in &members {
        out.push_str("  ");
        out.push_str(member.kind.glsl_type());
        out.push(' ');
        out.push_str(&member.name);
        out.push_str(";\n");
    }
    out.push_str("};\n\n");

    out.push_str(&bool_globals(doc));
    out.push_str(&texture_declarations(&bindings));
    out.push_str(VVSAMPLER_MACROS);
    out.push('\n');

    out.push_str("void isf_vertShaderInit() {\n");
    out.push_str("  gl_Position = vec4(isf_position, 0.0, 1.0);\n");
    out.push_str(
        "  isf_FragNormCoord = vec2((gl_Position.x + 1.0) / 2.0, (gl_Position.y + 1.0) / 2.0);\n",
    );
    out.push_str("}\n\n");
    out.push_str(&assign_output_locations(&body.source, 1));
    out.push_str(&global_init_function(&body.global_initialisers));
    out.push_str(&entry_wrapper(doc, !body.global_initialisers.is_empty()));
    out
}

/// Give the vertex stage's own `out` declarations explicit locations after
/// the skeleton's `isf_FragNormCoord` (location 0).
///
/// naga's implicit location assignment collides when the skeleton output and
/// a sidecar `out vec2 texOffsets[5]` are both implicit
/// (`BindingCollision { location: 0 }`). Arrays consume one location per
/// element, so the counter advances by the array length.
fn assign_output_locations(body: &str, mut next_location: u32) -> String {
    let mut out = String::with_capacity(body.len());
    for line in body.split_inclusive('\n') {
        let trimmed = line.trim_start();
        let indent_len = line.len() - trimmed.len();
        let Some(rest) = strip_keyword(trimmed, "out") else {
            out.push_str(line);
            continue;
        };
        if !rest.trim_end().ends_with(';') {
            out.push_str(line);
            continue;
        }
        let consumed = count_declarators(rest);
        if consumed == 0 {
            out.push_str(line);
            continue;
        }
        out.push_str(&format!(
            "{}layout(location = {}) out {}",
            &line[..indent_len],
            next_location,
            rest
        ));
        next_location = next_location.saturating_add(consumed);
    }
    out
}

/// Number of vertex-output locations a declaration list consumes.
///
/// WGSL gives a matrix one location per column (`mat2` 2, `mat3` 3, `mat4` 4,
/// `matCxR` C) and an array one location per element, so `mat2 m[3]` consumes
/// six. Sidecar `out` declarations are numbered from this count; before the
/// matrix case was handled every matrix was charged a single location.
fn count_declarators(rest: &str) -> u32 {
    let declaration = rest.trim().trim_end_matches(';');
    let Some((type_name, declarators)) = declaration.split_once(char::is_whitespace) else {
        return 0;
    };
    let per_declarator = matrix_columns(type_name);
    let mut total = 0;
    for declarator in split_args(declarators) {
        let declarator = declarator.trim();
        if declarator.is_empty() {
            continue;
        }
        let mut count = per_declarator;
        if let Some(open) = declarator.find('[')
            && let Some(close) = declarator[open..].find(']')
            && let Ok(size) = declarator[open + 1..open + close].trim().parse::<u32>()
        {
            count = per_declarator.saturating_mul(size.max(1));
        }
        total += count;
    }
    total
}

/// The number of interface locations a type occupies (`mat3` → 3, anything
/// else → 1).
fn matrix_columns(type_name: &str) -> u32 {
    let name = type_name.strip_prefix('d').unwrap_or(type_name);
    let Some(rest) = name.strip_prefix("mat") else {
        return 1;
    };
    rest.split('x')
        .next()
        .and_then(|columns| columns.parse::<u32>().ok())
        .unwrap_or(1)
        .clamp(1, 4)
}

/// A fully rewritten shader body plus the statements that must run before the
/// shader's own `main` (see [`hoist_global_initialisers`]).
struct RewrittenBody {
    /// The rewritten body.
    source: String,
    /// Hoisted global assignments, in declaration order.
    global_initialisers: Vec<String>,
}

/// Full fragment body rewrite: macro expansion, ES-isms, sampler call-site
/// substitution and qualifier stripping.
fn rewrite_fragment_body(doc: &IsfDocument) -> RewrittenBody {
    let mut body = rewrite_fragment_source(&doc.fragment_source, &texture_inputs(doc));
    for (original, sanitised) in body_input_renames(doc) {
        body = replace_ident(&body, &original, &sanitised);
    }
    let (mut source, global_initialisers) = hoist_global_initialisers(&body);
    if has_bool_inputs(doc) || !global_initialisers.is_empty() {
        source = rename_main(&source);
    }
    RewrittenBody {
        source,
        global_initialisers,
    }
}

/// The body rewrite without a document, for tests.
pub fn rewrite_fragment_source(source: &str, textures: &[String]) -> String {
    let expanded = expand_img_macros(source);
    let rewritten = rewrite_common(&expanded);
    let rewritten = substitute_sampler_arguments(&rewritten, textures);
    let rewritten = inline_sampler_functions(&rewritten, textures);
    let rewritten = replace_ident(&rewritten, "gl_FragColor", "isf_FragColor");
    let rewritten = split_matrix_varyings(&rewritten, &["in", "varying"]);
    let rewritten = append_default_returns(&rewritten);
    strip_qualifiers(&rewritten, &["in", "varying", "uniform"])
}

/// Vertex body rewrite: qualifiers → desktop 450 spellings.
fn rewrite_vertex_body(doc: &IsfDocument) -> RewrittenBody {
    let mut body = rewrite_vertex_source(
        doc.vertex_source.as_deref().unwrap_or(DEFAULT_VERTEX_BODY),
        &texture_inputs(doc),
    );
    for (original, sanitised) in body_input_renames(doc) {
        body = replace_ident(&body, &original, &sanitised);
    }
    let (mut source, global_initialisers) = hoist_global_initialisers(&body);
    if has_bool_inputs(doc) || !global_initialisers.is_empty() {
        source = rename_main(&source);
    }
    RewrittenBody {
        source,
        global_initialisers,
    }
}

/// The vertex body rewrite without a document.
pub fn rewrite_vertex_source(source: &str, textures: &[String]) -> String {
    let rewritten = rewrite_common(source);
    let rewritten = substitute_sampler_arguments(&rewritten, textures);
    let rewritten = inline_sampler_functions(&rewritten, textures);
    let rewritten = replace_ident(&rewritten, "attribute", "in");
    let rewritten = replace_ident(&rewritten, "varying", "out");
    let rewritten = strip_qualifiers(&rewritten, &["uniform"]);
    let rewritten = split_matrix_varyings(&rewritten, &["out"]);
    append_default_returns(&rewritten)
}

fn has_bool_inputs(doc: &IsfDocument) -> bool {
    doc.inputs
        .iter()
        .any(|input| input.type_ == "bool" || input.type_ == "event")
}

/// Shared rewrite: drop `#version`/`precision`, rename the ES texture
/// functions and `gl_FragColor`.
fn rewrite_common(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    for line in source.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with("#version") || trimmed.starts_with("precision ") {
            continue;
        }
        out.push_str(line);
    }
    let out = replace_ident(&out, "texture2DProj", "textureProj");
    let out = replace_ident(&out, "texture2DLod", "textureLod");
    let out = replace_ident(&out, "texture2DGrad", "textureGrad");
    // `texture2DRect`/`sampler2DRect` are deliberately *not* mapped: they take
    // pixel coordinates, so sampling them with normalised `texture()` would be
    // silently wrong. `translate::translate_fragment` rejects those shaders
    // with `TranslateError::Unsupported` instead.
    let out = replace_ident(&out, "texture2D", "texture");
    replace_ident(&out, "textureCube", "texture")
}

/// Remove the leading storage qualifier from a global declaration line.
///
/// `in vec2 x;` → `vec2 x;`, `varying vec4 y;` → `vec4 y;`. naga lowers
/// qualifier-less globals to zero-initialised private variables, which is the
/// behaviour the GLSL compiler gives the vendored corpus' template-leftover
/// varyings. Lines that are not declarations are left untouched.
fn strip_qualifiers(source: &str, qualifiers: &[&str]) -> String {
    let mut out = String::with_capacity(source.len());
    for line in source.split_inclusive('\n') {
        let trimmed = line.trim_start();
        let indent_len = line.len() - trimmed.len();
        let mut replaced = None;
        for qualifier in qualifiers {
            if let Some(rest) = strip_keyword(trimmed, qualifier)
                && rest.trim_end().ends_with(';')
            {
                replaced = Some(format!("{}{}", &line[..indent_len], rest));
                break;
            }
        }
        match replaced {
            Some(line) => out.push_str(&line),
            None => out.push_str(line),
        }
    }
    out
}

/// Strip `keyword` when it is followed by whitespace, returning the rest of
/// the line. `inout` never matches `in`.
fn strip_keyword<'a>(line: &'a str, keyword: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(keyword)?;
    if rest.starts_with(|c: char| c.is_whitespace()) {
        Some(rest.trim_start())
    } else {
        None
    }
}

/// Identifier-aware replacement. `replace_ident(s, "texture2D", "texture")`
/// leaves `my_texture2DProj` alone.
pub fn replace_ident(source: &str, from: &str, to: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut rest = source;
    while let Some(position) = rest.find(from) {
        let (before, after_start) = rest.split_at(position);
        out.push_str(before);
        let after = &after_start[from.len()..];
        let before_ok = before.chars().next_back().is_none_or(|c| !is_ident_char(c));
        let after_ok = after.chars().next().is_none_or(|c| !is_ident_char(c));
        if before_ok && after_ok {
            out.push_str(to);
        } else {
            out.push_str(from);
        }
        rest = after;
    }
    out.push_str(rest);
    out
}

/// Whether `name` appears in `source` as a whole identifier.
///
/// `contains_ident("my_texture2DRect", "texture2DRect")` is `false`.
pub fn contains_ident(source: &str, name: &str) -> bool {
    let mut rest = source;
    while let Some(position) = rest.find(name) {
        let (before, after_start) = rest.split_at(position);
        let after = &after_start[name.len()..];
        let before_ok = before.chars().next_back().is_none_or(|c| !is_ident_char(c));
        let after_ok = after.chars().next().is_none_or(|c| !is_ident_char(c));
        if before_ok && after_ok {
            return true;
        }
        rest = after;
    }
    false
}

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// `true` when `source` has a `#define <name>` (or `#define <name>(…)`).
pub fn defines_macro(source: &str, name: &str) -> bool {
    for line in source.lines() {
        let trimmed = line.trim_start();
        let Some(rest) = trimmed.strip_prefix("#define") else {
            continue;
        };
        let rest = rest.trim_start();
        let end = rest.find(|c: char| !is_ident_char(c)).unwrap_or(rest.len());
        if &rest[..end] == name {
            return true;
        }
    }
    false
}

/// Replace sampler arguments with the split `texture2D`/`sampler` pair the
/// naga front-end expects.
///
/// naga does not accept a combined `sampler2D` uniform declaration, so the
/// preprocessor binds `isf_tex_<name>` (a `texture2D`) and `isf_smp_<name>`
/// (a `sampler`) and rewrites the *first argument* of every sampling call
/// whose argument is exactly a texture name. Working at the call site (rather
/// than with a `#define`) keeps shaders that shadow a texture name with a
/// local (`Interlace.fs`: `vec4 lastRow = IMG_PIXEL(lastRow, …);`) compiling.
pub fn substitute_sampler_arguments(source: &str, textures: &[String]) -> String {
    const CALLS: &[&str] = &[
        "VVSAMPLER_2DBYPIXEL",
        "VVSAMPLER_2DBYNORM",
        "textureProj",
        "textureLod",
        "textureGrad",
        "textureSize",
        "texture",
    ];
    let mut current = source.to_string();
    for call in CALLS {
        current = substitute_in_calls(&current, call, textures);
    }
    current
}

fn substitute_in_calls(source: &str, call: &str, textures: &[String]) -> String {
    let mut out = String::with_capacity(source.len());
    let mut rest = source;
    loop {
        let Some((start, args_start, end)) = find_call(rest, call) else {
            out.push_str(rest);
            return out;
        };
        let args_text = &rest[args_start..end];
        let first_argument = split_args(args_text).first().copied().unwrap_or("");
        let name = first_argument.trim();
        out.push_str(&rest[..start]);
        if textures.iter().any(|texture| texture == name) {
            let leading = first_argument.len() - first_argument.trim_start().len();
            out.push_str(&rest[start..args_start + leading]);
            out.push_str(&format!("sampler2D(isf_tex_{name}, isf_smp_{name})"));
            out.push_str(&rest[args_start + leading + name.len()..=end]);
        } else {
            out.push_str(&rest[start..=end]);
        }
        rest = &rest[end + 1..];
    }
}

// ---------------------------------------------------------------------------
// General source passes
// ---------------------------------------------------------------------------

/// `true` for a GLSL built-in type name (used to recognise declarations).
fn is_glsl_type(name: &str) -> bool {
    matches!(
        name,
        "float"
            | "int"
            | "uint"
            | "bool"
            | "vec2"
            | "vec3"
            | "vec4"
            | "ivec2"
            | "ivec3"
            | "ivec4"
            | "uvec2"
            | "uvec3"
            | "uvec4"
            | "bvec2"
            | "bvec3"
            | "bvec4"
            | "mat2"
            | "mat3"
            | "mat4"
            | "mat2x2"
            | "mat2x3"
            | "mat2x4"
            | "mat3x2"
            | "mat3x3"
            | "mat3x4"
            | "mat4x2"
            | "mat4x3"
            | "mat4x4"
    )
}

/// Copy of `source` with `//` and `/* … */` comments removed.
///
/// Used for analysis only (brace counting, declaration matching); the original
/// text is what gets emitted.
fn strip_comments(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut out = String::with_capacity(source.len());
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'/') {
            index = match source[index..].find('\n') {
                Some(offset) => index + offset,
                None => bytes.len(),
            };
            continue;
        }
        if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'*') {
            index = match source[index..].find("*/") {
                Some(offset) => index + offset + 2,
                None => bytes.len(),
            };
            continue;
        }
        let character = source[index..].chars().next().expect("index is in bounds");
        out.push(character);
        index += character.len_utf8();
    }
    out
}

/// Net brace nesting change of comment-free source.
fn brace_delta(source: &str) -> i32 {
    source.chars().fold(0, |delta, character| match character {
        '{' => delta + 1,
        '}' => delta - 1,
        _ => delta,
    })
}

/// A file-scope function definition found by [`function_definitions`].
#[derive(Clone, Debug)]
struct FunctionDefinition {
    /// Byte offset of the return type.
    start: usize,
    /// Byte offset of the function name.
    name_start: usize,
    /// Function name.
    name: String,
    /// Return type as spelled (`void`, `float`, `vec4`, …).
    return_type: String,
    /// Parameter list.
    params: Vec<FunctionParameter>,
    /// Byte offset of the opening `{`.
    body_start: usize,
    /// Byte offset just past the closing `}`.
    end: usize,
}

/// One parameter of a [`FunctionDefinition`].
#[derive(Clone, Debug)]
struct FunctionParameter {
    /// The parameter as written (`sampler2D tex`).
    text: String,
    /// The parameter's type (`sampler2D`).
    type_name: String,
    /// The parameter's name (`tex`).
    name: String,
}

impl FunctionParameter {
    fn is_sampler(&self) -> bool {
        self.type_name == "sampler2D"
    }
}

/// The identifier that ends at `end`, skipping whitespace backwards.
fn identifier_before(source: &str, end: usize) -> Option<(usize, String)> {
    let bytes = source.as_bytes();
    let mut index = end.min(bytes.len());
    while index > 0 && bytes[index - 1].is_ascii_whitespace() {
        index -= 1;
    }
    let ident_end = index;
    while index > 0 && is_ident_char(bytes[index - 1] as char) {
        index -= 1;
    }
    if index == ident_end {
        return None;
    }
    Some((index, source[index..ident_end].to_string()))
}

/// Parse the `type name(params)` header that ends at the `{` at `brace`.
fn parse_function_header(source: &str, brace: usize) -> Option<FunctionDefinition> {
    let before = source[..brace].trim_end();
    let close = before.len().checked_sub(1)?;
    if !before.ends_with(')') {
        return None;
    }
    let mut nesting = 0usize;
    let mut open = None;
    for (index, character) in source[..close].char_indices().rev() {
        match character {
            ')' => nesting += 1,
            '(' => {
                if nesting == 0 {
                    open = Some(index);
                    break;
                }
                nesting -= 1;
            }
            _ => {}
        }
    }
    let open = open?;
    let (name_start, name) = identifier_before(source, open)?;
    let (start, return_type) = identifier_before(source, name_start)?;
    if return_type != "void" && !is_glsl_type(&return_type) {
        return None;
    }
    // A `{` preceded by an expression (`= foo(…) {`) is not a definition.
    if matches!(
        source[..start].trim_end().chars().next_back(),
        Some('=') | Some(',') | Some('.') | Some(')')
    ) {
        return None;
    }
    Some(FunctionDefinition {
        start,
        name_start,
        name,
        return_type,
        params: parse_parameters(&source[open + 1..close]),
        body_start: brace,
        end: brace,
    })
}

fn parse_parameters(text: &str) -> Vec<FunctionParameter> {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed == "void" {
        return Vec::new();
    }
    split_args(text)
        .into_iter()
        .filter_map(|parameter| {
            let parameter = parameter.trim();
            if parameter.is_empty() {
                return None;
            }
            let mut type_name = None;
            let mut name = None;
            for word in parameter.split_whitespace() {
                if matches!(word, "in" | "out" | "inout" | "const") {
                    continue;
                }
                if type_name.is_none() {
                    type_name = Some(word.to_string());
                } else if name.is_none() {
                    let end = word
                        .find(|character: char| !is_ident_char(character))
                        .unwrap_or(word.len());
                    name = Some(word[..end].to_string());
                }
            }
            Some(FunctionParameter {
                text: parameter.to_string(),
                type_name: type_name?,
                name: name?,
            })
        })
        .collect()
}

/// Offset just past the `}` matching the `{` at `open`, skipping comments.
fn matching_brace(source: &str, open: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut depth = 0usize;
    let mut index = open;
    while index < bytes.len() {
        match bytes[index] {
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                index = match source[index..].find('\n') {
                    Some(offset) => index + offset + 1,
                    None => bytes.len(),
                };
                continue;
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index = match source[index..].find("*/") {
                    Some(offset) => index + offset + 2,
                    None => bytes.len(),
                };
                continue;
            }
            b'{' => depth += 1,
            b'}' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(index + 1);
                }
            }
            _ => {}
        }
        index += 1;
    }
    None
}

/// Every file-scope function definition in `source`, in source order.
fn function_definitions(source: &str) -> Vec<FunctionDefinition> {
    let bytes = source.as_bytes();
    let mut definitions = Vec::new();
    let mut index = 0usize;
    let mut depth = 0usize;
    let mut at_line_start = true;
    while index < bytes.len() {
        if at_line_start {
            let mut probe = index;
            while probe < bytes.len() && (bytes[probe] == b' ' || bytes[probe] == b'\t') {
                probe += 1;
            }
            if probe < bytes.len() && bytes[probe] == b'#' {
                index = match source[probe..].find('\n') {
                    Some(offset) => probe + offset + 1,
                    None => bytes.len(),
                };
                continue;
            }
            at_line_start = false;
        }
        match bytes[index] {
            b'/' if bytes.get(index + 1) == Some(&b'/') => {
                index = match source[index..].find('\n') {
                    Some(offset) => index + offset + 1,
                    None => bytes.len(),
                };
                at_line_start = true;
                continue;
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index = match source[index..].find("*/") {
                    Some(offset) => index + offset + 2,
                    None => bytes.len(),
                };
                continue;
            }
            b'\n' => at_line_start = true,
            b'{' if depth == 0 => {
                if let Some(mut definition) = parse_function_header(source, index) {
                    let Some(end) = matching_brace(source, index) else {
                        break;
                    };
                    definition.end = end;
                    definitions.push(definition);
                    index = end;
                    at_line_start = false;
                    continue;
                }
                depth = 1;
            }
            b'{' => depth += 1,
            b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
        index += 1;
    }
    definitions
}

/// Whether the `name(...)` at `start` is a function prototype (`vec4 f(…)`)
/// rather than a call: the token before it is a type name.
fn is_function_prototype(source: &str, start: usize) -> bool {
    identifier_before(source, start).is_some_and(|(_, word)| is_glsl_type(&word))
}

/// Resolve a sampler argument to the texture it samples.
///
/// Accepts a bare texture name (`startImage`, the source spelling) and the
/// already-split `sampler2D(isf_tex_startImage, isf_smp_startImage)` form a
/// previous inline pass produced.
fn sampler_argument_texture(argument: &str, textures: &[String]) -> Option<String> {
    let argument = argument.trim();
    if let Some(texture) = textures.iter().find(|texture| texture.as_str() == argument) {
        return Some(texture.clone());
    }
    let inner = argument.strip_prefix("sampler2D(")?.strip_suffix(')')?;
    let (texture, _) = inner.split_once(',')?;
    let texture = texture.trim().strip_prefix("isf_tex_")?;
    textures
        .iter()
        .find(|candidate| candidate.as_str() == texture)
        .cloned()
}

/// All `name(...)` call sites in `source`, in source order.
fn find_calls(source: &str, name: &str) -> Vec<(usize, usize, usize)> {
    let mut calls = Vec::new();
    let mut search = 0usize;
    while let Some(relative) = source[search..].find(name) {
        let start = search + relative;
        let after = start + name.len();
        let before_ok = source[..start]
            .chars()
            .next_back()
            .is_none_or(|character| !is_ident_char(character));
        if before_ok && source[after..].starts_with('(') {
            let args_start = after + 1;
            if let Some(close) = find_close(&source[args_start..]) {
                calls.push((start, args_start, args_start + close));
                search = args_start + close + 1;
                continue;
            }
        }
        search = after;
    }
    calls
}

/// Inline functions that take a `sampler2D` parameter.
///
/// naga's GLSL front-end accepts `sampler2D(isf_tex_x, isf_smp_x)` as a
/// *sampling call argument* but cannot represent a sampler as a function
/// parameter. Every call site of such a function is rewritten to a clone with
/// the parameter removed and the actual texture's `texture`/`sampler` pair
/// substituted into the body; a sampler function with no call sites (dead
/// template code, as in `Point10 Banded Swirl`) is deleted. Functions that
/// cannot be resolved statically (recursion, a call before the definition, an
/// overloaded name, or an argument that is not a texture) are left untouched
/// rather than guessed at.
fn inline_sampler_functions(source: &str, textures: &[String]) -> String {
    let mut current = source.to_string();
    loop {
        let definitions = function_definitions(&current);
        let candidates: Vec<FunctionDefinition> = definitions
            .into_iter()
            .filter(|definition| definition.params.iter().any(FunctionParameter::is_sampler))
            .collect();
        if candidates.is_empty() {
            return current;
        }
        // Callees are defined before their callers, so trying the *last*
        // sampler function first resolves a chain of sampler functions one
        // level at a time (the outer call site still has a real texture).
        let mut rewritten = None;
        for definition in candidates.iter().rev() {
            if let Some(next) = inline_one_sampler_function(&current, definition, textures)
                && next != current
            {
                rewritten = Some(next);
                break;
            }
        }
        match rewritten {
            Some(next) => current = next,
            None => return current,
        }
    }
}

/// Rewrite every call of one sampler-taking `definition`.
///
/// Returns `None` when a call cannot be resolved (an unknown sampler argument,
/// recursion or a call before the definition), so the caller can try another
/// function instead of leaving a half-rewritten shader.
fn inline_one_sampler_function(
    source: &str,
    definition: &FunctionDefinition,
    textures: &[String],
) -> Option<String> {
    let sampler_positions: Vec<usize> = definition
        .params
        .iter()
        .enumerate()
        .filter(|(_, parameter)| parameter.is_sampler())
        .map(|(index, _)| index)
        .collect();
    let calls: Vec<(usize, usize, usize)> = find_calls(source, &definition.name)
        .into_iter()
        .filter(|(start, _, _)| *start != definition.name_start)
        .filter(|(start, _, _)| !is_function_prototype(source, *start))
        .collect();
    // Recursion, or a call before the definition (which would need a forward
    // declaration), cannot be inlined safely.
    if calls.iter().any(|(start, _, _)| {
        (*start >= definition.start && *start < definition.end) || *start < definition.start
    }) {
        return None;
    }

    let mut resolved = Vec::with_capacity(calls.len());
    for (start, args_start, close) in &calls {
        let args = split_args(&source[*args_start..*close]);
        let mut sampler_names = Vec::with_capacity(sampler_positions.len());
        for position in &sampler_positions {
            let argument = args.get(*position)?.trim();
            sampler_names.push(sampler_argument_texture(argument, textures)?);
        }
        let mut remaining = Vec::with_capacity(args.len() - sampler_positions.len());
        for (index, argument) in args.iter().enumerate() {
            if !sampler_positions.contains(&index) {
                remaining.push(argument.trim().to_string());
            }
        }
        resolved.push((*start, *close + 1, sampler_names, remaining));
    }

    // One clone per distinct sampler tuple.
    let mut clones: Vec<(String, String, String)> = Vec::new();
    let mut replacements = Vec::with_capacity(resolved.len());
    for (start, end, sampler_names, remaining) in resolved {
        let key = sampler_names.join("|");
        let clone_name = match clones.iter().find(|(existing, _, _)| *existing == key) {
            Some((_, name, _)) => name.clone(),
            None => {
                let suffix = sampler_names
                    .iter()
                    .map(|texture| {
                        texture
                            .chars()
                            .map(|character| {
                                if is_ident_char(character) {
                                    character
                                } else {
                                    '_'
                                }
                            })
                            .collect::<String>()
                    })
                    .collect::<Vec<_>>()
                    .join("_");
                let name = format!("{}_isf_{suffix}", definition.name);
                let mut body = source[definition.body_start..definition.end].to_string();
                for (parameter, texture) in definition
                    .params
                    .iter()
                    .filter(|parameter| parameter.is_sampler())
                    .zip(sampler_names.iter())
                {
                    body = replace_ident(
                        &body,
                        &parameter.name,
                        &format!("sampler2D(isf_tex_{texture}, isf_smp_{texture})"),
                    );
                }
                let parameters: Vec<String> = definition
                    .params
                    .iter()
                    .filter(|parameter| !parameter.is_sampler())
                    .map(|parameter| parameter.text.clone())
                    .collect();
                let text = format!(
                    "{} {name}({}) {body}",
                    definition.return_type,
                    parameters.join(", ")
                );
                clones.push((key, name.clone(), text));
                name
            }
        };
        replacements.push((
            start,
            end,
            format!("{clone_name}({})", remaining.join(", ")),
        ));
    }

    let clone_source = clones
        .iter()
        .map(|(_, _, text)| text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let mut edits = vec![(definition.start, definition.end, clone_source)];
    edits.extend(replacements);
    Some(apply_edits(source, edits))
}

/// Apply non-overlapping `(start, end, replacement)` edits to `source`.
fn apply_edits(source: &str, mut edits: Vec<(usize, usize, String)>) -> String {
    edits.sort_by_key(|(start, _, _)| *start);
    let mut out = String::with_capacity(source.len());
    let mut cursor = 0usize;
    for (start, end, replacement) in edits {
        if start < cursor || end > source.len() || start > end {
            continue;
        }
        out.push_str(&source[cursor..start]);
        out.push_str(&replacement);
        cursor = end;
    }
    out.push_str(&source[cursor..]);
    out
}

/// Append `return <zero>;` to non-void functions whose last top-level
/// statement is not a return.
///
/// WGSL (and naga's validator) requires a value on every path; GLSL is the
/// same but its compilers accept the fall-through with a warning, which is what
/// `Shape Morph`'s `shapeForType` relies on. The zero value is deterministic
/// and comes from the declared type; functions whose type has no obvious zero
/// (a struct, `sampler2D`) are left alone.
fn append_default_returns(source: &str) -> String {
    let mut insertions: Vec<(usize, String)> = Vec::new();
    for definition in function_definitions(source) {
        if definition.return_type == "void" {
            continue;
        }
        let Some(zero) = zero_value(&definition.return_type) else {
            continue;
        };
        let body = &source[definition.body_start + 1..definition.end - 1];
        if ends_with_return(body) {
            continue;
        }
        insertions.push((definition.end - 1, format!("\n  return {zero};\n")));
    }
    let mut out = source.to_string();
    for (position, text) in insertions.into_iter().rev() {
        out.insert_str(position, &text);
    }
    out
}

/// The WGSL/GLSL zero spelling for a return type.
fn zero_value(type_name: &str) -> Option<String> {
    match type_name {
        "float" => Some("0.0".to_string()),
        "int" => Some("0".to_string()),
        "uint" => Some("0u".to_string()),
        "bool" => Some("false".to_string()),
        "mat2" | "mat3" | "mat4" => Some(format!("{type_name}(0.0)")),
        _ => {
            for (prefix, zero) in [
                ("vec", "0.0"),
                ("ivec", "0"),
                ("uvec", "0u"),
                ("bvec", "false"),
            ] {
                if let Some(size) = type_name.strip_prefix(prefix)
                    && matches!(size, "2" | "3" | "4")
                {
                    return Some(format!("{type_name}({zero})"));
                }
            }
            None
        }
    }
}

/// Whether the last top-level statement of a function body is a `return`.
fn ends_with_return(body: &str) -> bool {
    let mut depth = 0i32;
    let mut statement_start = 0usize;
    let mut last_statement: Option<(usize, usize)> = None;
    for (index, character) in body.char_indices() {
        match character {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    last_statement = Some((statement_start, index + 1));
                    statement_start = index + 1;
                }
            }
            ';' if depth == 0 => {
                last_statement = Some((statement_start, index + 1));
                statement_start = index + 1;
            }
            _ => {}
        }
    }
    let Some((start, end)) = last_statement else {
        return false;
    };
    body[start..end]
        .trim_start()
        .strip_prefix("return")
        .is_some_and(|rest| {
            rest.starts_with(|character: char| {
                character.is_whitespace() || character == ';' || character == '('
            })
        })
}

/// Split `in`/`out`/`varying mat2|mat3|mat4` declarations into per-column
/// vector varyings.
///
/// WGSL entry-point IO cannot carry a matrix: naga rejects `out mat2 rotmat`
/// (`v002 Light Leak`'s vertex sidecar). Each column becomes its own varying
/// (which consumes the same number of locations, see [`count_declarators`]);
/// matrix assignments write the columns and matrix reads rebuild the value.
fn split_matrix_varyings(source: &str, qualifiers: &[&str]) -> String {
    let mut matrices: Vec<(String, usize)> = Vec::new();
    let mut out = String::with_capacity(source.len());
    let mut depth = 0i32;
    for line in source.split_inclusive('\n') {
        let code = strip_comments(line);
        if depth == 0
            && let Some((qualifier, columns, name)) = parse_matrix_varying(code.trim(), qualifiers)
        {
            let indent = &line[..line.len() - line.trim_start().len()];
            let column_type = match columns {
                2 => "vec2",
                3 => "vec3",
                _ => "vec4",
            };
            for column in 0..columns {
                out.push_str(indent);
                out.push_str(qualifier);
                out.push(' ');
                out.push_str(column_type);
                out.push(' ');
                out.push_str(&format!("{name}_{column};\n"));
            }
            matrices.push((name, columns));
            continue;
        }
        out.push_str(line);
        depth += brace_delta(&code);
    }
    let mut result = out;
    for (name, columns) in &matrices {
        let Some(rewritten) = rewrite_matrix_assignments(&result, name, *columns) else {
            // An assignment not at statement level cannot be split without
            // rewriting arbitrary expressions; leave the shader as it was.
            return source.to_string();
        };
        result = rewritten;
        let reconstructed = format!(
            "mat{columns}({})",
            (0..*columns)
                .map(|column| format!("{name}_{column}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        result = replace_ident(&result, name, &reconstructed);
    }
    result
}

fn parse_matrix_varying<'a>(
    declaration: &str,
    qualifiers: &[&'a str],
) -> Option<(&'a str, usize, String)> {
    for qualifier in qualifiers {
        let Some(rest) = strip_keyword(declaration, qualifier) else {
            continue;
        };
        let Some(declaration) = rest.trim_end().strip_suffix(';') else {
            continue;
        };
        let mut words = declaration.split_whitespace();
        let type_name = words.next()?;
        let name = words.next()?;
        if words.next().is_some() {
            continue;
        }
        let columns = match type_name {
            "mat2" => 2,
            "mat3" => 3,
            "mat4" => 4,
            _ => continue,
        };
        if name.is_empty() || !name.chars().all(is_ident_char) {
            continue;
        }
        return Some((qualifier, columns, name.to_string()));
    }
    None
}

/// Rewrite statement-level `m = <expression>;` assignments of a split matrix
/// varying into per-column writes, keeping the expression evaluated once.
///
/// Returns `None` when an assignment is not a statement of its own (for
/// example inside a `for` header), where the split cannot be proved correct.
fn rewrite_matrix_assignments(source: &str, name: &str, columns: usize) -> Option<String> {
    let reconstructed = format!(
        "mat{columns}({})",
        (0..columns)
            .map(|column| format!("{name}_{column}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let mut out = String::with_capacity(source.len());
    let mut rest = source;
    let mut temporaries = 0usize;
    loop {
        let Some(position) = find_identifier(rest, name) else {
            out.push_str(rest);
            return Some(out);
        };
        let after_name = position + name.len();
        let after = &rest[after_name..];
        let spaces = after.len() - after.trim_start().len();
        let candidate = &after[spaces..];
        if candidate.starts_with('=') && !candidate.starts_with("==") {
            let prefix = strip_comments(&rest[..position]);
            let prefix = prefix.trim_end();
            if !(prefix.is_empty()
                || prefix.ends_with(';')
                || prefix.ends_with('{')
                || prefix.ends_with('}'))
            {
                return None;
            }
            let equals = after_name + spaces;
            if let Some(offset) = rest[equals..].find(';') {
                let semicolon = equals + offset;
                let rhs = rest[equals + 1..semicolon].trim();
                out.push_str(&rest[..position]);
                out.push_str(&format!("{{ mat{columns} isf_mat_{temporaries} = "));
                out.push_str(&replace_ident(rhs, name, &reconstructed));
                out.push_str("; ");
                for column in 0..columns {
                    out.push_str(&format!(
                        "{name}_{column} = isf_mat_{temporaries}[{column}]; "
                    ));
                }
                out.push('}');
                temporaries += 1;
                rest = &rest[semicolon + 1..];
                continue;
            }
        }
        out.push_str(&rest[..after_name]);
        rest = &rest[after_name..];
    }
}

/// Offset of the first whole-identifier occurrence of `name`.
fn find_identifier(source: &str, name: &str) -> Option<usize> {
    let mut search = 0usize;
    while let Some(relative) = source[search..].find(name) {
        let start = search + relative;
        let after = start + name.len();
        let before_ok = source[..start]
            .chars()
            .next_back()
            .is_none_or(|character| !is_ident_char(character));
        let after_ok = source[after..]
            .chars()
            .next()
            .is_none_or(|character| !is_ident_char(character));
        if before_ok && after_ok {
            return Some(start);
        }
        search = after;
    }
    None
}

/// Split file-scope declarations whose initialiser naga cannot represent and
/// move the initialisation into a generated function.
///
/// WGSL globals are initialised with constant expressions only; naga parses a
/// GLSL initialiser such as `float d = min(progress, 1.0);` but (correctly)
/// rejects the module. The declaration keeps its name and type (a private,
/// zero-initialised global) and the assignment runs at the top of the entry
/// point, in declaration order, so a global that reads another one still sees
/// the same value. Declarations inside `#if` blocks are skipped: moving the
/// assignment out of the guard could reference a name that was never declared
/// (a module-scope initialiser inside a guard that reads a hoisted global would
/// see its zero value; no corpus shader does that).
fn hoist_global_initialisers(source: &str) -> (String, Vec<String>) {
    let mut out = String::with_capacity(source.len() + 64);
    let mut initialisers = Vec::new();
    let mut depth = 0i32;
    let mut conditionals = 0usize;
    let mut lines = source.split_inclusive('\n').peekable();
    while let Some(line) = lines.next() {
        let code = strip_comments(line);
        let trimmed = code.trim();
        if trimmed.starts_with("#if") {
            conditionals += 1;
        } else if trimmed.starts_with("#endif") {
            conditionals = conditionals.saturating_sub(1);
        }
        let candidate =
            depth == 0 && conditionals == 0 && !trimmed.is_empty() && !trimmed.starts_with('#');
        if !candidate {
            out.push_str(line);
            depth += brace_delta(&code);
            continue;
        }
        let indent = &line[..line.len() - line.trim_start().len()];
        let mut statement = String::from(line);
        loop {
            let code = strip_comments(&statement);
            if code.contains(';') || code.contains('{') {
                break;
            }
            match lines.next() {
                Some(next) => statement.push_str(next),
                None => break,
            }
        }
        let code = strip_comments(&statement);
        if !code.contains('{')
            && let Some((declaration, assignments)) = rewrite_global_declaration(code.trim())
        {
            out.push_str(indent);
            out.push_str(&declaration);
            out.push('\n');
            initialisers.extend(assignments);
            continue;
        }
        out.push_str(&statement);
        depth += brace_delta(&code);
    }
    (out, initialisers)
}

/// Rewrite one `type a = expr, b;` statement into `type a, b;` plus the
/// assignments for each initialised declarator.
///
/// Returns `None` when the statement is not a plain non-`const` declaration or
/// when it contains an array initialiser, leaving the original text intact.
fn rewrite_global_declaration(code: &str) -> Option<(String, Vec<String>)> {
    let statement = code.strip_suffix(';')?;
    if statement.contains('{') || statement.contains('}') || statement.contains(';') {
        return None;
    }
    let mut words = statement.split_whitespace();
    let type_name = words.next()?;
    if type_name == "const" || !is_glsl_type(type_name) {
        return None;
    }
    let rest = statement[type_name.len()..].trim_start();
    let mut names = Vec::new();
    let mut assignments = Vec::new();
    for declarator in split_args(rest) {
        let declarator = declarator.trim();
        if declarator.is_empty() {
            return None;
        }
        let (left, right) = match split_assignment(declarator) {
            Some((left, right)) => (left.trim(), Some(right.trim())),
            None => (declarator, None),
        };
        let (name, suffix) = split_array_suffix(left)?;
        if suffix.is_some() {
            return None;
        }
        names.push(name.to_string());
        if let Some(right) = right {
            if right.is_empty() || right.contains('{') || right.contains(';') {
                return None;
            }
            assignments.push(format!("{name} = {right};"));
        }
    }
    if assignments.is_empty() {
        return None;
    }
    Some((format!("{type_name} {};", names.join(", ")), assignments))
}

/// Split at the first top-level `=` that is not part of `==`, `<=`, `>=` or
/// `!=`.
fn split_assignment(source: &str) -> Option<(&str, &str)> {
    let bytes = source.as_bytes();
    let mut depth = 0i32;
    for (index, character) in source.char_indices() {
        match character {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            '=' if depth == 0 => {
                let before = index.checked_sub(1).map(|position| bytes[position]);
                if matches!(before, Some(b'=') | Some(b'<') | Some(b'>') | Some(b'!')) {
                    continue;
                }
                if bytes.get(index + 1) == Some(&b'=') {
                    continue;
                }
                return Some((&source[..index], &source[index + 1..]));
            }
            _ => {}
        }
    }
    None
}

/// Split a declarator into its identifier and optional `[size]` suffix.
fn split_array_suffix(declaration: &str) -> Option<(&str, Option<&str>)> {
    let declaration = declaration.trim();
    let end = declaration
        .find(|character: char| !is_ident_char(character))
        .unwrap_or(declaration.len());
    let name = &declaration[..end];
    if name.is_empty() || !name.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let suffix = declaration[end..].trim();
    if suffix.is_empty() {
        Some((name, None))
    } else if suffix.starts_with('[') && suffix.ends_with(']') {
        Some((name, Some(suffix)))
    } else {
        None
    }
}

/// `true` when `source` defines or declares `name`.
///
/// Used for `PI`: the corpus has shaders that `#define PI …` *and* shaders
/// that declare `const float PI = …` (`Bounce.fs`). Either way the
/// preprocessor must not inject its own definition, or the shader's line is
/// macro-expanded into `const float 3.14159…`.
pub fn declares_identifier(source: &str, name: &str) -> bool {
    if defines_macro(source, name) {
        return true;
    }
    const TYPE_WORDS: &[&str] = &[
        "float", "int", "uint", "bool", "vec2", "vec3", "vec4", "ivec2", "ivec3", "ivec4", "uvec2",
        "uvec3", "uvec4", "bvec2", "bvec3", "bvec4", "mat2", "mat3", "mat4",
    ];
    for line in source.lines() {
        let mut search = 0;
        while let Some(relative) = line[search..].find(name) {
            let start = search + relative;
            let end = start + name.len();
            let before_ok = line[..start]
                .chars()
                .next_back()
                .is_none_or(|character| !is_ident_char(character));
            let after_ok = line[end..]
                .chars()
                .next()
                .is_none_or(|character| !is_ident_char(character));
            if before_ok && after_ok {
                let mut previous_reversed = String::new();
                for character in line[..start].trim_end().chars().rev() {
                    if is_ident_char(character) {
                        previous_reversed.push(character);
                    } else {
                        break;
                    }
                }
                let previous: String = previous_reversed.chars().rev().collect();
                if TYPE_WORDS.contains(&previous.as_str()) {
                    return true;
                }
            }
            search = end;
        }
    }
    false
}

/// Expand `IMG_THIS_PIXEL`, `IMG_THIS_NORM_PIXEL`, `IMG_PIXEL`,
/// `IMG_NORM_PIXEL` and `IMG_SIZE`.
///
/// This is the hand-written equivalent of `ISFParser.replaceSpecialFunctions`
/// (`ISFParser.js:144-176`) and its two helper regexes: every call is scanned
/// with balanced parentheses, then split on the first top-level comma. Unlike
/// the JavaScript regexes, which rely on `.` not matching a newline, split
/// calls across lines are handled too.
pub fn expand_img_macros(source: &str) -> String {
    let mut out = expand_simple(source, "IMG_THIS_PIXEL", |arg| {
        format!("texture({}, isf_FragNormCoord)", arg.trim())
    });
    out = expand_simple(&out, "IMG_THIS_NORM_PIXEL", |arg| {
        format!("texture({}, isf_FragNormCoord)", arg.trim())
    });
    out = expand_simple(&out, "IMG_PIXEL", |arg| {
        let parts = split_args(arg);
        let sampler = parts.first().copied().unwrap_or("").trim();
        let coord = parts.get(1).copied().unwrap_or("").trim();
        format!("texture({sampler}, ({coord}) / RENDERSIZE)")
    });
    out = expand_simple(&out, "IMG_NORM_PIXEL", |arg| {
        let parts = split_args(arg);
        let sampler = parts.first().copied().unwrap_or("").trim();
        let coord = parts.get(1).copied().unwrap_or("").trim();
        format!(
            "VVSAMPLER_2DBYNORM({sampler}, _{sampler}_imgRect, _{sampler}_imgSize, _{sampler}_flip, {coord})"
        )
    });
    expand_simple(&out, "IMG_SIZE", |arg| format!("_{}_imgSize", arg.trim()))
}

/// Replace every `name(…)` call, repeating until no call matches.
fn expand_simple<F>(source: &str, name: &str, replace: F) -> String
where
    F: Fn(&str) -> String,
{
    let mut current = source.to_string();
    loop {
        let Some((start, args_start, end)) = find_call(&current, name) else {
            return current;
        };
        let mut next = String::with_capacity(current.len() + 32);
        next.push_str(&current[..start]);
        next.push_str(&replace(&current[args_start..end]));
        next.push_str(&current[end + 1..]);
        current = next;
    }
}

/// Locate `name(...)` in `source`, returning `(start, args_start, close)`.
fn find_call(source: &str, name: &str) -> Option<(usize, usize, usize)> {
    let mut search_from = 0;
    while let Some(relative) = source[search_from..].find(name) {
        let start = search_from + relative;
        let after = start + name.len();
        let before_ok = source[..start]
            .chars()
            .next_back()
            .is_none_or(|c| !is_ident_char(c));
        let following = source[after..].chars().next();
        if before_ok && following == Some('(') {
            let args_start = after + 1;
            if let Some(close) = find_close(&source[args_start..]) {
                return Some((start, args_start, args_start + close));
            }
            return None;
        }
        search_from = after;
    }
    None
}

/// Offset of the `)` that closes the parenthesis opened just before `source`.
fn find_close(source: &str) -> Option<usize> {
    let mut depth = 1usize;
    for (index, character) in source.char_indices() {
        match character {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

/// Split macro arguments on top-level commas.
fn split_args(source: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (index, character) in source.char_indices() {
        match character {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.push(&source[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    parts.push(&source[start..]);
    parts
}

/// Substitution + evaluation failure for [`evaluate_size`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SizeEvalError {
    /// The expression referenced a name with no value.
    UnknownVariable(String),
    /// The expression could not be parsed.
    Parse(String),
    /// The expression called a function the evaluator does not know.
    UnknownFunction(String),
    /// Division by zero or a non-finite result.
    NotFinite,
}

impl fmt::Display for SizeEvalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SizeEvalError::UnknownVariable(name) => write!(f, "unknown size variable ${name}"),
            SizeEvalError::Parse(message) => write!(f, "invalid size expression: {message}"),
            SizeEvalError::UnknownFunction(name) => write!(f, "unknown size function {name}()"),
            SizeEvalError::NotFinite => write!(f, "size expression is not finite"),
        }
    }
}

impl std::error::Error for SizeEvalError {}

/// Port of `ISFRenderer.evaluateSize` (`ISFRenderer.js:404-421`).
///
/// Substitutes `$WIDTH`, `$HEIGHT`, the runtime scalars the JavaScript exposes
/// as uniforms (`$TIME`, `$TIMEDELTA`, `$FRAMEINDEX`, `$PASSINDEX`) and every
/// `$<input>` value, then evaluates a deliberately small expression grammar:
///
/// ```text
/// expr    := term (('+' | '-') term)*
/// term    := unary (('*' | '/') unary)*
/// unary   := ('-' | '+') unary | primary
/// primary := number | '(' expr ')' | ident '(' args ')' | ident
/// ```
///
/// Functions: `max`, `min` (2 args), `floor`, `ceil`, `abs`, `round`
/// (1 arg). The result is truncated towards zero (WebGL's `GLsizei`
/// conversion), clamped to at least 1 pixel and at most
/// [`SizeVars::max_dimension`] so a hostile formula cannot request a gigapixel
/// texture and kill the module with a wgpu validation error.
///
/// Vector uniforms (`$RENDERSIZE`, `$DATE`, `$<texture>_imgSize`) have no
/// scalar spelling: they are [`SizeEvalError::UnknownVariable`], which the
/// runtime reports as [`super::runtime::IsfRuntimeError::InvalidPassSize`] for
/// that pass instead of rendering nothing forever.
pub fn evaluate_size(expr: &str, vars: &SizeVars) -> Result<u32, SizeEvalError> {
    let substituted = substitute_size_vars(expr, vars)?;
    let mut parser = ExprParser::new(&substituted);
    let value = parser.expression()?;
    parser.skip_whitespace();
    if !parser.at_end() {
        return Err(SizeEvalError::Parse(format!(
            "unexpected trailing input at byte {}",
            parser.position
        )));
    }
    if !value.is_finite() {
        return Err(SizeEvalError::NotFinite);
    }
    let limit = vars.max_dimension.max(1) as f64;
    Ok((value.max(0.0).min(limit) as u32).max(1))
}

/// Substitute `$NAME` tokens longest-first (a name that is a prefix of
/// another must not win — the JavaScript's insertion-order `replace` does not
/// guarantee that, which matters for e.g. `$size` vs `$sizeFactor`).
fn substitute_size_vars(expr: &str, vars: &SizeVars) -> Result<String, SizeEvalError> {
    let mut names: Vec<(&str, String)> = vec![
        ("WIDTH", format_number(vars.width)),
        ("HEIGHT", format_number(vars.height)),
        ("TIME", format_number(vars.time)),
        ("TIMEDELTA", format_number(vars.time_delta)),
        ("FRAMEINDEX", format_number(vars.frame_index)),
        ("PASSINDEX", format_number(vars.pass_index)),
    ];
    for (name, value) in &vars.inputs {
        names.push((name.as_str(), format_number(*value)));
    }
    names.sort_by_key(|(name, _)| std::cmp::Reverse(name.len()));

    let mut out = expr.to_string();
    for (name, value) in names {
        out = replace_prefixed(&out, &format!("${name}"), &value);
    }
    // Anything still starting with `$` was never supplied.
    if let Some(position) = out.find('$') {
        let name: String = out[position + 1..]
            .chars()
            .take_while(|c| is_ident_char(*c))
            .collect();
        return Err(SizeEvalError::UnknownVariable(name));
    }
    Ok(out)
}

/// Replace `$name` only when it is not followed by an identifier character.
fn replace_prefixed(source: &str, needle: &str, replacement: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut rest = source;
    while let Some(position) = rest.find(needle) {
        let (before, after_start) = rest.split_at(position);
        out.push_str(before);
        let after = &after_start[needle.len()..];
        let boundary = after.chars().next().is_none_or(|c| !is_ident_char(c));
        if boundary {
            out.push_str(replacement);
            rest = after;
        } else {
            out.push_str(needle);
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

/// JavaScript-ish number formatting (`String(1280)` → `"1280"`,
/// `String(0.9)` → `"0.9"`).
fn format_number(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1.0e15 {
        format!("{}", value as i64)
    } else {
        let mut text = format!("{value}");
        if text.contains('e') || text.contains('E') {
            text = format!("{value:.10}");
            while text.ends_with('0') {
                text.pop();
            }
            if text.ends_with('.') {
                text.push('0');
            }
        }
        text
    }
}

struct ExprParser<'a> {
    source: &'a [u8],
    position: usize,
}

impl<'a> ExprParser<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source: source.as_bytes(),
            position: 0,
        }
    }

    fn at_end(&self) -> bool {
        self.position >= self.source.len()
    }

    fn skip_whitespace(&mut self) {
        while self
            .source
            .get(self.position)
            .is_some_and(u8::is_ascii_whitespace)
        {
            self.position += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.source.get(self.position).copied()
    }

    fn expression(&mut self) -> Result<f64, SizeEvalError> {
        let mut value = self.term()?;
        loop {
            self.skip_whitespace();
            match self.peek() {
                Some(b'+') => {
                    self.position += 1;
                    value += self.term()?;
                }
                Some(b'-') => {
                    self.position += 1;
                    value -= self.term()?;
                }
                _ => return Ok(value),
            }
        }
    }

    fn term(&mut self) -> Result<f64, SizeEvalError> {
        let mut value = self.unary()?;
        loop {
            self.skip_whitespace();
            match self.peek() {
                Some(b'*') => {
                    self.position += 1;
                    value *= self.unary()?;
                }
                Some(b'/') => {
                    self.position += 1;
                    let divisor = self.unary()?;
                    if divisor == 0.0 {
                        return Err(SizeEvalError::NotFinite);
                    }
                    value /= divisor;
                }
                _ => return Ok(value),
            }
        }
    }

    fn unary(&mut self) -> Result<f64, SizeEvalError> {
        self.skip_whitespace();
        match self.peek() {
            Some(b'-') => {
                self.position += 1;
                Ok(-self.unary()?)
            }
            Some(b'+') => {
                self.position += 1;
                self.unary()
            }
            _ => self.primary(),
        }
    }

    fn primary(&mut self) -> Result<f64, SizeEvalError> {
        self.skip_whitespace();
        match self.peek() {
            Some(b'(') => {
                self.position += 1;
                let value = self.expression()?;
                self.skip_whitespace();
                if self.peek() != Some(b')') {
                    return Err(SizeEvalError::Parse("expected ')'".into()));
                }
                self.position += 1;
                Ok(value)
            }
            Some(byte) if byte.is_ascii_digit() || byte == b'.' => self.number(),
            Some(byte) if byte.is_ascii_alphabetic() || byte == b'_' => self.name_or_call(),
            other => Err(SizeEvalError::Parse(format!(
                "unexpected input at byte {} ({other:?})",
                self.position
            ))),
        }
    }

    fn number(&mut self) -> Result<f64, SizeEvalError> {
        let start = self.position;
        let mut seen_dot = false;
        let mut seen_exponent = false;
        while let Some(byte) = self.peek() {
            match byte {
                b'0'..=b'9' => self.position += 1,
                b'.' if !seen_dot && !seen_exponent => {
                    seen_dot = true;
                    self.position += 1;
                }
                b'e' | b'E' if !seen_exponent => {
                    seen_exponent = true;
                    self.position += 1;
                    if matches!(self.peek(), Some(b'+') | Some(b'-')) {
                        self.position += 1;
                    }
                }
                _ => break,
            }
        }
        let text = std::str::from_utf8(&self.source[start..self.position])
            .map_err(|error| SizeEvalError::Parse(error.to_string()))?;
        text.parse::<f64>()
            .map_err(|error| SizeEvalError::Parse(error.to_string()))
    }

    fn name_or_call(&mut self) -> Result<f64, SizeEvalError> {
        let start = self.position;
        while let Some(byte) = self.peek() {
            if byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'.' {
                self.position += 1;
            } else {
                break;
            }
        }
        let name = std::str::from_utf8(&self.source[start..self.position])
            .map_err(|error| SizeEvalError::Parse(error.to_string()))?
            .to_string();
        self.skip_whitespace();
        if self.peek() != Some(b'(') {
            return Err(SizeEvalError::UnknownVariable(name));
        }
        self.position += 1;
        let mut args = Vec::new();
        loop {
            self.skip_whitespace();
            if self.peek() == Some(b')') {
                self.position += 1;
                break;
            }
            args.push(self.expression()?);
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.position += 1,
                Some(b')') => {
                    self.position += 1;
                    break;
                }
                _ => return Err(SizeEvalError::Parse("expected ',' or ')'".into())),
            }
        }
        match (name.as_str(), args.as_slice()) {
            ("max", [a, b]) => Ok(a.max(*b)),
            ("min", [a, b]) => Ok(a.min(*b)),
            ("floor", [a]) => Ok(a.floor()),
            ("ceil", [a]) => Ok(a.ceil()),
            ("abs", [a]) => Ok(a.abs()),
            ("round", [a]) => Ok(a.round()),
            (name, _) => Err(SizeEvalError::UnknownFunction(name.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::isf::parser::parse_isf;

    fn doc(source: &str) -> IsfDocument {
        parse_isf(source, None).expect("test shader parses")
    }

    #[test]
    fn uniform_layout_matches_wgsl_rules() {
        let members = vec![
            UniformMember {
                name: "DATE".into(),
                kind: UniformKind::Vec4,
                source: UniformSource::Date,
            },
            UniformMember {
                name: "RENDERSIZE".into(),
                kind: UniformKind::Vec2,
                source: UniformSource::RenderSize,
            },
            UniformMember {
                name: "PASSINDEX".into(),
                kind: UniformKind::Int,
                source: UniformSource::PassIndex,
            },
            UniformMember {
                name: "TIME".into(),
                kind: UniformKind::Float,
                source: UniformSource::Time,
            },
            UniformMember {
                name: "tiny".into(),
                kind: UniformKind::Float,
                source: UniformSource::Input("tiny".into()),
            },
            UniformMember {
                name: "point".into(),
                kind: UniformKind::Vec2,
                source: UniformSource::Input("point".into()),
            },
            UniformMember {
                name: "colour".into(),
                kind: UniformKind::Vec4,
                source: UniformSource::Input("colour".into()),
            },
        ];
        let layout = uniform_layout(&members);
        let offsets: Vec<usize> = layout.members.iter().map(|member| member.offset).collect();
        assert_eq!(offsets, vec![0, 16, 24, 28, 32, 40, 48]);
        assert_eq!(layout.size, 64, "rounded up to a 16-byte multiple");
        assert_eq!(&layout.members[1].name, "RENDERSIZE");
        assert_eq!(layout.members[1].kind, UniformKind::Vec2);
    }

    #[test]
    fn uniform_members_cover_every_input_type() {
        let shader = r#"/*{
            "INPUTS": [
                { "NAME": "gain", "TYPE": "float" },
                { "NAME": "flip", "TYPE": "bool" },
                { "NAME": "tap", "TYPE": "event" },
                { "NAME": "mode", "TYPE": "long" },
                { "NAME": "tint", "TYPE": "color" },
                { "NAME": "centre", "TYPE": "point2D" },
                { "NAME": "inputImage", "TYPE": "image" },
                { "NAME": "fft", "TYPE": "audioFFT" }
            ],
            "PASSES": [ { "TARGET": "scratch" } ]
        }*/
        void main() {}"#;
        let members = uniform_members(&doc(shader));
        let names: Vec<&str> = members.iter().map(|member| member.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "DATE",
                "RENDERSIZE",
                "PASSINDEX",
                "FRAMEINDEX",
                "TIME",
                "TIMEDELTA",
                "gain",
                "isf_bool_flip",
                "isf_bool_tap",
                "mode",
                "tint",
                "centre",
                "_inputImage_imgSize",
                "_inputImage_imgRect",
                "_inputImage_flip",
                "_fft_imgSize",
                "_fft_imgRect",
                "_fft_flip",
                "_scratch_imgSize",
                "_scratch_imgRect",
                "_scratch_flip",
            ]
        );
        let kinds: Vec<UniformKind> = members.iter().map(|member| member.kind).collect();
        assert_eq!(kinds[6], UniformKind::Float);
        assert_eq!(kinds[7], UniformKind::Int);
        assert_eq!(kinds[10], UniformKind::Vec4);
        assert_eq!(kinds[11], UniformKind::Vec2);
    }

    #[test]
    fn texture_bindings_are_pairs_after_the_uniform_block() {
        let shader = r#"/*{ "INPUTS": [ { "NAME": "inputImage", "TYPE": "image" } ],
            "PASSES": [ { "TARGET": "a" }, { "TARGET": "b" }, {} ] }*/
        void main() {}"#;
        let bindings = texture_bindings(&doc(shader));
        assert_eq!(
            bindings
                .iter()
                .map(|binding| (
                    binding.name.as_str(),
                    binding.texture_binding,
                    binding.sampler_binding
                ))
                .collect::<Vec<_>>(),
            vec![("inputImage", 1, 2), ("a", 3, 4), ("b", 5, 6),]
        );
    }

    #[test]
    fn fragment_prelude_contains_the_runtime_contract() {
        let shader = r#"/*{
            "INPUTS": [ { "NAME": "inputImage", "TYPE": "image" },
                        { "NAME": "bright", "TYPE": "float", "DEFAULT": 0.5 } ]
        }*/
        void main() { gl_FragColor = IMG_THIS_PIXEL(inputImage) * bright; }"#;
        let glsl = build_fragment_glsl(&doc(shader), 0);
        assert!(glsl.starts_with("#version 450\n"), "{glsl}");
        assert_eq!(glsl.matches("#version").count(), 1, "only the prelude");
        assert!(glsl.contains("layout(binding = 0) uniform IsfUniforms {"));
        assert!(glsl.contains("  float TIME;"), "{glsl}");
        assert!(glsl.contains("  float TIMEDELTA;"), "{glsl}");
        assert!(glsl.contains("  int PASSINDEX;"), "{glsl}");
        assert!(glsl.contains("  int FRAMEINDEX;"), "{glsl}");
        assert!(glsl.contains("  vec4 DATE;"), "{glsl}");
        assert!(glsl.contains("  vec2 RENDERSIZE;"), "{glsl}");
        assert!(glsl.contains("  float bright;"));
        assert!(glsl.contains("in vec2 isf_FragNormCoord;"));
        assert!(glsl.contains("layout(location = 0) out vec4 isf_FragColor;"));
        assert!(glsl.contains("layout(binding = 1) uniform texture2D isf_tex_inputImage;"));
        assert!(glsl.contains("layout(binding = 2) uniform sampler isf_smp_inputImage;"));
        assert!(glsl.contains("uniform texture2D isf_tex_inputImage;"));
        assert!(glsl.contains("uniform sampler isf_smp_inputImage;"));
        assert!(glsl.contains("VVSAMPLER_2DBYPIXEL"));
        assert!(glsl.contains("VVSAMPLER_2DBYNORM"));
        assert!(glsl.contains("#define PI 3.14159265358979323846"));
        assert!(glsl.contains("#define ISF_PASS_INDEX 0"));
        assert!(glsl.contains(
            "texture(sampler2D(isf_tex_inputImage, isf_smp_inputImage), isf_FragNormCoord)"
        ));
        assert!(glsl.contains("isf_FragColor ="));
        assert!(!glsl.contains("gl_FragColor ="), "rewritten to the out var");
    }

    #[test]
    fn pass_index_reaches_the_prelude_define() {
        let glsl = build_fragment_glsl(&doc("void main() {}"), 3);
        assert!(glsl.contains("#define ISF_PASS_INDEX 3"));
    }

    #[test]
    fn pi_is_not_redefined_when_the_shader_has_its_own() {
        let shader = "#define PI 3.141592653589\nvoid main() { gl_FragColor = vec4(PI); }";
        let glsl = build_fragment_glsl(&doc(shader), 0);
        assert!(!glsl.contains("3.14159265358979323846"), "{glsl}");
        assert!(glsl.contains("#define PI 3.141592653589"), "{glsl}");
    }

    #[test]
    fn bool_inputs_use_int_members_and_a_private_global() {
        let shader = r#"/*{ "INPUTS": [ { "NAME": "restartNow", "TYPE": "event" } ] }*/
        void main() { if (restartNow) { gl_FragColor = vec4(1.0); } }"#;
        let glsl = build_fragment_glsl(&doc(shader), 0);
        assert!(glsl.contains("  int isf_bool_restartNow;"), "{glsl}");
        assert!(glsl.contains("bool restartNow;"), "{glsl}");
        assert!(!glsl.contains("#define restartNow"), "{glsl}");
        assert!(glsl.contains("void isf_init_inputs() {"), "{glsl}");
        assert!(glsl.contains("restartNow = (isf_bool_restartNow != 0);"));
        assert!(glsl.contains("isf_user_main();"), "{glsl}");
        assert!(glsl.contains("void isf_user_main()"), "{glsl}");
        assert!(glsl.contains("void main() {"), "{glsl}");
    }

    #[test]
    fn a_short_bool_input_name_does_not_rewrite_swizzles() {
        // `RGB Invert.fs` has an input named `r`; a macro definition would
        // turn every `.r` into `.bool(isf_bool_r)`.
        let shader = r#"/*{ "INPUTS": [ { "NAME": "r", "TYPE": "bool" } ] }*/
        void main() {
            vec4 pixel = vec4(0.0);
            if (r) { pixel.r = 1.0; }
            gl_FragColor = pixel;
        }"#;
        let glsl = build_fragment_glsl(&doc(shader), 0);
        assert!(glsl.contains("pixel.r = 1.0;"), "{glsl}");
        assert!(!glsl.contains(".bool("), "{glsl}");
    }

    #[test]
    fn es_rewrite_handles_version_precision_and_texture_functions() {
        let shader = r#"#version 300 es
precision highp float;
precision mediump int;
uniform sampler2D tex;
void main() {
  vec4 a = texture2D(tex, vec2(0.5));
  vec4 b = texture2DLod(tex, vec2(0.5), 0.0);
  vec4 c = texture2DProj(tex, vec3(0.5, 0.5, 1.0));
  gl_FragColor = a + b + c;
}"#;
        let glsl = build_fragment_glsl(&doc(shader), 0);
        assert!(!glsl.contains("#version 300"), "{glsl}");
        assert!(!glsl.contains("precision"), "{glsl}");
        assert!(glsl.contains("texture(tex, vec2(0.5))"));
        assert!(glsl.contains("textureLod(tex, vec2(0.5), 0.0)"));
        assert!(glsl.contains("textureProj(tex, vec3(0.5, 0.5, 1.0))"));
        assert!(glsl.contains("isf_FragColor = a + b + c;"));
    }

    #[test]
    fn img_pixel_expands_to_the_rendersize_division() {
        assert_eq!(
            expand_img_macros("IMG_PIXEL(inputImage, gl_FragCoord.xy)"),
            "texture(inputImage, (gl_FragCoord.xy) / RENDERSIZE)"
        );
        assert_eq!(
            expand_img_macros("IMG_NORM_PIXEL(inputImage, coords)"),
            "VVSAMPLER_2DBYNORM(inputImage, _inputImage_imgRect, _inputImage_imgSize, _inputImage_flip, coords)"
        );
        assert_eq!(
            expand_img_macros("IMG_THIS_PIXEL(inputImage)"),
            "texture(inputImage, isf_FragNormCoord)"
        );
        assert_eq!(
            expand_img_macros("IMG_THIS_NORM_PIXEL(inputImage)"),
            "texture(inputImage, isf_FragNormCoord)"
        );
        assert_eq!(
            expand_img_macros("vec2 s = IMG_SIZE(lastData);"),
            "vec2 s = _lastData_imgSize;"
        );
    }

    #[test]
    fn img_macros_handle_nested_parentheses_and_commas() {
        assert_eq!(
            expand_img_macros("IMG_NORM_PIXEL(img, vec2(clamp(x, 0.0, 1.0), y))"),
            "VVSAMPLER_2DBYNORM(img, _img_imgRect, _img_imgSize, _img_flip, vec2(clamp(x, 0.0, 1.0), y))"
        );
        assert_eq!(
            expand_img_macros("IMG_PIXEL(img, vec2(IMG_SIZE(img)))"),
            "texture(img, (vec2(_img_imgSize)) / RENDERSIZE)"
        );
        // IMG_PIXEL_MACRO is a different identifier and stays untouched.
        assert_eq!(
            expand_img_macros("IMG_PIXEL_MACRO(img, uv)"),
            "IMG_PIXEL_MACRO(img, uv)"
        );
    }

    #[test]
    fn image_inputs_get_a_sampler_macro() {
        let shader = r#"/*{ "INPUTS": [ { "NAME": "inputImage", "TYPE": "image" } ] }*/
        void main() { gl_FragColor = texture2D(inputImage, vec2(0.5)); }"#;
        let glsl = build_fragment_glsl(&doc(shader), 0);
        assert!(glsl.contains("uniform sampler isf_smp_inputImage;"));
        assert!(
            glsl.contains("texture(sampler2D(isf_tex_inputImage, isf_smp_inputImage), vec2(0.5))")
        );
    }

    #[test]
    fn fragment_input_declarations_become_zero_globals() {
        let shader = r#"#if __VERSION__ <= 120
varying vec2 left_coord;
varying vec2 texOffsets[5];
#else
in vec2 left_coord;
in vec2 texOffsets[5];
#endif
uniform vec2 mouse;
void main() { gl_FragColor = vec4(left_coord, mouse); }"#;
        let glsl = build_fragment_glsl(&doc(shader), 0);
        assert!(!glsl.contains("varying"), "{glsl}");
        assert!(!glsl.contains("in vec2 left_coord"), "{glsl}");
        assert!(!glsl.contains("uniform vec2 mouse"), "{glsl}");
        assert!(glsl.contains("vec2 left_coord;"), "{glsl}");
        assert!(glsl.contains("vec2 texOffsets[5];"), "{glsl}");
        // Only the prelude declares an entry-point input.
        assert_eq!(glsl.matches("in vec2 ").count(), 1, "{glsl}");
    }

    #[test]
    fn vertex_skeleton_declares_the_isf_contract() {
        let shader = r#"/*{ "INPUTS": [ { "NAME": "blurAmount", "TYPE": "float" } ] }*/
        void main() {}"#;
        let vertex = r#"#if __VERSION__ <= 120
varying vec2 texOffsets[5];
#else
out vec2 texOffsets[5];
#endif
void main(void) {
  isf_vertShaderInit();
  texOffsets[0] = isf_FragNormCoord;
}"#;
        let document = parse_isf(shader, Some(vertex)).expect("parses");
        let glsl = build_vertex_glsl(&document);
        assert!(glsl.starts_with("#version 450\n"));
        assert!(glsl.contains("layout(location = 0) in vec2 isf_position;"));
        assert!(glsl.contains("out vec2 isf_FragNormCoord;"));
        assert!(glsl.contains("void isf_vertShaderInit()"));
        assert!(glsl.contains("gl_Position = vec4(isf_position, 0.0, 1.0);"));
        assert!(glsl.contains("  float blurAmount;"), "{glsl}");
        assert!(glsl.contains("out vec2 texOffsets[5];"), "{glsl}");
        assert!(!glsl.contains("varying"), "{glsl}");
        assert!(glsl.contains("isf_vertShaderInit();"));
    }

    #[test]
    fn vertex_defaults_to_the_isf_main_when_no_sidecar_is_given() {
        let glsl = build_vertex_glsl(&doc("void main() {}"));
        assert!(glsl.contains("void main() { isf_vertShaderInit(); }"));
    }

    #[test]
    fn replace_ident_respects_identifier_boundaries() {
        assert_eq!(
            replace_ident("my_texture2D + texture2D(x)", "texture2D", "texture"),
            "my_texture2D + texture(x)"
        );
        assert_eq!(
            replace_ident("gl_FragColor_extra = gl_FragColor;", "gl_FragColor", "out"),
            "gl_FragColor_extra = out;"
        );
    }

    #[test]
    fn evaluate_soft_blur_width_formulas() {
        let vars = SizeVars {
            width: 1280.0,
            height: 720.0,
            ..SizeVars::default()
        };
        assert_eq!(
            evaluate_size("max(floor($WIDTH*0.02),1.0)", &vars).expect("evaluates"),
            25
        );
        assert_eq!(
            evaluate_size("max(floor($HEIGHT*0.02),1.0)", &vars).expect("evaluates"),
            14
        );
        assert_eq!(
            evaluate_size("max(floor($WIDTH*0.25),1.0)", &vars).expect("evaluates"),
            320
        );
        assert_eq!(
            evaluate_size("max(floor($HEIGHT*0.25),1.0)", &vars).expect("evaluates"),
            180
        );
    }

    #[test]
    fn evaluate_size_clamps_to_one_pixel() {
        let vars = SizeVars {
            width: 10.0,
            height: 10.0,
            ..SizeVars::default()
        };
        assert_eq!(
            evaluate_size("max(floor($WIDTH*0.02),1.0)", &vars).expect("evaluates"),
            1
        );
        assert_eq!(evaluate_size("0", &vars).expect("evaluates"), 1);
        assert_eq!(evaluate_size("-4", &vars).expect("evaluates"), 1);
    }

    #[test]
    fn evaluate_size_substitutes_inputs() {
        let vars = SizeVars {
            width: 640.0,
            height: 480.0,
            inputs: vec![("scale".into(), 0.5), ("offset".into(), 2.0)],
            ..SizeVars::default()
        };
        assert_eq!(
            evaluate_size("$WIDTH * $scale + $offset", &vars).expect("evaluates"),
            322
        );
    }

    #[test]
    fn evaluate_size_prefers_the_longest_variable_name() {
        let vars = SizeVars {
            width: 100.0,
            height: 100.0,
            inputs: vec![("size".into(), 2.0), ("sizeFactor".into(), 4.0)],
            ..SizeVars::default()
        };
        assert_eq!(
            evaluate_size("$sizeFactor + $size", &vars).expect("evaluates"),
            6
        );
    }

    #[test]
    fn evaluate_size_supports_every_documented_function() {
        let vars = SizeVars::default();
        assert_eq!(evaluate_size("max(3, 7)", &vars).expect("ok"), 7);
        assert_eq!(evaluate_size("min(3, 7)", &vars).expect("ok"), 3);
        assert_eq!(evaluate_size("floor(3.9)", &vars).expect("ok"), 3);
        assert_eq!(evaluate_size("ceil(3.1)", &vars).expect("ok"), 4);
        assert_eq!(evaluate_size("abs(-3.5)", &vars).expect("ok"), 3);
        assert_eq!(evaluate_size("round(3.6)", &vars).expect("ok"), 4);
        assert_eq!(evaluate_size("(2 + 3) * 4 - 6 / 3", &vars).expect("ok"), 18);
        assert_eq!(evaluate_size("-(-2)", &vars).expect("ok"), 2);
    }

    #[test]
    fn evaluate_size_reports_bad_input() {
        let vars = SizeVars::default();
        assert_eq!(
            evaluate_size("$WIDTH + 1", &vars).expect("ok"),
            1,
            "$WIDTH is 0 and the result clamps to 1"
        );
        assert_eq!(
            evaluate_size("$nope", &vars).expect_err("unknown"),
            SizeEvalError::UnknownVariable("nope".into())
        );
        assert_eq!(
            evaluate_size("pow(2, 3)", &vars).expect_err("unknown fn"),
            SizeEvalError::UnknownFunction("pow".into())
        );
        assert!(matches!(
            evaluate_size("1 / 0", &vars),
            Err(SizeEvalError::NotFinite)
        ));
        assert!(matches!(
            evaluate_size("(1 + 2", &vars),
            Err(SizeEvalError::Parse(_))
        ));
    }

    #[test]
    fn sampler_arguments_are_split_at_the_call_site() {
        let textures = vec!["lastRow".to_string(), "inputImage".to_string()];
        let source = r#"
            vec4 a = texture(lastRow, vec2(0.5));
            vec4 lastRow = texture(lastRow, vec2(0.5));
            vec4 b = textureLod(inputImage, uv, 0.0);
            vec4 c = texture(other, uv);
        "#;
        let out = substitute_sampler_arguments(source, &textures);
        assert!(
            out.contains(
                "vec4 a = texture(sampler2D(isf_tex_lastRow, isf_smp_lastRow), vec2(0.5));"
            )
        );
        assert!(out.contains(
            "vec4 lastRow = texture(sampler2D(isf_tex_lastRow, isf_smp_lastRow), vec2(0.5));"
        ));
        assert!(
            out.contains("textureLod(sampler2D(isf_tex_inputImage, isf_smp_inputImage), uv, 0.0)")
        );
        assert!(out.contains("texture(other, uv)"), "unknown names stay put");
    }

    #[test]
    fn sampler_parameters_are_cloned_per_actual_texture() {
        let textures = vec!["startImage".to_string(), "endImage".to_string()];
        let source = r#"
            vec4 SampleWithBorder(vec4 border, sampler2D tex, vec2 uv) {
                if (uv.x < 0.0) return border;
                return texture(tex, uv);
            }
            void main() {
                vec4 a = SampleWithBorder(vec4(0.0), startImage, uv);
                vec4 b = SampleWithBorder(vec4(1.0), endImage, uv);
            }
        "#;
        let out = inline_sampler_functions(source, &textures);
        assert!(
            !out.contains("sampler2D tex"),
            "the sampler parameter is gone: {out}"
        );
        assert!(
            out.contains("SampleWithBorder_isf_startImage(vec4 border, vec2 uv)"),
            "{out}"
        );
        assert!(
            out.contains("SampleWithBorder_isf_endImage(vec4 border, vec2 uv)"),
            "{out}"
        );
        assert!(
            out.contains("texture(sampler2D(isf_tex_startImage, isf_smp_startImage), uv)"),
            "{out}"
        );
        assert!(
            out.contains("= SampleWithBorder_isf_startImage(vec4(0.0), uv)"),
            "{out}"
        );
        assert!(
            out.contains("= SampleWithBorder_isf_endImage(vec4(1.0), uv)"),
            "{out}"
        );
        assert!(
            !out.contains("SampleWithBorder(vec4"),
            "no call keeps the sampler argument: {out}"
        );
    }

    #[test]
    fn an_uncalled_sampler_function_is_removed() {
        let textures = vec!["startImage".to_string()];
        let source = r#"
            vec4 Unused(vec4 border, sampler2D tex, vec2 uv) { return texture(tex, uv); }
            void main() { gl_FragColor = texture(startImage, uv); }
        "#;
        let out = inline_sampler_functions(source, &textures);
        assert!(!out.contains("sampler2D"), "{out}");
        assert!(!out.contains("Unused"), "{out}");
        assert!(out.contains("void main()"), "{out}");
    }

    #[test]
    fn a_sampler_argument_that_is_not_a_texture_is_left_alone() {
        let textures = vec!["startImage".to_string()];
        let source = r#"
            vec4 Sample(vec4 border, sampler2D tex, vec2 uv) { return texture(tex, uv); }
            void main() { gl_FragColor = Sample(vec4(0.0), other, uv); }
        "#;
        let out = inline_sampler_functions(source, &textures);
        assert_eq!(out, source, "an unknown sampler argument is not guessed at");
    }

    #[test]
    fn nested_sampler_functions_are_inlined_through_the_split_form() {
        let textures = vec!["startImage".to_string()];
        let source = r#"
            vec4 Inner(vec4 border, sampler2D tex, vec2 uv) { return texture(tex, uv); }
            vec4 Outer(sampler2D tex, vec2 uv) { return Inner(vec4(0.0), tex, uv); }
            void main() { gl_FragColor = Outer(startImage, uv); }
        "#;
        let out = inline_sampler_functions(source, &textures);
        assert!(!out.contains("sampler2D tex"), "{out}");
        assert!(out.contains("Inner_isf_startImage(vec4(0.0), uv)"), "{out}");
        assert!(out.contains("Outer_isf_startImage(uv)"), "{out}");
    }

    #[test]
    fn non_constant_global_initialisers_move_into_the_entry_point() {
        let shader = r#"/*{ "INPUTS": [
            { "NAME": "progress", "TYPE": "float" },
            { "NAME": "steps", "TYPE": "float" }
        ] }*/
        float d = min(progress, 1.0 - progress);
        float dist = steps > 0.0 ? ceil(d * float(steps)) / float(steps) : d;
        vec2 squareSize = 2.0 * dist / vec2(4.0);
        void main() { gl_FragColor = vec4(squareSize, dist, 1.0); }"#;
        let glsl = build_fragment_glsl(&doc(shader), 0);
        assert!(glsl.contains("float d;"), "{glsl}");
        assert!(glsl.contains("float dist;"), "{glsl}");
        assert!(glsl.contains("vec2 squareSize;"), "{glsl}");
        assert!(!glsl.contains("float d = min("), "{glsl}");
        let init = glsl
            .find("void isf_init_globals() {")
            .expect("initialiser function");
        let d = glsl[init..].find("d = min(progress, 1.0 - progress);");
        let dist = glsl[init..].find("dist = steps > 0.0");
        let square = glsl[init..].find("squareSize = 2.0 * dist");
        assert!(
            d.is_some() && dist.is_some() && square.is_some(),
            "all three assignments are hoisted: {glsl}"
        );
        assert!(
            d < dist && dist < square,
            "declaration order is kept: {glsl}"
        );
        assert!(
            glsl.contains("  isf_init_globals();\n  isf_user_main();"),
            "{glsl}"
        );
        assert!(!glsl.contains("void main() { gl_FragColor"), "{glsl}");
    }

    #[test]
    fn constant_global_initialisers_stay_at_file_scope() {
        let shader = r#"const float pi = 3.14159;
        const vec4 tint = vec4(1.0);
        void main() { gl_FragColor = tint * pi; }"#;
        let glsl = build_fragment_glsl(&doc(shader), 0);
        assert!(glsl.contains("const float pi = 3.14159;"), "{glsl}");
        assert!(!glsl.contains("isf_init_globals"), "{glsl}");
        assert!(!glsl.contains("isf_user_main"), "{glsl}");
        assert!(glsl.contains("void main() {"), "{glsl}");
    }

    #[test]
    fn globals_are_not_hoisted_out_of_a_preprocessor_branch() {
        let shader = r#"#if __VERSION__ <= 120
        float legacy = 1.0;
        #else
        float legacy = 2.0;
        #endif
        void main() { gl_FragColor = vec4(legacy); }"#;
        let glsl = build_fragment_glsl(&doc(shader), 0);
        assert!(!glsl.contains("isf_init_globals"), "{glsl}");
    }

    #[test]
    fn non_void_functions_get_a_default_return() {
        let source = r#"
            float f(float x) { if (x > 0.0) return x; }
            vec4 g(vec4 c) { if (c.a > 0.5) return c; }
            int h(int x) { for (int i = 0; i < x; i++) { x += i; } }
            void main() {}
        "#;
        let out = append_default_returns(source);
        assert!(
            out.contains("if (x > 0.0) return x; \n  return 0.0;"),
            "{out}"
        );
        assert!(out.contains("return vec4(0.0);"), "{out}");
        assert!(out.contains("return 0;"), "{out}");
        assert!(
            !out.contains("void main() {\n  return"),
            "void functions are untouched: {out}"
        );
    }

    #[test]
    fn a_function_that_always_returns_is_not_given_dead_code() {
        let source = "float f(float x) { return x; }";
        assert_eq!(append_default_returns(source), source);
        // A trailing `if/else` whose branches both return still gets the
        // deterministic default: proving every branch returns needs a control
        // flow analysis, and unreachable `return` statements are legal.
        let source = "float f(float x) { if (x > 0.0) { return x; } else { return -x; } }";
        assert!(append_default_returns(source).contains("return 0.0;"));
    }

    #[test]
    fn an_unknown_return_type_is_not_given_a_guess() {
        let source = "struct S { float x; };\nS make() { S value; }";
        assert_eq!(append_default_returns(source), source);
    }

    #[test]
    fn matrix_varyings_split_into_columns() {
        let shader = r#"/*{ "INPUTS": [ { "NAME": "angle", "TYPE": "float" } ] }*/
        void main() { gl_FragColor = vec4(angle); }"#;
        let vertex = r#"
varying mat2 rotmat;
void main() {
    isf_vertShaderInit();
    float theta = radians(angle);
    float c = cos(theta);
    float s = sin(theta);
    rotmat = mat2(c, s, -s, c);
}"#;
        let document = parse_isf(shader, Some(vertex)).expect("parses");
        let glsl = build_vertex_glsl(&document);
        assert!(
            glsl.contains("layout(location = 1) out vec2 rotmat_0;"),
            "{glsl}"
        );
        assert!(
            glsl.contains("layout(location = 2) out vec2 rotmat_1;"),
            "{glsl}"
        );
        assert!(!glsl.contains("out mat2"), "{glsl}");
        assert!(
            glsl.contains(
                "isf_mat_0 = mat2(c, s, -s, c); rotmat_0 = isf_mat_0[0]; rotmat_1 = isf_mat_0[1];"
            ),
            "{glsl}"
        );
    }

    #[test]
    fn a_fragment_matrix_varying_is_rebuilt_at_its_use_site() {
        let source = r#"varying mat2 rotmat;
        void main() { vec2 point = (isf_FragNormCoord - 0.5) * rotmat + 0.5; }"#;
        let out = rewrite_fragment_source(source, &[]);
        assert!(out.contains("vec2 rotmat_0;"), "{out}");
        assert!(out.contains("vec2 rotmat_1;"), "{out}");
        assert!(out.contains("mat2(rotmat_0, rotmat_1)"), "{out}");
        assert!(!out.contains("mat2 rotmat;"), "{out}");
    }

    #[test]
    fn imgs_that_expand_to_sampling_calls_get_split_samplers() {
        let textures = vec!["inputImage".to_string()];
        let expanded =
            expand_img_macros("IMG_THIS_NORM_PIXEL(inputImage) + IMG_NORM_PIXEL(inputImage, uv)");
        let out = substitute_sampler_arguments(&expanded, &textures);
        assert!(out.contains(
            "texture(sampler2D(isf_tex_inputImage, isf_smp_inputImage), isf_FragNormCoord)"
        ));
        assert!(
            out.contains("VVSAMPLER_2DBYNORM(sampler2D(isf_tex_inputImage, isf_smp_inputImage),")
        );
    }

    #[test]
    fn pi_is_not_injected_when_the_shader_declares_a_constant() {
        assert!(declares_identifier("const float PI = 3.14159265358;", "PI"));
        assert!(declares_identifier("#define PI 3.14", "PI"));
        assert!(declares_identifier("float PI;", "PI"));
        assert!(!declares_identifier("float PITCH;", "PI"));
        assert!(!declares_identifier("vec2 uv = PIXEL_COORD;", "PI"));
    }

    #[test]
    fn defines_macro_finds_function_like_and_plain_defines() {
        assert!(defines_macro("#define PI 3.14", "PI"));
        assert!(defines_macro("  #define PANEL_C(x) ((x)+1)", "PANEL_C"));
        assert!(!defines_macro("#define TAU 6.28", "PI"));
        assert!(!defines_macro("// #define PI 3.14", "PI"));
    }

    #[test]
    fn format_number_matches_javascript_s_spelling() {
        assert_eq!(format_number(0.0), "0");
        assert_eq!(format_number(1280.0), "1280");
        assert_eq!(format_number(0.9), "0.9");
        assert_eq!(format_number(0.02), "0.02");
    }

    #[test]
    fn evaluate_size_clamps_to_the_device_dimension_limit() {
        let vars = SizeVars {
            width: 1920.0,
            height: 1080.0,
            max_dimension: 8192,
            ..SizeVars::default()
        };
        assert_eq!(
            evaluate_size("$WIDTH * 1000000", &vars).expect("evaluates"),
            8192,
            "a gigapixel request must clamp to max_texture_dimension_2d"
        );
        assert_eq!(evaluate_size("1073741824", &vars).expect("evaluates"), 8192);
        let tiny = SizeVars {
            max_dimension: 16,
            ..SizeVars::default()
        };
        assert_eq!(evaluate_size("100", &tiny).expect("evaluates"), 16);
    }

    #[test]
    fn evaluate_size_substitutes_the_runtime_scalars() {
        // `ISFRenderer.evaluateSize` replaces every uniform, not just inputs:
        // `$TIME`, `$TIMEDELTA`, `$FRAMEINDEX` and `$PASSINDEX` must resolve.
        let vars = SizeVars {
            width: 100.0,
            height: 50.0,
            time: 2.5,
            time_delta: 0.5,
            frame_index: 7.0,
            pass_index: 3.0,
            ..SizeVars::default()
        };
        assert_eq!(evaluate_size("floor($TIME * 10.0)", &vars).expect("ok"), 25);
        assert_eq!(
            evaluate_size("$FRAMEINDEX + $PASSINDEX", &vars).expect("ok"),
            10
        );
        assert_eq!(evaluate_size("$TIMEDELTA * 4.0", &vars).expect("ok"), 2);
    }

    #[test]
    fn vector_uniforms_are_a_clear_error_not_a_silent_zero() {
        let vars = SizeVars {
            width: 100.0,
            height: 50.0,
            ..SizeVars::default()
        };
        assert_eq!(
            evaluate_size("$RENDERSIZE.x", &vars).expect_err("vector uniform"),
            SizeEvalError::UnknownVariable("RENDERSIZE".into())
        );
        assert_eq!(
            evaluate_size("$DATE", &vars).expect_err("date uniform"),
            SizeEvalError::UnknownVariable("DATE".into())
        );
    }

    #[test]
    fn statement_keyword_inputs_keep_control_flow_intact() {
        // R-26: an input named `return`/`for`/`if` used to be replaced in the
        // body, turning every `return`/`for`/`if` keyword into an identifier.
        let shader = r#"/*{ "INPUTS": [
            { "NAME": "return", "TYPE": "float" },
            { "NAME": "for", "TYPE": "float" },
            { "NAME": "if", "TYPE": "event" }
        ] }*/
        void main() {
            float total = 0.0;
            for (int i = 0; i < 3; i++) { total += 1.0; }
            if (total > 0.0) { gl_FragColor = vec4(total); return; }
            gl_FragColor = vec4(0.0);
        }"#;
        let glsl = build_fragment_glsl(&doc(shader), 0);
        // The declarations are prefixed ...
        assert!(glsl.contains("  float isf_input_return;"), "{glsl}");
        assert!(glsl.contains("  float isf_input_for;"), "{glsl}");
        assert!(glsl.contains("  int isf_bool_isf_input_if;"), "{glsl}");
        // ... and the keywords are untouched.
        assert!(glsl.contains("for (int i = 0; i < 3; i++)"), "{glsl}");
        assert!(glsl.contains("if (total > 0.0)"), "{glsl}");
        assert!(glsl.contains("return;"), "{glsl}");
        assert!(!glsl.contains("isf_input_for ("), "{glsl}");
        crate::isf::translate::translate_fragment(&doc(shader), 0).expect("translates");
    }

    #[test]
    fn qualifier_named_inputs_are_still_rewritten_in_the_body() {
        // `UltimateSpiral.fs` reads its `smooth` input as an expression; the
        // qualifier spelling can only be fixed by renaming body occurrences.
        let shader = r#"/*{ "INPUTS": [ { "NAME": "smooth", "TYPE": "float" } ] }*/
        void main() { gl_FragColor = vec4(step(smooth, 0.5)); }"#;
        let glsl = build_fragment_glsl(&doc(shader), 0);
        assert!(glsl.contains("  float isf_input_smooth;"), "{glsl}");
        assert!(glsl.contains("step(isf_input_smooth, 0.5)"), "{glsl}");
        crate::isf::translate::translate_fragment(&doc(shader), 0).expect("translates");
    }

    #[test]
    fn rect_samplers_are_not_rewritten_to_normalised_texture() {
        assert_eq!(
            rewrite_fragment_source("texture2DRect(tex, coord)", &[]),
            "texture2DRect(tex, coord)"
        );
        let shader = "uniform sampler2DRect rect;\nvoid main() { gl_FragColor = texture2DRect(rect, vec2(1.0)); }";
        let error = crate::isf::translate::translate_fragment(&doc(shader), 0)
            .expect_err("rect samplers are rejected");
        assert!(
            matches!(
                error,
                crate::isf::translate::TranslateError::Unsupported { .. }
            ),
            "{error:?}"
        );
        assert!(error.to_string().contains("texture2DRect"));
    }

    #[test]
    fn contains_ident_respects_identifier_boundaries() {
        assert!(contains_ident("texture2DRect(tex, uv)", "texture2DRect"));
        assert!(!contains_ident(
            "my_texture2DRect(tex, uv)",
            "texture2DRect"
        ));
        assert!(!contains_ident("front_texture2DRectX", "texture2DRect"));
        assert!(!contains_ident("texture2D(tex, uv)", "texture2DRect"));
    }

    #[test]
    fn matrix_outputs_consume_one_location_per_column() {
        // `v002 Light Leak.vs` declares `out mat2 rotmat`; WGSL gives a matrix
        // one location per column (mat2 2, mat3 3, mat4 4, mat2x3 2).
        assert_eq!(count_declarators("mat2 rotmat;"), 2);
        assert_eq!(count_declarators("mat3 basis;"), 3);
        assert_eq!(count_declarators("mat4 model;"), 4);
        assert_eq!(count_declarators("mat2x3 m;"), 2);
        assert_eq!(count_declarators("dmat4 model;"), 4);
        assert_eq!(count_declarators("vec2 texOffsets[5];"), 5);
        assert_eq!(count_declarators("mat2 mats[3];"), 6);
        assert_eq!(count_declarators("mat3 a, b;"), 6);

        let body = "out mat2 rotmat;\nout vec2 uv;\n";
        let numbered = assign_output_locations(body, 1);
        assert!(
            numbered.contains("layout(location = 1) out mat2 rotmat;"),
            "{numbered}"
        );
        assert!(
            numbered.contains("layout(location = 3) out vec2 uv;"),
            "{numbered}"
        );
    }
}
