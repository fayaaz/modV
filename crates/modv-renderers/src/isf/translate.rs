//! GLSL (ES → desktop 450) → WGSL translation via naga, plus the tracked
//! corpus report.
//!
//! # Pipeline (`docs/isf-translation-notes.md`)
//!
//! ```text
//! .fs ──[source::build_fragment_glsl]──▶ desktop GLSL 450
//!                                        │
//!                     naga::front::glsl (stage = Fragment, __VERSION__ = 450)
//!                                        ▼
//!                              naga::valid::Validator (all flags/capabilities)
//!                                        ▼
//!                          naga::back::wgsl (WriterFlags::empty)
//!                                        ▼
//!                                       WGSL
//! ```
//!
//! naga's GLSL front-end only accepts core `#version 440/450/460`; the
//! preprocessor in [`super::source`] performs the ES rewrite. Defining
//! `__VERSION__` matters: without it the `#if __VERSION__ <= 120` branches in
//! 37 corpus shaders select their `varying` declarations. GLSL `bool` in a
//! uniform block is not host-shareable, so `bool`/`event` inputs travel as
//! `int` plus a `bool()` macro (see [`super::source::uniform_members`]).
//!
//! # Overrides
//!
//! When a shader fails to translate and `<name>.wgsl` exists in [`OVERRIDES`],
//! the override is validated and returned instead (`Translated::Override`).
//! Overrides must implement the runtime's binding contract — see
//! `overrides/README.md` and [`super::source::uniform_members`] for the exact
//! `IsfUniforms` member order.
//!
//! Validation is not just "it parses": [`validate_override`] also requires a
//! `@fragment fn main`, a `@location(0) vec4<f32>` output, the generated
//! `IsfUniforms` members in the generated order/types and every texture/sampler
//! binding pair in [`super::source::texture_bindings`] order. A contract
//! violation is a [`TranslateError::InvalidOverride`], surfaced instead of a
//! wgpu validation panic at pipeline creation.
//!
//! Override sources are embedded into the binary by `build.rs`
//! (`EMBEDDED_OVERRIDES`, one `include_str!` per file) so a packaged engine
//! carries them; the committed directory stays the source of truth and is
//! still read first, so `MODV_ISF_OVERRIDES_DIR` can shadow the embedded copy.
//!
//! # Override name collisions
//!
//! The file stem alone cannot prove a shader is the corpus shader it is named
//! after, so an override only applies when
//!
//! * the shader is known to come from the bundled corpus
//!   ([`ShaderOrigin::Corpus`]), or
//! * the parsed `fragment_source` matches the fingerprint recorded for that
//!   name in `data/isf-overrides.json` ([`document_fingerprint`]).
//!
//! A user media shader called `Soft Blur.fs` therefore does not inherit the
//! corpus override; it translates (or fails) on its own merits. Bump the
//! manifest entry when a corpus shader changes.
//!
//! # Corpus report
//!
//! [`corpus_report`] translates every `.fs` in the vendored corpus
//! (`src/renderer/src/application/renderers/isf/modules/isf`) and every `.vs`
//! sidecar. The test suite compares that against the checked-in
//! `data/isf-corpus-baseline.json` ([`CorpusBaseline`]) — the tracked
//! `docs/isf-corpus-report.md` is **not** rewritten by tests; a regression is
//! a failing assertion with a per-shader diff instead. Use
//! [`write_corpus_report`] manually to refresh the markdown.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use super::parser::IsfDocument;
use super::source;
use super::source::UniformKind;

/// Anything that can stop a shader reaching WGSL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TranslateError {
    /// naga's GLSL front-end rejected the preprocessed source.
    GlslParse {
        /// First error with its line/column.
        message: String,
    },
    /// naga's validator rejected the parsed module.
    Validation {
        /// Validator message.
        message: String,
    },
    /// naga's WGSL backend failed to write the module.
    Wgsl {
        /// Writer message.
        message: String,
    },
    /// The shader uses a feature the translator refuses to guess at.
    Unsupported {
        /// Why the shader is rejected.
        message: String,
    },
    /// An override file exists but is not valid WGSL.
    InvalidOverride {
        /// Override path.
        path: PathBuf,
        /// naga's error.
        message: String,
    },
    /// An override file could not be read.
    Io {
        /// Override path.
        path: PathBuf,
        /// OS error.
        message: String,
    },
}

impl fmt::Display for TranslateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TranslateError::GlslParse { message } => write!(f, "GLSL parse error: {message}"),
            TranslateError::Validation { message } => write!(f, "WGSL validation error: {message}"),
            TranslateError::Wgsl { message } => write!(f, "WGSL write error: {message}"),
            TranslateError::Unsupported { message } => {
                write!(f, "unsupported ISF shader: {message}")
            }
            TranslateError::InvalidOverride { path, message } => {
                write!(f, "invalid WGSL override {}: {message}", path.display())
            }
            TranslateError::Io { path, message } => {
                write!(f, "cannot read {}: {message}", path.display())
            }
        }
    }
}

impl std::error::Error for TranslateError {}

/// Directory scanned by [`write_corpus_report`] when
/// `MODV_ISF_CORPUS_DIR` is unset. Relative to the crate root.
pub const DEFAULT_CORPUS_DIR: &str = "../../src/renderer/src/application/renderers/isf/modules/isf";

/// Where WGSL overrides live, relative to the crate root.
pub const OVERRIDES: &str = "src/isf/overrides";

/// Which bundle a shader came from; decides whether a name-keyed override is
/// trusted without a fingerprint (see [`ShaderOrigin`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShaderOrigin {
    /// The vendored corpus (`src/renderer/src/application/renderers/isf/modules/isf`).
    /// The file name identifies the shader, so overrides apply by name.
    Corpus,
    /// A shader from a user media directory. An override only applies when the
    /// body matches the fingerprint in `data/isf-overrides.json` — otherwise a
    /// user shader that happens to share a corpus name would silently inherit
    /// the corpus override.
    External,
}

/// FNV-1a 64-bit fingerprint of a parsed shader's `fragment_source`,
/// spelled `"fnv64:<16 hex digits>"`.
///
/// This is the content guard for overrides: it only has to stop *accidental*
/// name collisions, not to resist an attacker, so a fast non-cryptographic
/// hash is enough. The input is [`IsfDocument::fragment_source`] (the body
/// after the v1 upgrader renames), not the raw file, so the corpus report and
/// the runtime hash the same text.
pub fn document_fingerprint(doc: &IsfDocument) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in doc.fragment_source.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("fnv64:{hash:016x}")
}

/// The checked-in override fingerprints (`data/isf-overrides.json`).
const OVERRIDE_MANIFEST: &str = include_str!("../../data/isf-overrides.json");

#[derive(Debug, Default, PartialEq, Eq)]
struct OverrideManifest {
    fingerprints: BTreeMap<String, String>,
}

impl OverrideManifest {
    fn parse(text: &str) -> Self {
        let value: serde_json::Value =
            serde_json::from_str(text).unwrap_or(serde_json::Value::Null);
        let mut fingerprints = BTreeMap::new();
        if let Some(entries) = value.get("overrides").and_then(|value| value.as_object()) {
            for (name, fingerprint) in entries {
                if let Some(fingerprint) = fingerprint.as_str() {
                    fingerprints.insert(name.clone(), fingerprint.to_string());
                }
            }
        }
        Self { fingerprints }
    }

    /// Whether `doc` is the corpus shader the override for `name` was written
    /// against.
    fn allows(&self, name: &str, doc: &IsfDocument) -> bool {
        self.fingerprints
            .get(name)
            .is_some_and(|fingerprint| *fingerprint == document_fingerprint(doc))
    }
}

fn override_manifest() -> &'static OverrideManifest {
    static MANIFEST: OnceLock<OverrideManifest> = OnceLock::new();
    MANIFEST.get_or_init(|| OverrideManifest::parse(OVERRIDE_MANIFEST))
}

mod generated {
    include!(concat!(env!("OUT_DIR"), "/isf_overrides.rs"));
}

/// Overrides embedded into the binary by `build.rs` (`include_str!` per file).
///
/// This is what packaged builds use: the `src/isf/overrides` directory is a
/// development convenience and may not exist next to the executable.
pub const EMBEDDED_OVERRIDES: &[(&str, &str)] = generated::EMBEDDED_OVERRIDES;

