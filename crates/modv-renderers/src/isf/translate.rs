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
//! the override is validated with naga's WGSL front-end and returned instead
//! (`Translated::Override`). Overrides must implement the runtime's binding
//! contract — see `overrides/README.md` and [`super::source::uniform_members`]
//! for the exact `IsfUniforms` member order.
//!
//! # Corpus report
//!
//! [`write_corpus_report`] translates every `.fs` in the vendored corpus
//! (`src/renderer/src/application/renderers/isf/modules/isf`) and every `.vs`
//! sidecar, then writes `docs/isf-corpus-report.md` (overridable with
//! `MODV_ISF_REPORT_PATH`). The `#[test]` in this module regenerates the
//! artefact, so the tracked file cannot silently rot.

#![forbid(unsafe_code)]

use std::fmt;
use std::path::{Path, PathBuf};

use super::parser::IsfDocument;
use super::source;

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

/// Validate a WGSL override with naga's WGSL front-end. Returns the source on
/// success so callers can keep it verbatim.
fn validate_wgsl(source: &str, path: &Path) -> Result<String, TranslateError> {
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
    Ok(source.to_string())
}

fn load_override(name: &str, directory: &Path) -> Result<Option<String>, TranslateError> {
    let path = directory.join(format!("{name}.wgsl"));
    if !path.is_file() {
        return Ok(None);
    }
    let source = std::fs::read(&path)
        .map_err(|error| TranslateError::Io {
            path: path.clone(),
            message: error.to_string(),
        })
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())?;
    validate_wgsl(&source, &path).map(Some)
}

/// Translate one pass of a fragment shader. No override lookup: use
/// [`translate_named`] when the shader has a name.
pub fn translate_fragment(
    doc: &IsfDocument,
    pass_index: usize,
) -> Result<TranslatedShader, TranslateError> {
    let glsl = source::build_fragment_glsl(doc, pass_index);
    let wgsl = glsl_to_wgsl(&glsl, naga::ShaderStage::Fragment)?;
    Ok(TranslatedShader {
        glsl,
        wgsl: Some(wgsl),
        source: Translated::Translated,
    })
}

/// Translate a fragment shader with override fallback from the default
/// overrides directory.
pub fn translate_named(name: &str, doc: &IsfDocument) -> Result<TranslatedShader, TranslateError> {
    translate_named_with_overrides(name, doc, Some(&default_overrides_dir()))
}

/// Translate every pass of `doc`, falling back to
/// `<overrides_dir>/<name>.wgsl` when naga rejects any pass.
///
/// Passing `None` disables the lookup (used by [`translate_fragment`]).
pub fn translate_named_with_overrides(
    name: &str,
    doc: &IsfDocument,
    overrides_dir: Option<&Path>,
) -> Result<TranslatedShader, TranslateError> {
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
            if let Some(directory) = overrides_dir {
                if let Some(override_source) = load_override(name, directory)? {
                    return Ok(TranslatedShader {
                        glsl: first_glsl,
                        wgsl: Some(override_source),
                        source: Translated::Override,
                    });
                }
            }
            Err(error.unwrap_or_else(|| TranslateError::GlslParse {
                message: "no pass was translated".to_string(),
            }))
        }
    }
}

/// Translate the vertex sidecar (a `.vs` file) with the ISF vertex skeleton.
///
/// This is used by the corpus report. The runtime currently draws every pass
/// with its built-in full-screen triangle (`runtime::DEFAULT_VERTEX_WGSL`) and
/// does not consume translated sidecars; see the module docs.
pub fn translate_vertex(doc: &IsfDocument) -> Result<TranslatedShader, TranslateError> {
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
        if let (Some(directory), Some(repo)) = (canonical_dir, canonical_repo) {
            if let Ok(relative) = directory.strip_prefix(&repo) {
                return relative.display().to_string();
            }
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

    /// Render the tracked `docs/isf-corpus-report.md` artefact.
    pub fn to_markdown(&self) -> String {
        let mut out = String::new();
        out.push_str("# ISF corpus translation report\n\n");
        out.push_str(
            "Generated by `write_corpus_report` in \
             `crates/modv-renderers/src/isf/translate.rs` — run \
             `cargo test -p modv-renderers \
             corpus_report_counts_every_shader_and_writes_the_markdown` to \
             regenerate. The corpus is the vendored \
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
        let path = report_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, self.to_markdown())?;
        Ok(path)
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

        let (outcome, error) =
            match translate_named_with_overrides(&name, &document, Some(&overrides_dir)) {
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
pub fn write_corpus_report(directory: &Path) -> CorpusReport {
    let report = corpus_report(directory);
    if let Err(error) = report.write_markdown() {
        eprintln!("isf corpus report: could not write markdown: {error}");
    }
    report
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
        let translated = translate_named_with_overrides("Soft Blur", &doc(shader), None)
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
        let override_source = r#"
            @group(0) @binding(0) var<uniform> time_uniform: f32;
            @fragment fn main() -> @location(0) vec4<f32> {
                return vec4<f32>(1.0);
            }
        "#;
        std::fs::write(directory.join("Broken.wgsl"), override_source).expect("write override");

        let translated = translate_named_with_overrides("Broken", &document, Some(&directory))
            .expect("override is used");
        assert_eq!(translated.source, Translated::Override);
        assert!(translated.wgsl.expect("WGSL").contains("fn main"));

        // Without the override the translation still fails.
        let error = translate_named_with_overrides("Broken", &document, None)
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

        let error = translate_named_with_overrides("Broken", &document, Some(&directory))
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
    fn corpus_report_counts_every_shader_and_writes_the_markdown() {
        let directory = default_corpus_dir();
        assert!(
            directory.is_dir(),
            "corpus directory not found at {} — set MODV_ISF_CORPUS_DIR",
            directory.display()
        );
        let report = write_corpus_report(&directory);
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
        assert!(report.translated > 0, "{}", report.summary());
        assert!(
            report.total_passes >= report.total,
            "each shader has at least one pass"
        );
        let markdown = report.to_markdown();
        assert!(markdown.contains("# ISF corpus translation report"));
        assert!(markdown.contains(&format!("**{}**", report.total)));
        assert!(markdown.contains("## Failures"));
        // Printed failure lines make the test log useful when the rate drops.
        for entry in report.failures().take(10) {
            println!(
                "corpus failure: {} — {}",
                entry.name,
                entry.error.as_deref().unwrap_or("(no message)")
            );
        }
        let path = report.write_markdown().expect("report is writable");
        assert!(path.is_file(), "{}", path.display());
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