/// How a shader reached WGSL.
#[derive(Clone, Debug, PartialEq)]
pub enum Translated {
    /// Straight naga translation of the preprocessed GLSL.
    Translated,
    /// naga failed; a complete WGSL override was substituted.
    Override,
    /// Translation failed and no override was available. Carries the first
    /// error so callers/reports can show it.
    Failed(TranslateError),
}

/// The result of translating one shader.
#[derive(Clone, Debug, PartialEq)]
pub struct TranslatedShader {
    /// The desktop-GLSL 450 source fed to naga (the preprocessed fragment).
    pub glsl: String,
    /// The WGSL module, when translation or an override succeeded.
    pub wgsl: Option<String>,
    /// How the WGSL was obtained.
    pub source: Translated,
}

impl TranslatedShader {
    /// Whether this shader can be compiled into a pipeline.
    pub fn is_usable(&self) -> bool {
        self.wgsl.is_some() && !matches!(self.source, Translated::Failed(_))
    }
}

/// The crate root inferred from `CARGO_MANIFEST_DIR`
/// (`crates/modv-renderers`).
pub fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The default corpus directory (`MODV_ISF_CORPUS_DIR` wins).
pub fn default_corpus_dir() -> PathBuf {
    match std::env::var_os("MODV_ISF_CORPUS_DIR") {
        Some(path) => PathBuf::from(path),
        None => crate_root().join(DEFAULT_CORPUS_DIR),
    }
}

/// The default overrides directory (`MODV_ISF_OVERRIDES_DIR` wins).
pub fn default_overrides_dir() -> PathBuf {
    match std::env::var_os("MODV_ISF_OVERRIDES_DIR") {
        Some(path) => PathBuf::from(path),
        None => crate_root().join(OVERRIDES),
    }
}

/// Where the tracked report is written (`MODV_ISF_REPORT_PATH` wins,
/// otherwise `<repo>/docs/isf-corpus-report.md`).
pub fn report_path() -> PathBuf {
    match std::env::var_os("MODV_ISF_REPORT_PATH") {
        Some(path) => PathBuf::from(path),
        None => crate_root().join("../../docs/isf-corpus-report.md"),
    }
}

fn naga_defines() -> naga::FastHashMap<String, String> {
    let mut defines = naga::FastHashMap::default();
    defines.insert("__VERSION__".to_string(), "450".to_string());
    defines
}

fn format_glsl_errors(errors: &naga::front::glsl::ParseErrors, source: &str) -> String {
    let mut message = String::new();
    for error in &errors.errors {
        if !message.is_empty() {
            message.push_str("; ");
        }
        message.push_str(&error.kind.to_string());
        let location = error.meta.location(source);
        message.push_str(&format!(
            " at line {}:{}",
            location.line_number, location.line_position
        ));
    }
    if message.is_empty() {
        "unknown GLSL error".to_string()
    } else {
        message
    }
}

/// Translate one GLSL source with naga.
fn glsl_to_wgsl(source: &str, stage: naga::ShaderStage) -> Result<String, TranslateError> {
    let mut frontend = naga::front::glsl::Frontend::default();
    let options = naga::front::glsl::Options {
        stage,
        defines: naga_defines(),
    };
    let module = frontend
        .parse(&options, source)
        .map_err(|errors| TranslateError::GlslParse {
            message: format_glsl_errors(&errors, source),
        })?;
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    let info = validator
        .validate(&module)
        .map_err(|error| TranslateError::Validation {
            message: format!("{error}"),
        })?;
    naga::back::wgsl::write_string(&module, &info, naga::back::wgsl::WriterFlags::empty()).map_err(
        |error| TranslateError::Wgsl {
            message: error.to_string(),
        },
    )
}

/// Validate a WGSL override against naga and the runtime binding contract.
///
/// Parsing is not enough (R-26): an override with no `main`, a mismatched
/// `IsfUniforms` struct or a missing texture binding would fail inside wgpu
/// when the pipeline is created, which is a panic rather than a diagnostic.
/// Everything the runtime relies on is checked here and reported as
/// [`TranslateError::InvalidOverride`].
fn validate_override(
    source: &str,
    path: &Path,
    doc: &IsfDocument,
) -> Result<String, TranslateError> {
    let module =
        naga::front::wgsl::parse_str(source).map_err(|error| TranslateError::InvalidOverride {
            path: path.to_path_buf(),
            message: format!("{error}"),
        })?;
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    validator
        .validate(&module)
        .map_err(|error| TranslateError::InvalidOverride {
            path: path.to_path_buf(),
            message: format!("{error}"),
        })?;
    check_override_contract(&module, doc).map_err(|message| TranslateError::InvalidOverride {
        path: path.to_path_buf(),
        message,
    })?;
    Ok(source.to_string())
}

/// The `(group, binding)` of a module-scope variable.
fn binding_point(variable: &naga::GlobalVariable) -> Option<(u32, u32)> {
    variable
        .binding
        .as_ref()
        .map(|binding| (binding.group, binding.binding))
}

/// Check that `module` implements what the runtime binds for `doc`.
fn check_override_contract(module: &naga::Module, doc: &IsfDocument) -> Result<(), String> {
    let entry = module
        .entry_points
        .iter()
        .find(|entry| entry.name == "main" && entry.stage == naga::ShaderStage::Fragment)
        .ok_or_else(|| "no `@fragment` entry point named `main`".to_string())?;

    check_fragment_output(module, entry)?;

    let uniform = module
        .global_variables
        .iter()
        .map(|(_, variable)| variable)
        .find(|variable| binding_point(variable) == Some((0, 0)))
        .ok_or_else(|| "no `@group(0) @binding(0)` uniform variable".to_string())?;
    if uniform.space != naga::AddressSpace::Uniform {
        return Err("`@group(0) @binding(0)` is not a uniform buffer".to_string());
    }
    let naga::TypeInner::Struct { members, .. } = &module.types[uniform.ty].inner else {
        return Err("`@group(0) @binding(0)` is not a struct type".to_string());
    };

    let expected = source::uniform_members(doc);
    if members.len() != expected.len() {
        return Err(format!(
            "IsfUniforms has {} members, the runtime binds {} for this shader",
            members.len(),
            expected.len()
        ));
    }
    for (index, (member, expected)) in members.iter().zip(expected.iter()).enumerate() {
        let name = member.name.as_deref().unwrap_or("");
        if name != expected.name {
            return Err(format!(
                "IsfUniforms member {index} is {name:?}, the runtime binds {:?}",
                expected.name
            ));
        }
        if !type_matches(module, member.ty, expected.kind) {
            return Err(format!(
                "IsfUniforms member {name:?} has the wrong type (expected {})",
                expected.kind.glsl_type()
            ));
        }
    }

    for binding in source::texture_bindings(doc) {
        let texture = find_bound_global(module, (0, binding.texture_binding)).ok_or_else(|| {
            format!(
                "no texture at `@group(0) @binding({})` for {:?}",
                binding.texture_binding, binding.name
            )
        })?;
        match &module.types[texture.ty].inner {
            naga::TypeInner::Image {
                dim: naga::ImageDimension::D2,
                arrayed: false,
                class:
                    naga::ImageClass::Sampled {
                        kind: naga::ScalarKind::Float,
                        multi: false,
                    },
            } => {}
            _ => {
                return Err(format!(
                    "{:?} must be bound as `texture_2d<f32>` at `@binding({})`",
                    binding.name, binding.texture_binding
                ));
            }
        }
        let sampler = find_bound_global(module, (0, binding.sampler_binding)).ok_or_else(|| {
            format!(
                "no sampler at `@group(0) @binding({})` for {:?}",
                binding.sampler_binding, binding.name
            )
        })?;
        match &module.types[sampler.ty].inner {
            naga::TypeInner::Sampler { comparison: false } => {}
            _ => {
                return Err(format!(
                    "{:?} must be bound as a non-comparison `sampler` at `@binding({})`",
                    binding.name, binding.sampler_binding
                ));
            }
        }
    }
    Ok(())
}

fn find_bound_global(module: &naga::Module, point: (u32, u32)) -> Option<&naga::GlobalVariable> {
    module
        .global_variables
        .iter()
        .map(|(_, variable)| variable)
        .find(|variable| binding_point(variable) == Some(point))
}

/// A fragment entry point must write `@location(0) vec4<f32>`.
fn check_fragment_output(module: &naga::Module, entry: &naga::EntryPoint) -> Result<(), String> {
    let Some(result) = entry.function.result.as_ref() else {
        return Err("`main` does not return a value".to_string());
    };
    let is_vec4 = |ty| {
        matches!(&module.types[ty].inner, naga::TypeInner::Vector {
        size: naga::VectorSize::Quad,
        scalar,
    } if scalar.kind == naga::ScalarKind::Float && scalar.width == 4)
    };
    let location_zero = |binding: Option<&naga::Binding>| {
        matches!(binding, Some(naga::Binding::Location { location: 0, .. }))
    };
    match &module.types[result.ty].inner {
        naga::TypeInner::Struct { members, .. } => {
            if members
                .iter()
                .any(|member| location_zero(member.binding.as_ref()) && is_vec4(member.ty))
            {
                Ok(())
            } else {
                Err("`main` has no `@location(0) vec4<f32>` member".to_string())
            }
        }
        _ if location_zero(result.binding.as_ref()) && is_vec4(result.ty) => Ok(()),
        _ => Err("`main` must return `@location(0) vec4<f32>`".to_string()),
    }
}

/// Whether a `Type` is the WGSL spelling of a generated uniform member.
fn type_matches(module: &naga::Module, ty: naga::Handle<naga::Type>, kind: UniformKind) -> bool {
    match (&module.types[ty].inner, kind) {
        (naga::TypeInner::Scalar(scalar), UniformKind::Float) => {
            scalar.kind == naga::ScalarKind::Float && scalar.width == 4
        }
        (naga::TypeInner::Scalar(scalar), UniformKind::Int) => {
            scalar.kind == naga::ScalarKind::Sint && scalar.width == 4
        }
        (naga::TypeInner::Vector { size, scalar }, UniformKind::Vec2) => {
            *size == naga::VectorSize::Bi
                && scalar.kind == naga::ScalarKind::Float
                && scalar.width == 4
        }
        (naga::TypeInner::Vector { size, scalar }, UniformKind::Vec4) => {
            *size == naga::VectorSize::Quad
                && scalar.kind == naga::ScalarKind::Float
                && scalar.width == 4
        }
        _ => false,
    }
}

/// Resolve an override for `name`, honouring the fingerprint guard.
///
/// `directory` is tried first (development checkouts,
/// `MODV_ISF_OVERRIDES_DIR` and tests); otherwise the compiled-in table is
/// used, which is what a packaged engine relies on.
fn load_override(
    name: &str,
    doc: &IsfDocument,
    directory: Option<&Path>,
    embedded: &[(&str, &str)],
    origin: ShaderOrigin,
    manifest: &OverrideManifest,
) -> Result<Option<String>, TranslateError> {
    if origin == ShaderOrigin::External && !manifest.allows(name, doc) {
        // The name matches an override but the body does not: a user shader
        // must not inherit corpus behaviour (R-26).
        return Ok(None);
    }

    if let Some(path) = directory
        .map(|directory| directory.join(format!("{name}.wgsl")))
        .filter(|path| path.is_file())
    {
        let source = std::fs::read(&path)
            .map_err(|error| TranslateError::Io {
                path: path.clone(),
                message: error.to_string(),
            })
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())?;
        return validate_override(&source, &path, doc).map(Some);
    }

    if let Some((_, source)) = embedded
        .iter()
        .find(|(embedded_name, _)| *embedded_name == name)
    {
        let label = crate_root().join(OVERRIDES).join(format!("{name}.wgsl"));
        return validate_override(source, &label, doc).map(Some);
    }

    Ok(None)
}

/// Reject the ISF 1.0 pixel-coordinate sampler family.
///
/// `texture2DRect`/`sampler2DRect` take *pixel* coordinates; mapping them to
/// the normalised `texture()` builtin (as the preprocessor used to) samples
/// the wrong texels silently. Rejecting the shader is honest: it needs a real
/// `textureLoad` translation and a rect-sized texture, which the runtime does
/// not provide.
fn reject_rect_samplers(source: &str) -> Result<(), TranslateError> {
    for name in ["texture2DRect", "sampler2DRect"] {
        if source::contains_ident(source, name) {
            return Err(TranslateError::Unsupported {
                message: format!(
                    "{name} samples in pixel coordinates and cannot be mapped to normalised \
                     `texture()` sampling; a WGSL override is required"
                ),
            });
        }
    }
    Ok(())
}

/// Translate one pass of a fragment shader. No override lookup: use
/// [`translate_named`] when the shader has a name.
pub fn translate_fragment(
    doc: &IsfDocument,
    pass_index: usize,
) -> Result<TranslatedShader, TranslateError> {
    reject_rect_samplers(&doc.fragment_source)?;
    let glsl = source::build_fragment_glsl(doc, pass_index);
    let wgsl = glsl_to_wgsl(&glsl, naga::ShaderStage::Fragment)?;
    Ok(TranslatedShader {
        glsl,
        wgsl: Some(wgsl),
        source: Translated::Translated,
    })
}

/// Translate a fragment shader with override fallback from the default
/// overrides directory, treating it as an [`ShaderOrigin::External`] shader
/// (i.e. the fingerprint guard applies).
pub fn translate_named(name: &str, doc: &IsfDocument) -> Result<TranslatedShader, TranslateError> {
    translate_named_with_overrides(
        name,
        doc,
        Some(&default_overrides_dir()),
        ShaderOrigin::External,
    )
}

/// Translate every pass of `doc`, falling back to `<overrides_dir>/<name>.wgsl`
/// (or the embedded copy) when naga rejects any pass.
///
/// Passing `None` disables the directory lookup (the embedded table is still
/// consulted). `origin` decides whether the override is trusted by name
/// ([`ShaderOrigin::Corpus`]) or must match the fingerprint manifest
/// ([`ShaderOrigin::External`]).
pub fn translate_named_with_overrides(
    name: &str,
    doc: &IsfDocument,
    overrides_dir: Option<&Path>,
    origin: ShaderOrigin,
) -> Result<TranslatedShader, TranslateError> {
    reject_rect_samplers(&doc.fragment_source)?;
    let mut first_glsl = String::new();
    let mut first_wgsl = None;
    let mut first_error = None;
    for (index, _) in doc.passes.iter().enumerate() {
        let glsl = source::build_fragment_glsl(doc, index);
        if index == 0 {
            first_glsl = glsl.clone();
        }
        match glsl_to_wgsl(&glsl, naga::ShaderStage::Fragment) {
            Ok(wgsl) => {
                if index == 0 {
                    first_wgsl = Some(wgsl);
                }
            }
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }

    match (first_wgsl, first_error) {
        (Some(wgsl), None) => Ok(TranslatedShader {
            glsl: first_glsl,
            wgsl: Some(wgsl),
            source: Translated::Translated,
        }),
        (_, error) => {
            if let Some(override_source) = load_override(
                name,
                doc,
                overrides_dir,
                EMBEDDED_OVERRIDES,
                origin,
                override_manifest(),
            )? {
                return Ok(TranslatedShader {
                    glsl: first_glsl,
                    wgsl: Some(override_source),
                    source: Translated::Override,
                });
            }
            Err(error.unwrap_or_else(|| TranslateError::GlslParse {
                message: "no pass was translated".to_string(),
            }))
        }
    }
}

/// Whether `wgsl` parses and defines a `@fragment` entry point named `main`.
///
/// This replaces the old `wgsl.contains("fn main")` check, which a comment
/// containing `// fn main` could satisfy (R-26).
pub fn has_fragment_main(wgsl: &str) -> Result<bool, TranslateError> {
    let module = naga::front::wgsl::parse_str(wgsl).map_err(|error| TranslateError::Wgsl {
        message: format!("generated WGSL does not parse: {error}"),
    })?;
    Ok(module
        .entry_points
        .iter()
        .any(|entry| entry.name == "main" && entry.stage == naga::ShaderStage::Fragment))
}

/// Translate the vertex sidecar (a `.vs` file) with the ISF vertex skeleton.
///
/// This is used by the corpus report. The runtime currently draws every pass
/// with its built-in full-screen triangle (`runtime::DEFAULT_VERTEX_WGSL`) and
/// does not consume translated sidecars; see the module docs.
pub fn translate_vertex(doc: &IsfDocument) -> Result<TranslatedShader, TranslateError> {
    reject_rect_samplers(doc.vertex_source.as_deref().unwrap_or(""))?;
    let glsl = source::build_vertex_glsl(doc);
    let wgsl = glsl_to_wgsl(&glsl, naga::ShaderStage::Vertex)?;
    Ok(TranslatedShader {
        glsl,
        wgsl: Some(wgsl),
        source: Translated::Translated,
    })
}

/// Outcome of translating one corpus file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CorpusOutcome {
    /// Translated by naga.
    Translated,
    /// Translated via an override.
    Override,
    /// Failed to parse or translate.
    Failed,
}

/// Which stage a report entry covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CorpusStage {
    /// A `.fs` fragment shader.
    Fragment,
    /// A `.vs` vertex sidecar.
    Vertex,
}

/// One corpus file's result.
#[derive(Clone, Debug, PartialEq)]
pub struct CorpusEntry {
    /// Shader name (file stem).
    pub name: String,
    /// Fragment or vertex.
    pub stage: CorpusStage,
    /// Number of `PASSES` in the document (1 for header-less shaders).
    pub passes: usize,
    /// Whether the shader declares any `PERSISTENT` buffer.
    pub persistent: bool,
    /// Whether the shader declares any `FLOAT` buffer.
    pub float: bool,
    /// Outcome.
    pub outcome: CorpusOutcome,
    /// First error message, when the outcome is `Failed`.
    pub error: Option<String>,
}

/// Aggregate corpus translation results.
#[derive(Clone, Debug, PartialEq)]
pub struct CorpusReport {
    /// The directory scanned.
    pub corpus_dir: PathBuf,
    /// `.fs` files found.
    pub total: usize,
    /// Fragments translated without an override.
    pub translated: usize,
    /// Fragments that used an override.
    pub overridden: usize,
    /// Fragments that failed.
    pub failed: usize,
    /// `.vs` sidecars found.
    pub vertex_total: usize,
    /// Sidecars translated.
    pub vertex_translated: usize,
    /// Sidecars failed.
    pub vertex_failed: usize,
    /// Sum of `PASSES` across all fragments.
    pub total_passes: usize,
    /// Fragments with at least one `PERSISTENT` pass.
    pub persistent_shaders: usize,
    /// Fragments with at least one `FLOAT` pass.
    pub float_shaders: usize,
    /// Per-file results, fragments then vertices, sorted by name.
    pub entries: Vec<CorpusEntry>,
}

impl CorpusReport {
    /// The corpus directory as a repository-relative path when possible.
    pub fn corpus_display(&self) -> String {
        let display = self.corpus_dir.display().to_string();
        if let Some(index) = display.find("src/renderer/") {
            return display[index..].to_string();
        }
        let canonical_dir = std::fs::canonicalize(&self.corpus_dir).ok();
        let canonical_repo = std::fs::canonicalize(crate_root().join("../..")).ok();
        if let (Some(directory), Some(repo)) = (canonical_dir, canonical_repo)
            && let Ok(relative) = directory.strip_prefix(&repo)
        {
            return relative.display().to_string();
        }
        display
    }

    /// Percentage of fragments that translated without an override.
    pub fn translated_percent(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.translated as f64 * 100.0 / self.total as f64
        }
    }

    fn failures(&self) -> impl Iterator<Item = &CorpusEntry> {
        self.entries
            .iter()
            .filter(|entry| entry.outcome == CorpusOutcome::Failed)
    }

    fn overrides(&self) -> impl Iterator<Item = &CorpusEntry> {
        self.entries
            .iter()
            .filter(|entry| entry.outcome == CorpusOutcome::Override)
    }

    /// Render the `docs/isf-corpus-report.md` artefact.
    ///
    /// The test suite never writes this file: it compares the report against
    /// the checked-in `data/isf-corpus-baseline.json` instead, so a
    /// machine/naga-version regression is a failing assertion, not a silent
    /// documentation rewrite.
    pub fn to_markdown(&self) -> String {
        let mut out = String::new();
        out.push_str("# ISF corpus translation report\n\n");
        out.push_str(
            "Generated by `CorpusReport::to_markdown` in \
             `crates/modv-renderers/src/isf/translate.rs`. The test suite does \
             **not** rewrite this file — it asserts the checked-in \
             `crates/modv-renderers/data/isf-corpus-baseline.json` instead. \
             Regenerate both manually with `write_corpus_report` after \
             reviewing a real change. The corpus is the vendored \
             `src/renderer/src/application/renderers/isf/modules/isf`.\n\n",
        );
        out.push_str(&format!(
            "- Corpus directory: `{}`\n",
            self.corpus_display()
        ));
        out.push_str(&format!("- Fragment shaders (`.fs`): **{}**\n", self.total));
        out.push_str(&format!(
            "- Translated without an override: **{}** ({:.1} %)\n",
            self.translated,
            self.translated_percent()
        ));
        out.push_str(&format!(
            "- Translated via an override: **{}**\n",
            self.overridden
        ));
        out.push_str(&format!("- Failed: **{}**\n", self.failed));
        out.push_str(&format!(
            "- Total passes across the corpus: {}\n",
            self.total_passes
        ));
        out.push_str(&format!(
            "- Shaders with a `PERSISTENT` buffer: {}; with a `FLOAT` buffer: {}\n",
            self.persistent_shaders, self.float_shaders
        ));
        out.push_str(&format!(
            "- Vertex sidecars (`.vs`): {} — translated {}, failed {}\n",
            self.vertex_total, self.vertex_translated, self.vertex_failed
        ));
        out.push_str("\n## Overrides\n\n");
        let overrides: Vec<&CorpusEntry> = self.overrides().collect();
        if overrides.is_empty() {
            out.push_str("None.\n");
        } else {
            out.push_str("| shader | stage | passes |\n| --- | --- | --- |\n");
            for entry in overrides {
                out.push_str(&format!(
                    "| `{}` | {} | {} |\n",
                    entry.name,
                    match entry.stage {
                        CorpusStage::Fragment => "fragment",
                        CorpusStage::Vertex => "vertex",
                    },
                    entry.passes
                ));
            }
        }
        out.push_str("\n## Failures\n\n");
        let failures: Vec<&CorpusEntry> = self.failures().collect();
        if failures.is_empty() {
            out.push_str("None.\n");
        } else {
            out.push_str("| shader | stage | passes | first error |\n| --- | --- | --- | --- |\n");
            for entry in failures {
                let error = entry
                    .error
                    .as_deref()
                    .unwrap_or("(no message)")
                    .replace('|', "\\|")
                    .replace('\n', " ");
                out.push_str(&format!(
                    "| `{}` | {} | {} | {} |\n",
                    entry.name,
                    match entry.stage {
                        CorpusStage::Fragment => "fragment",
                        CorpusStage::Vertex => "vertex",
                    },
                    entry.passes,
                    error
                ));
            }
        }
        out.push_str("\n## Notes\n\n");
        out.push_str(
            "- A shader counts as translated only when **every** pass translates.\n\
             - Coordinate systems differ from GL: `isf_FragNormCoord` is top-left \
             origin in wgpu, `gl_FragCoord` is top-left instead of bottom-left. \
             Texture sampling stays visually identical; shaders that treat \
             `isf_FragNormCoord` as a Cartesian coordinate are mirrored.\n\
             - `bool`/`event` inputs travel as `int` uniforms copied into private \
             `bool` globals at the top of the entry point (a `bool(...)` macro \
             would rewrite `.r` swizzles for inputs named `r`), and \
             `_<name>_flip` is an `int` because WGSL forbids `bool` in a \
             uniform block.\n\
             - The remaining failures are the known-hard corners that need \
             hand-written WGSL overrides (see `src/isf/overrides/README.md`): \
             non-constant global initialisers (`Pixelize`), `sampler2D` \
             function parameters (`Point10 *`), a non-void function with a \
             path that returns nothing (`Shape Morph *`), and a `mat2` vertex \
             varying (`v002 Light Leak`).\n\
             - Fragment-declared `in`/`varying` lines become zero-initialised \
             private globals: the runtime always draws the built-in \
             full-screen triangle, so unbound sidecar varyings (the corpus' \
             `left_coord`/`texOffsets` template leftovers) read zero instead \
             of failing to link.\n",
        );
        out
    }

    /// Write the markdown report to [`report_path`], creating parent
    /// directories.
    pub fn write_markdown(&self) -> std::io::Result<PathBuf> {
        self.write_markdown_to(&report_path())
    }

    /// Write the markdown report to an explicit path (tests use this so they
    /// never touch the tracked `docs/` copy).
    pub fn write_markdown_to(&self, path: &Path) -> std::io::Result<PathBuf> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, self.to_markdown())?;
        Ok(path.to_path_buf())
    }

    /// One-line summary, printed by the corpus test.
    pub fn summary(&self) -> String {
        format!(
            "ISF corpus: {} fragments ({} translated, {} override, {} failed, {:.1} %), \
             {} passes, {} persistent / {} float shaders, {} vertex sidecars ({} translated, {} failed)",
            self.total,
            self.translated,
            self.overridden,
            self.failed,
            self.translated_percent(),
            self.total_passes,
            self.persistent_shaders,
            self.float_shaders,
            self.vertex_total,
            self.vertex_translated,
            self.vertex_failed
        )
    }
}

fn read_source(path: &Path) -> Result<String, String> {
    std::fs::read(path)
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .map_err(|error| format!("{}: {error}", path.display()))
}

fn first_line(message: &str) -> String {
    message.lines().next().unwrap_or(message).to_string()
}

/// Build a [`CorpusReport`] by translating every `.fs`/`.vs` in `directory`.
///
/// Missing directories produce an empty report (`total == 0`) rather than an
/// error, so a checkout without the vendored corpus still compiles and tests
/// can assert on [`CorpusReport::total`].
pub fn corpus_report(directory: &Path) -> CorpusReport {
    let mut report = CorpusReport {
        corpus_dir: directory.to_path_buf(),
        total: 0,
        translated: 0,
        overridden: 0,
        failed: 0,
        vertex_total: 0,
        vertex_translated: 0,
        vertex_failed: 0,
        total_passes: 0,
        persistent_shaders: 0,
        float_shaders: 0,
        entries: Vec::new(),
    };

    let Ok(read_dir) = std::fs::read_dir(directory) else {
        return report;
    };
    let mut fragments: Vec<PathBuf> = read_dir
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "fs"))
        .collect();
    fragments.sort();

    let overrides_dir = default_overrides_dir();
    let mut vertex_entries = Vec::new();

    for fragment_path in fragments {
        let Some(stem) = fragment_path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        let name = stem.to_string();
        let vertex_path = fragment_path.with_extension("vs");
        let vertex_source = if vertex_path.is_file() {
            read_source(&vertex_path).ok()
        } else {
            None
        };

        report.total += 1;
        let fragment_source = match read_source(&fragment_path) {
            Ok(source) => source,
            Err(message) => {
                report.failed += 1;
                report.entries.push(CorpusEntry {
                    name,
                    stage: CorpusStage::Fragment,
                    passes: 0,
                    persistent: false,
                    float: false,
                    outcome: CorpusOutcome::Failed,
                    error: Some(message),
                });
                continue;
            }
        };

        let document = match super::parser::parse_isf(&fragment_source, vertex_source.as_deref()) {
            Ok(document) => document,
            Err(error) => {
                report.failed += 1;
                report.entries.push(CorpusEntry {
                    name,
                    stage: CorpusStage::Fragment,
                    passes: 0,
                    persistent: false,
                    float: false,
                    outcome: CorpusOutcome::Failed,
                    error: Some(error.to_string()),
                });
                continue;
            }
        };

        let passes = document.passes.len();
        report.total_passes += passes;
        let persistent = document.passes.iter().any(|pass| pass.persistent);
        let float = document.passes.iter().any(|pass| pass.float);
        report.persistent_shaders += usize::from(persistent);
        report.float_shaders += usize::from(float);

        let (outcome, error) = match translate_named_with_overrides(
            &name,
            &document,
            Some(&overrides_dir),
            ShaderOrigin::Corpus,
        ) {
            Ok(translated) => match translated.source {
                Translated::Override => (CorpusOutcome::Override, None),
                _ => (CorpusOutcome::Translated, None),
            },
            Err(error) => (CorpusOutcome::Failed, Some(first_line(&error.to_string()))),
        };
        match outcome {
            CorpusOutcome::Translated => report.translated += 1,
            CorpusOutcome::Override => report.overridden += 1,
            CorpusOutcome::Failed => report.failed += 1,
        }
        report.entries.push(CorpusEntry {
            name: name.clone(),
            stage: CorpusStage::Fragment,
            passes,
            persistent,
            float,
            outcome,
            error,
        });

        if document.vertex_source.is_some() {
            report.vertex_total += 1;
            let (outcome, error) = match translate_vertex(&document) {
                Ok(_) => (CorpusOutcome::Translated, None),
                Err(error) => (CorpusOutcome::Failed, Some(first_line(&error.to_string()))),
            };
            match outcome {
                CorpusOutcome::Translated => report.vertex_translated += 1,
                CorpusOutcome::Failed => report.vertex_failed += 1,
                CorpusOutcome::Override => {}
            }
            vertex_entries.push(CorpusEntry {
                name,
                stage: CorpusStage::Vertex,
                passes,
                persistent,
                float,
                outcome,
                error,
            });
        }
    }

    report.entries.extend(vertex_entries);
    report
}

/// Build the corpus report and write the tracked markdown artefact.
///
/// `directory` is normally [`default_corpus_dir`]. The markdown destination is
/// [`report_path`] (`MODV_ISF_REPORT_PATH` overrides the tracked path).
/// Write failures are reported on stderr but do not discard the report.
///
/// This is a **manual** tool: the test suite deliberately does not call it, so
/// running `cargo test` never rewrites `docs/isf-corpus-report.md`.
pub fn write_corpus_report(directory: &Path) -> CorpusReport {
    let report = corpus_report(directory);
    if let Err(error) = report.write_markdown() {
        eprintln!("isf corpus report: could not write markdown: {error}");
    }
    report
}

/// The checked-in corpus baseline (`data/isf-corpus-baseline.json`).
pub const CORPUS_BASELINE: &str = include_str!("../../data/isf-corpus-baseline.json");

/// The acceptance bar for translations that need no override (WS-4 in
/// `docs/rust-hybrid.md`).
pub const CORPUS_PASS_RATE: f64 = 95.0;

/// A checked-in snapshot of the corpus translation results.
///
/// The test suite asserts equality against this so a naga upgrade or a
/// preprocessor change cannot move the pass rate silently; a deliberate change
/// updates the baseline and the markdown report together.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CorpusBaseline {
    /// `.fs` files expected.
    pub total: usize,
    /// Shader names expected to translate without an override (sorted).
    pub translated: Vec<String>,
    /// Shader names expected to use an override (sorted).
    pub overridden: Vec<String>,
    /// Shader names expected to fail (sorted).
    pub failed: Vec<String>,
    /// `.vs` sidecars expected.
    pub vertex_total: usize,
    /// Sidecar names expected to fail (sorted).
    pub vertex_failed: Vec<String>,
}

fn string_list(value: Option<&serde_json::Value>) -> Vec<String> {
    let mut names: Vec<String> = value
        .and_then(serde_json::Value::as_array)
        .map(|array| {
            array
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

impl CorpusBaseline {
    /// Parse the JSON manifest. Missing keys default to empty/zero.
    pub fn from_json(text: &str) -> Result<Self, String> {
        let value: serde_json::Value =
            serde_json::from_str(text).map_err(|error| error.to_string())?;
        let count = |key: &str| {
            value
                .get(key)
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0) as usize
        };
        Ok(Self {
            total: count("total"),
            translated: string_list(value.get("translated")),
            overridden: string_list(value.get("overridden")),
            failed: string_list(value.get("failed")),
            vertex_total: count("vertex_total"),
            vertex_failed: string_list(value.get("vertex_failed")),
        })
    }

    /// Snapshot `report` in manifest form (used to regenerate the file).
    pub fn from_report(report: &CorpusReport) -> Self {
        let names = |outcome: CorpusOutcome, stage: CorpusStage| {
            let mut names: Vec<String> = report
                .entries
                .iter()
                .filter(|entry| entry.stage == stage && entry.outcome == outcome)
                .map(|entry| entry.name.clone())
                .collect();
            names.sort();
            names
        };
        Self {
            total: report.total,
            translated: names(CorpusOutcome::Translated, CorpusStage::Fragment),
            overridden: names(CorpusOutcome::Override, CorpusStage::Fragment),
            failed: names(CorpusOutcome::Failed, CorpusStage::Fragment),
            vertex_total: report.vertex_total,
            vertex_failed: names(CorpusOutcome::Failed, CorpusStage::Vertex),
        }
    }

    /// The manifest as pretty JSON (what the checked-in file contains).
    pub fn to_json(&self) -> String {
        let list = |names: &[String]| {
            serde_json::Value::Array(
                names
                    .iter()
                    .map(|name| serde_json::Value::String(name.clone()))
                    .collect(),
            )
        };
        let value = serde_json::json!({
            "total": self.total,
            "translated": list(&self.translated),
            "overridden": list(&self.overridden),
            "failed": list(&self.failed),
            "vertex_total": self.vertex_total,
            "vertex_failed": list(&self.vertex_failed),
        });
        serde_json::to_string_pretty(&value).unwrap_or_default()
    }

    fn translated_percent(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.translated.len() as f64 * 100.0 / self.total as f64
        }
    }

    /// Every way `report` differs from the baseline, as printable lines.
    ///
    /// Empty means the corpus matches; the test turns each line into a failure
    /// message so a regression names the shaders instead of rewriting docs.
    pub fn differences(&self, report: &CorpusReport) -> Vec<String> {
        let actual = Self::from_report(report);
        let mut problems = Vec::new();
        if actual.total != self.total {
            problems.push(format!(
                "fragment count: baseline {}, now {}",
                self.total, actual.total
            ));
        }
        if actual.vertex_total != self.vertex_total {
            problems.push(format!(
                "vertex sidecar count: baseline {}, now {}",
                self.vertex_total, actual.vertex_total
            ));
        }
        let compare = |label: &str, expected: &[String], found: &[String]| {
            let expected: BTreeSet<&String> = expected.iter().collect();
            let found: BTreeSet<&String> = found.iter().collect();
            let regressed: Vec<&String> = expected.difference(&found).copied().collect();
            let appeared: Vec<&String> = found.difference(&expected).copied().collect();
            let mut lines = Vec::new();
            if !regressed.is_empty() {
                lines.push(format!("{label} lost: {regressed:?}"));
            }
            if !appeared.is_empty() {
                lines.push(format!("{label} gained: {appeared:?}"));
            }
            lines
        };
        problems.extend(compare("translated", &self.translated, &actual.translated));
        problems.extend(compare("overridden", &self.overridden, &actual.overridden));
        problems.extend(compare("failed", &self.failed, &actual.failed));
        problems.extend(compare(
            "vertex failed",
            &self.vertex_failed,
            &actual.vertex_failed,
        ));
        if actual.translated_percent() < CORPUS_PASS_RATE {
            problems.push(format!(
                "pass rate {:.1} % is below the {:.1} % acceptance bar",
                actual.translated_percent(),
                CORPUS_PASS_RATE
            ));
        }
        problems
    }
}

/// The checked-in baseline, parsed and memoised.
pub fn corpus_baseline() -> Result<&'static CorpusBaseline, String> {
    static BASELINE: OnceLock<Result<CorpusBaseline, String>> = OnceLock::new();
    BASELINE
        .get_or_init(|| CorpusBaseline::from_json(CORPUS_BASELINE))
        .as_ref()
        .map_err(Clone::clone)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::isf::parser::parse_isf;

    fn doc(source: &str) -> IsfDocument {
        parse_isf(source, None).expect("test shader parses")
    }

    fn assert_valid_wgsl(wgsl: &str) {
        let module = naga::front::wgsl::parse_str(wgsl).expect("WGSL must parse");
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("WGSL must validate");
    }

    /// The smallest override that satisfies the runtime contract for a shader
    /// with no inputs and no texture targets.
    const MINIMAL_OVERRIDE: &str = r#"
struct IsfUniforms {
    DATE: vec4<f32>,
    RENDERSIZE: vec2<f32>,
    PASSINDEX: i32,
    FRAMEINDEX: i32,
    TIME: f32,
    TIMEDELTA: f32,
};

@group(0) @binding(0) var<uniform> global: IsfUniforms;

@fragment
fn main() -> @location(0) vec4<f32> {
    return vec4<f32>(1.0);
}
"#;

    fn temp_override_dir(label: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("modv-isf-{label}-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("temp dir");
        directory
    }

    #[test]
    fn minimal_override_satisfies_the_contract() {
        let document = doc("void main() { gl_FragColor = vec4(1.0); }");
        let path = crate_root().join(OVERRIDES).join("Minimal.wgsl");
        validate_override(MINIMAL_OVERRIDE, &path, &document).expect("contract is satisfied");
    }

    #[test]
    fn an_override_without_main_is_rejected() {
        let document = doc("void main() { gl_FragColor = vec4(1.0); }");
        let source = MINIMAL_OVERRIDE.replace("fn main(", "fn not_main(");
        let error = validate_override(&source, Path::new("NoMain.wgsl"), &document)
            .expect_err("no `main` entry point");
        let message = error.to_string();
        assert!(message.contains("entry point"), "{message}");
        assert!(matches!(error, TranslateError::InvalidOverride { .. }));
    }

    #[test]
    fn an_override_with_a_comment_saying_fn_main_is_rejected() {
        // The old `wgsl.contains("fn main")` check passed this shader.
        let document = doc("void main() { gl_FragColor = vec4(1.0); }");
        let source =
            MINIMAL_OVERRIDE.replace("@fragment\nfn main(", "// fn main\n@fragment\nfn other(");
        let error = validate_override(&source, Path::new("Commented.wgsl"), &document)
            .expect_err("the comment must not satisfy the entry-point check");
        assert!(
            matches!(error, TranslateError::InvalidOverride { .. }),
            "{error:?}"
        );
    }

    #[test]
    fn an_override_with_the_wrong_uniform_block_is_rejected() {
        let document = doc(r#"/*{ "INPUTS": [ { "NAME": "gain", "TYPE": "float" } ] }*/
            void main() { gl_FragColor = vec4(gain); }"#);
        // The generated block has a `gain` member; this override does not.
        let error = validate_override(MINIMAL_OVERRIDE, Path::new("Gain.wgsl"), &document)
            .expect_err("member list must match the generated one");
        let message = error.to_string();
        assert!(message.contains("IsfUniforms"), "{message}");
        assert!(message.contains("runtime binds 7"), "{message}");
    }

    #[test]
    fn an_override_without_the_texture_bindings_is_rejected() {
        let document = doc(
            r#"/*{ "INPUTS": [ { "NAME": "inputImage", "TYPE": "image" } ] }*/
            void main() { gl_FragColor = IMG_THIS_NORM_PIXEL(inputImage); }"#,
        );
        // The uniform block is right, but the texture/sampler pair is missing.
        let source = r#"
struct IsfUniforms {
    DATE: vec4<f32>,
    RENDERSIZE: vec2<f32>,
    PASSINDEX: i32,
    FRAMEINDEX: i32,
    TIME: f32,
    TIMEDELTA: f32,
    _inputImage_imgSize: vec2<f32>,
    _inputImage_imgRect: vec4<f32>,
    _inputImage_flip: i32,
};

@group(0) @binding(0) var<uniform> global: IsfUniforms;

@fragment
fn main() -> @location(0) vec4<f32> {
    return vec4<f32>(1.0);
}
"#;
        let error = validate_override(source, Path::new("Input.wgsl"), &document)
            .expect_err("texture bindings are part of the contract");
        let message = error.to_string();
        assert!(message.contains("binding(1)"), "{message}");
    }

    #[test]
    fn fragment_entry_point_check_is_a_parse_not_a_substring() {
        assert!(
            !has_fragment_main("// fn main\n@compute @workgroup_size(1) fn cs() {}")
                .expect("parses")
        );
        assert!(
            !has_fragment_main(
                "@fragment fn not_main() -> @location(0) vec4<f32> { return vec4<f32>(1.0); }"
            )
            .expect("parses")
        );
        assert!(has_fragment_main(MINIMAL_OVERRIDE).expect("parses"));
    }

    #[test]
    fn an_external_shader_does_not_inherit_an_override_by_name() {
        // R-26: a user shader called `Broken` must not pick up the corpus
        // override for `Broken`.
        let shader = "uniform sampler3D vol;\nvoid main() { gl_FragColor = vec4(1.0); }";
        let document = doc(shader);
        let directory = temp_override_dir("guard");
        std::fs::write(directory.join("Broken.wgsl"), MINIMAL_OVERRIDE).expect("write override");

        // Corpus origin trusts the name ...
        let corpus = load_override(
            "Broken",
            &document,
            Some(&directory),
            &[],
            ShaderOrigin::Corpus,
            &OverrideManifest::default(),
        )
        .expect("corpus override loads")
        .expect("corpus override applies");
        assert_eq!(corpus, MINIMAL_OVERRIDE);

        // ... an external shader needs a matching fingerprint.
        let external = load_override(
            "Broken",
            &document,
            Some(&directory),
            &[],
            ShaderOrigin::External,
            &OverrideManifest::default(),
        )
        .expect("lookup succeeds");
        assert!(external.is_none(), "no fingerprint, no override");

        let fingerprint = document_fingerprint(&document);
        let manifest = OverrideManifest::parse(&format!(
            "{{\"overrides\": {{\"Broken\": \"{fingerprint}\"}}}}"
        ));
        let matched = load_override(
            "Broken",
            &document,
            Some(&directory),
            &[],
            ShaderOrigin::External,
            &manifest,
        )
        .expect("lookup succeeds")
        .expect("matching fingerprint applies the override");
        assert_eq!(matched, MINIMAL_OVERRIDE);

        // A different body with the same name does not match the fingerprint.
        let impostor = doc("void main() { gl_FragColor = vec4(0.0); }");
        let guarded = load_override(
            "Broken",
            &impostor,
            Some(&directory),
            &[],
            ShaderOrigin::External,
            &manifest,
        )
        .expect("lookup succeeds");
        assert!(
            guarded.is_none(),
            "body changed, fingerprint does not match"
        );

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn embedded_overrides_are_used_when_no_directory_has_the_file() {
        let document = doc("void main() { gl_FragColor = vec4(1.0); }");
        let embedded: &[(&str, &str)] = &[("Bundled", MINIMAL_OVERRIDE)];
        let source = load_override(
            "Bundled",
            &document,
            None,
            embedded,
            ShaderOrigin::Corpus,
            &OverrideManifest::default(),
        )
        .expect("lookup succeeds")
        .expect("embedded override applies");
        assert_eq!(source, MINIMAL_OVERRIDE);

        // An embedded override is validated like a file one.
        let broken: &[(&str, &str)] = &[("Bundled", "not wgsl")];
        let error = load_override(
            "Bundled",
            &document,
            None,
            broken,
            ShaderOrigin::Corpus,
            &OverrideManifest::default(),
        )
        .expect_err("invalid embedded override is reported");
        assert!(
            matches!(error, TranslateError::InvalidOverride { .. }),
            "{error:?}"
        );
    }

    #[test]
    fn every_override_file_is_embedded_in_the_binary() {
        let directory = crate_root().join(OVERRIDES);
        let mut files: Vec<String> = std::fs::read_dir(&directory)
            .expect("overrides directory")
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "wgsl")
            })
            .filter_map(|path| {
                path.file_stem()
                    .and_then(|stem| stem.to_str())
                    .map(str::to_string)
            })
            .collect();
        files.sort();
        let embedded: Vec<String> = EMBEDDED_OVERRIDES
            .iter()
            .map(|(name, _)| name.to_string())
            .collect();
        assert_eq!(
            embedded, files,
            "build.rs must embed every src/isf/overrides/*.wgsl"
        );
        assert!(
            EMBEDDED_OVERRIDES
                .iter()
                .all(|(_, source)| !source.is_empty()),
            "embedded overrides carry their source"
        );
    }

    #[test]
    fn fingerprints_change_with_the_body() {
        let first = doc("void main() { gl_FragColor = vec4(1.0); }");
        let second = doc("void main() { gl_FragColor = vec4(0.0); }");
        assert_eq!(
            document_fingerprint(&first),
            document_fingerprint(&doc("void main() { gl_FragColor = vec4(1.0); }"))
        );
        assert_ne!(document_fingerprint(&first), document_fingerprint(&second));
        assert!(document_fingerprint(&first).starts_with("fnv64:"));
        assert_eq!(document_fingerprint(&first).len(), "fnv64:".len() + 16);
    }

    #[test]
    fn simple_gl_frag_color_shader_translates() {
        let shader = "void main() { gl_FragColor = vec4(1.0, 0.0, 0.0, 1.0); }";
        let translated = translate_fragment(&doc(shader), 0).expect("translates");
        assert_eq!(translated.source, Translated::Translated);
        assert!(translated.is_usable());
        let wgsl = translated.wgsl.expect("WGSL");
        assert_valid_wgsl(&wgsl);
        assert!(wgsl.contains("@fragment"), "{wgsl}");
        assert!(wgsl.contains("vec4<f32>"));
    }

    #[test]
    fn main_image_style_helper_function_translates() {
        let shader = r#"
            vec4 mainImage(vec2 uv) {
                return vec4(uv, 0.5, 1.0);
            }
            void main() {
                gl_FragColor = mainImage(gl_FragCoord.xy / RENDERSIZE);
            }"#;
        let wgsl = translate_fragment(&doc(shader), 0)
            .expect("translates")
            .wgsl
            .expect("WGSL");
        assert_valid_wgsl(&wgsl);
        assert!(wgsl.contains("@builtin(position)"));
    }

    #[test]
    fn image_input_translates_to_texture_bindings() {
        let shader = r#"/*{ "INPUTS": [ { "NAME": "inputImage", "TYPE": "image" } ] }*/
        void main() { gl_FragColor = IMG_THIS_NORM_PIXEL(inputImage); }"#;
        let translated = translate_fragment(&doc(shader), 0).expect("translates");
        let wgsl = translated.wgsl.expect("WGSL");
        assert_valid_wgsl(&wgsl);
        assert!(wgsl.contains("@group(0) @binding(1)"), "{wgsl}");
        assert!(wgsl.contains("@group(0) @binding(2)"), "{wgsl}");
        assert!(wgsl.contains("textureSample"), "{wgsl}");
    }

    #[test]
    fn img_norm_pixel_uses_the_sampler_metadata_uniforms() {
        let shader = r#"/*{
            "INPUTS": [ { "NAME": "inputImage", "TYPE": "image" } ],
            "PASSES": [ { "TARGET": "acc", "PERSISTENT": true } ]
        }*/
        void main() {
            gl_FragColor = IMG_NORM_PIXEL(acc, vec2(0.5)) + IMG_PIXEL(inputImage, gl_FragCoord.xy);
        }"#;
        let translated = translate_fragment(&doc(shader), 0).expect("translates");
        let wgsl = translated.wgsl.expect("WGSL");
        assert_valid_wgsl(&wgsl);
        assert!(wgsl.contains("_acc_imgSize"), "{wgsl}");
        assert!(wgsl.contains("_acc_imgRect"), "{wgsl}");
        assert!(wgsl.contains("_acc_flip"), "{wgsl}");
    }

    #[test]
    fn uniform_inputs_reach_the_uniform_block() {
        let shader = r#"/*{ "INPUTS": [
            { "NAME": "bright", "TYPE": "float", "DEFAULT": 0.5 },
            { "NAME": "restart", "TYPE": "event" },
            { "NAME": "mode", "TYPE": "long", "DEFAULT": 1 },
            { "NAME": "tint", "TYPE": "color" },
            { "NAME": "centre", "TYPE": "point2D" }
        ] }*/
        void main() {
            float amount = bright + float(mode) + float(restart);
            gl_FragColor = tint * amount + vec4(centre, 0.0, 1.0);
        }"#;
        let wgsl = translate_fragment(&doc(shader), 0)
            .expect("translates")
            .wgsl
            .expect("WGSL");
        assert_valid_wgsl(&wgsl);
        assert!(wgsl.contains("struct IsfUniforms"), "{wgsl}");
        assert!(wgsl.contains("bright: f32"), "{wgsl}");
        assert!(wgsl.contains("isf_bool_restart: i32"), "{wgsl}");
        assert!(wgsl.contains("mode: i32"), "{wgsl}");
    }

    #[test]
    fn multi_pass_document_translates_every_pass() {
        let shader = r#"/*{
            "INPUTS": [
                { "NAME": "inputImage", "TYPE": "image" },
                { "NAME": "softness", "TYPE": "float", "DEFAULT": 0.9, "MIN": 0.0, "MAX": 1.0 }
            ],
            "PASSES": [
                { "TARGET": "smaller", "WIDTH": "max(floor($WIDTH*0.02),1.0)", "HEIGHT": "max(floor($HEIGHT*0.02),1.0)" },
                { "TARGET": "small", "WIDTH": "max(floor($WIDTH*0.25),1.0)", "HEIGHT": "max(floor($HEIGHT*0.25),1.0)" },
                { }
            ]
        }*/
        void main() {
            vec4 colour = IMG_THIS_NORM_PIXEL(inputImage);
            if (PASSINDEX == 1) { colour = IMG_THIS_NORM_PIXEL(smaller); }
            gl_FragColor = mix(colour, vec4(1.0), softness);
        }"#;
        let translated =
            translate_named_with_overrides("Soft Blur", &doc(shader), None, ShaderOrigin::Corpus)
                .expect("every pass translates");
        assert_eq!(translated.source, Translated::Translated);
        let wgsl = translated.wgsl.expect("WGSL");
        assert_valid_wgsl(&wgsl);
        assert!(wgsl.contains("PASSINDEX"), "{wgsl}");
    }

    #[test]
    fn problematic_shader_uses_an_override() {
        // `sampler3D` is not something naga's GLSL front-end accepts, so this
        // forces the failure path deterministically.
        let shader = "uniform sampler3D vol;\nvoid main() { gl_FragColor = vec4(1.0); }";
        let document = doc(shader);
        let directory =
            std::env::temp_dir().join(format!("modv-isf-override-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("temp dir");
        let override_source = MINIMAL_OVERRIDE;
        std::fs::write(directory.join("Broken.wgsl"), override_source).expect("write override");

        let translated = translate_named_with_overrides(
            "Broken",
            &document,
            Some(&directory),
            ShaderOrigin::Corpus,
        )
        .expect("override is used");
        assert_eq!(translated.source, Translated::Override);
        assert!(translated.wgsl.expect("WGSL").contains("fn main"));

        // Without the override the translation still fails.
        let error = translate_named_with_overrides("Broken", &document, None, ShaderOrigin::Corpus)
            .expect_err("sampler3D is unsupported");
        assert!(matches!(
            error,
            TranslateError::GlslParse { .. } | TranslateError::Validation { .. }
        ));

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn an_invalid_override_is_reported() {
        let shader = "uniform sampler3D vol;\nvoid main() { gl_FragColor = vec4(1.0); }";
        let document = doc(shader);
        let directory =
            std::env::temp_dir().join(format!("modv-isf-bad-override-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("temp dir");
        std::fs::write(directory.join("Broken.wgsl"), "this is not wgsl").expect("write");

        let error = translate_named_with_overrides(
            "Broken",
            &document,
            Some(&directory),
            ShaderOrigin::Corpus,
        )
        .expect_err("invalid override must fail");
        assert!(
            matches!(error, TranslateError::InvalidOverride { .. }),
            "{error:?}"
        );

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn vertex_sidecar_translates() {
        let vertex = r#"#if __VERSION__ <= 120
varying vec2 texOffsets[5];
#else
out vec2 texOffsets[5];
#endif
void main(void) {
  isf_vertShaderInit();
  texOffsets[0] = isf_FragNormCoord;
}"#;
        let document = parse_isf("void main() {}", Some(vertex)).expect("parses");
        let translated = translate_vertex(&document).expect("translates");
        let wgsl = translated.wgsl.expect("WGSL");
        assert_valid_wgsl(&wgsl);
        assert!(wgsl.contains("@vertex"), "{wgsl}");
    }

    #[test]
    fn translation_errors_keep_the_first_location() {
        let shader = "void main() { gl_FragColor = this_does_not_exist; }";
        let error = translate_fragment(&doc(shader), 0).expect_err("fails");
        let message = error.to_string();
        assert!(message.contains("this_does_not_exist"), "{message}");
        assert!(message.contains("line"), "{message}");
    }

    #[test]
    fn corpus_matches_the_checked_in_baseline() {
        let directory = default_corpus_dir();
        assert!(
            directory.is_dir(),
            "corpus directory not found at {} — set MODV_ISF_CORPUS_DIR",
            directory.display()
        );
        let report = corpus_report(&directory);
        println!("{}", report.summary());
        assert!(
            report.total >= 100,
            "expected at least 100 shaders, found {} in {}",
            report.total,
            directory.display()
        );
        assert_eq!(
            report.total,
            report.translated + report.overridden + report.failed,
            "every shader gets exactly one fragment outcome"
        );
        assert_eq!(
            report.vertex_total,
            report.vertex_translated + report.vertex_failed,
            "every sidecar gets exactly one vertex outcome"
        );
        assert!(
            report.total_passes >= report.total,
            "each shader has at least one pass"
        );
        assert!(
            report.translated_percent() >= CORPUS_PASS_RATE,
            "{}",
            report.summary()
        );

        let baseline = corpus_baseline().expect("baseline manifest parses");
        let differences = baseline.differences(&report);
        for entry in report.failures().take(10) {
            println!(
                "corpus failure: {} — {}",
                entry.name,
                entry.error.as_deref().unwrap_or("(no message)")
            );
        }
        assert!(
            differences.is_empty(),
            "corpus differs from crates/modv-renderers/data/isf-corpus-baseline.json:\n{}\n\
             (update the baseline and docs/isf-corpus-report.md together only for a real change)\n\
             baseline JSON:\n{}",
            differences.join("\n"),
            CorpusBaseline::from_report(&report).to_json()
        );

        // The test never writes the tracked report; a machine-readable copy of
        // what it saw goes to the test output directory instead.
        let markdown = report.to_markdown();
        assert!(markdown.contains("# ISF corpus translation report"));
        assert!(markdown.contains(&format!("**{}**", report.total)));
        assert!(markdown.contains("## Failures"));
        let generated = Path::new(env!("OUT_DIR")).join("isf-corpus-report.md");
        report
            .write_markdown_to(&generated)
            .expect("report is writable");
        println!("corpus report written to {}", generated.display());
    }

    #[test]
    fn baseline_json_round_trips() {
        let report = corpus_report(Path::new("/definitely/not/a/real/directory"));
        let baseline = CorpusBaseline::from_report(&report);
        let parsed = CorpusBaseline::from_json(&baseline.to_json()).expect("parses");
        assert_eq!(baseline, parsed);
        let checked_in = corpus_baseline().expect("checked-in baseline parses");
        assert!(
            checked_in.total > 0,
            "the checked-in baseline must describe the real corpus"
        );
        assert_eq!(
            checked_in.translated.len() + checked_in.overridden.len() + checked_in.failed.len(),
            checked_in.total,
            "every baseline fragment has exactly one outcome"
        );
    }

    #[test]
    fn baseline_diffs_name_the_shaders_that_changed() {
        let baseline = CorpusBaseline {
            total: 2,
            translated: vec!["Alpha".into()],
            failed: vec!["Beta".into()],
            ..CorpusBaseline::default()
        };
        let mut report = corpus_report(Path::new("/definitely/not/a/real/directory"));
        report.total = 2;
        report.translated = 1;
        report.failed = 1;
        report.entries = vec![
            CorpusEntry {
                name: "Alpha".into(),
                stage: CorpusStage::Fragment,
                passes: 1,
                persistent: false,
                float: false,
                outcome: CorpusOutcome::Failed,
                error: Some("boom".into()),
            },
            CorpusEntry {
                name: "Gamma".into(),
                stage: CorpusStage::Fragment,
                passes: 1,
                persistent: false,
                float: false,
                outcome: CorpusOutcome::Translated,
                error: None,
            },
        ];
        let differences = baseline.differences(&report);
        assert!(
            differences.iter().any(|line| line.contains("Alpha")),
            "{differences:?}"
        );
        assert!(
            differences.iter().any(|line| line.contains("Gamma")),
            "{differences:?}"
        );
    }

    #[test]
    fn missing_corpus_directory_gives_an_empty_report() {
        let report = corpus_report(Path::new("/definitely/not/a/real/directory"));
        assert_eq!(report.total, 0);
        assert_eq!(report.failed, 0);
        assert_eq!(report.translated_percent(), 0.0);
        assert!(
            report
                .to_markdown()
                .contains("Fragment shaders (`.fs`): **0**")
        );
    }
}
