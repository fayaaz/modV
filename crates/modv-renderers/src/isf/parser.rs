//! ISF header parser + ISF v1 upgrader.
//!
//! Port of the two pieces of the vendored `interactive-shader-format` package
//! that run before GLSL compilation (`renderers/isf/index.js:145-179`):
//!
//! * [`parse_isf`] — [`MetadataExtractor`] + `ISFParser.parse` +
//!   `ISFParser.parsePasses` (`ISFParser.js:26-76`), i.e. pull the leading
//!   `/*{ ... }*/` JSON block out of the source, read `INPUTS`, `PASSES`,
//!   `ISFVSN`, `CREDIT`, `DESCRIPTION`, `CATEGORIES`, `MODV_CATEGORY`, and
//!   build the pass list.
//! * [`upgrade_fragment`] / [`upgrade_vertex`] — the `ISFUpgrader` port
//!   (`ISFUpgrader.js:3-37`): fold `PERSISTENT_BUFFERS` into the per-pass
//!   `PERSISTENT` flag and rename the v1 `vv_` identifiers.
//!
//! # What the JavaScript does, and where the port is deliberately stricter
//!
//! * `ISFParser.parse` recovers the header with the comment regex
//!   `/\*([^*]|[\r\n]|(\*+([^*/]|[\r\n])))*\*+/` and then takes the text after
//!   `indexOf(metadataString) + metadataString.length + 2` — the `+ 2` is the
//!   closing `*` and `/`. [`extract_header`] does the same thing with a plain
//!   `/*` … `*/` scan, which is equivalent for a header that is the first
//!   block comment in the file.
//! * `ISFParser.inferISFVersion` returns **1 or 2** and *ignores* an
//!   `"ISFVSN": "1"` header: it only looks for `PERSISTENT_BUFFERS` and the
//!   `vv_` identifiers. This port honours an explicit `ISFVSN < 2` as well,
//!   because the metadata is the documented source of truth; the inferred
//!   value is used when `ISFVSN` is absent. The difference only affects
//!   shaders that declare `ISFVSN: 1` *and* have no v1 markers, which do not
//!   exist in the vendored corpus (they all carry `vv_` markers).
//! * Key lookup is case-insensitive. The vendored corpus has three shaders
//!   with a lowercase `"persistent": true` pass key
//!   (`Auto Color Tone.fs`, `Echo Trace.fs`, `Optical Flow Distort.fs`);
//!   JavaScript would silently drop the flag for those. Matching the
//!   documented ISF key case-insensitively is a bug fix, and it is what keeps
//!   those three buffers persistent.
//! * `CLEAR` is parsed here, although `ISFParser.parsePasses` ignores it
//!   entirely. The runtime honours it (ISF v2 default: clear = true).
//! * `WIDTH`/`HEIGHT` may be JSON numbers (`fractal-mountain-tunnel.fs` writes
//!   `"WIDTH": 1`); they are normalised to strings. A numeric `0` is treated
//!   as absent to reproduce the JavaScript `passDefinition.WIDTH || "$WIDTH"`
//!   fallback.
//!
//! The parser never compiles GLSL; it only reshapes the document. Source
//! assembly lives in [`super::source`] and the naga translation in
//! [`super::translate`].

#![forbid(unsafe_code)]

use std::fmt;

use serde_json::{Map, Value};

/// One entry of the header's `INPUTS` array.
///
/// Field names mirror the ISF JSON keys exactly (`NAME`, `TYPE`, `DEFAULT`,
/// `MIN`, `MAX`, `VALUES`, `LABELS`, `LABEL`). `default` is
/// [`Value::Null`] when the header omits `DEFAULT`; the prop synthesiser
/// distinguishes "absent" from "present but null" via [`IsfInput::has_default`].
#[derive(Clone, Debug, PartialEq)]
pub struct IsfInput {
    /// `NAME` — the uniform name and the prop key.
    pub name: String,
    /// `TYPE` — `float`, `bool`, `long`, `color`, `point2D`, `image`,
    /// `audio`, `audioFFT` or `event`.
    pub type_: String,
    /// `DEFAULT`, or [`Value::Null`] when absent.
    pub default: Value,
    /// `MIN` (numbers only; the corpus never uses strings).
    pub min: Option<f64>,
    /// `MAX`.
    pub max: Option<f64>,
    /// `VALUES` (enumerated `long` inputs).
    pub values: Vec<Value>,
    /// `LABELS` (parallel to [`Self::values`]).
    pub labels: Vec<String>,
    /// `LABEL`.
    pub label: Option<String>,
}

impl IsfInput {
    /// Whether the header declared `DEFAULT` at all.
    ///
    /// The renderer's prop synthesis (`index.js:214`) distinguishes
    /// `typeof input.DEFAULT !== "undefined"`, so `DEFAULT: null` and no
    /// `DEFAULT` key are different.
    pub fn has_default(&self) -> bool {
        !self.default.is_null()
    }
}

/// One entry of the header's `PASSES` array.
///
/// `width`/`height` are `None` when the pass does not override them; the
/// renderer then substitutes `$WIDTH`/`$HEIGHT` (`ISFParser.js:64-65`).
#[derive(Clone, Debug, PartialEq)]
pub struct IsfPass {
    /// `TARGET` — the buffer name this pass renders into. `None` means the
    /// implicit final pass to the destination framebuffer.
    pub target: Option<String>,
    /// `WIDTH` size formula.
    pub width: Option<String>,
    /// `HEIGHT` size formula.
    pub height: Option<String>,
    /// `PERSISTENT` (case-insensitive). A persistent buffer keeps its
    /// contents between frames (ping-pong texture pair).
    pub persistent: bool,
    /// `FLOAT`. Combined with `persistent`, allocates an `Rgba32Float`
    /// texture with nearest filtering — the native replacement for
    /// `float-buffer-support.js`.
    pub float: bool,
    /// `CLEAR`. ISF default is `true`.
    pub clear: bool,
}

impl Default for IsfPass {
    fn default() -> Self {
        Self {
            target: None,
            width: None,
            height: None,
            persistent: false,
            float: false,
            clear: true,
        }
    }
}

/// A fully parsed ISF document.
#[derive(Clone, Debug, PartialEq)]
pub struct IsfDocument {
    /// Effective ISF version. An explicit `ISFVSN` wins; otherwise the
    /// version is inferred from v1 markers (`PERSISTENT_BUFFERS`, `vv_`).
    pub version: f32,
    /// The effective `ISFVSN` exactly as declared, when it was declared.
    pub declared_version: Option<f32>,
    /// The header object with the upgrade applied (`PERSISTENT_BUFFERS`
    /// removed by the v1 upgrader). Empty object for header-less sources.
    pub metadata: Value,
    /// Parsed `INPUTS` in declaration order.
    pub inputs: Vec<IsfInput>,
    /// Parsed `PASSES`; always at least one entry. A header-less source gets
    /// a single implicit pass with no target.
    pub passes: Vec<IsfPass>,
    /// The fragment body *after* the metadata block, with the v1 identifier
    /// renames applied. This is the text the preprocessor assembles around.
    pub fragment_source: String,
    /// The vertex sidecar (`.vs`) after the v1 renames, if one was supplied.
    pub vertex_source: Option<String>,
    /// Whether the v1 upgrader ran.
    pub upgraded: bool,
}

impl IsfDocument {
    /// `ISFParser.hasAudio` — whether any input is an `audio`/`audioFFT`
    /// texture (`ISFParser.js:79-82`).
    pub fn has_audio(&self) -> bool {
        self.inputs
            .iter()
            .any(|input| input.type_ == "audio" || input.type_ == "audioFFT")
    }

    /// `ISFParser.inferFilterType` (`ISFParser.js:250-272`):
    /// `"filter"`, `"transition"` or `"generator"`.
    pub fn filter_type(&self) -> &'static str {
        let has = |name: &str, ty: &str| {
            self.inputs
                .iter()
                .any(|input| input.name == name && input.type_ == ty)
        };
        if has("inputImage", "image") {
            "filter"
        } else if has("startImage", "image") && has("endImage", "image") && has("progress", "float")
        {
            "transition"
        } else {
            "generator"
        }
    }
}

/// Errors from [`parse_isf`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IsfParseError {
    /// The `/* ... */` block is not valid JSON.
    InvalidJson(String),
    /// The header parsed but is not a JSON object.
    MetadataNotAnObject,
    /// `INPUTS` is present but is not an array, or an entry is malformed.
    InvalidInputs(String),
    /// `PASSES` is present but is not an array, or an entry is malformed.
    InvalidPasses(String),
}

impl fmt::Display for IsfParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IsfParseError::InvalidJson(message) => {
                write!(f, "invalid ISF metadata JSON: {message}")
            }
            IsfParseError::MetadataNotAnObject => {
                write!(f, "ISF metadata must be a JSON object")
            }
            IsfParseError::InvalidInputs(message) => write!(f, "invalid INPUTS: {message}"),
            IsfParseError::InvalidPasses(message) => write!(f, "invalid PASSES: {message}"),
        }
    }
}

impl std::error::Error for IsfParseError {}

/// The text before `/*`, the JSON text and the fragment body after `*/`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeaderSplit<'a> {
    /// Text between `/*` and `*/` (braces included).
    pub metadata: &'a str,
    /// Everything after `*/`.
    pub body: &'a str,
}

/// Split the leading `/*{ ... }*/` block from a source.
///
/// Returns `None` when the source has no block comment at all; a header-less
/// source is a valid ISF generator (one implicit pass).
pub fn extract_header(source: &str) -> Option<HeaderSplit<'_>> {
    let start = source.find("/*")?;
    let after_open = start + 2;
    let close = source[after_open..].find("*/")?;
    let end = after_open + close;
    Some(HeaderSplit {
        metadata: &source[after_open..end],
        body: &source[end + 2..],
    })
}

/// Parse the block comment's JSON text (the JavaScript passes exactly this
/// substring — braces included — to `json_parse`).
fn parse_metadata(raw: &str) -> Result<Value, IsfParseError> {
    let value: Value = serde_json::from_str(raw.trim())
        .map_err(|error| IsfParseError::InvalidJson(format!("{error}: {}", raw.trim())))?;
    if !value.is_object() {
        return Err(IsfParseError::MetadataNotAnObject);
    }
    Ok(value)
}

/// Case-insensitive object lookup.
///
/// The corpus is consistent (`AUTO COLOR TONE.fs` and friends write a
/// lowercase `persistent`), and ISF keys are documented as uppercase.
fn ci_get<'a>(map: &'a Map<String, Value>, key: &str) -> Option<&'a Value> {
    if let Some(value) = map.get(key) {
        return Some(value);
    }
    map.iter()
        .find(|(candidate, _)| candidate.eq_ignore_ascii_case(key))
        .map(|(_, value)| value)
}

/// JavaScript truthiness (`!!value`) as applied by `ISFParser.parsePasses`.
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(number) => number.as_f64().is_some_and(|value| value != 0.0),
        Value::String(string) => !string.is_empty(),
        Value::Array(array) => !array.is_empty(),
        Value::Object(_) => true,
    }
}

/// Numbers and strings as a size formula (see the module docs for the `0`
/// fallback rule).
fn size_expr(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) if !text.trim().is_empty() => Some(text.clone()),
        Value::Number(number) => {
            let value = number.as_f64()?;
            if value == 0.0 {
                None
            } else {
                Some(number.to_string())
            }
        }
        _ => None,
    }
}

fn number(value: Option<&Value>) -> Option<f64> {
    match value? {
        Value::Number(number) => number.as_f64(),
        // The corpus never quotes MIN/MAX, but accepting the ISF spelling is
        // cheaper than failing a shader over `"0.5"`.
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

fn strings(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| match item {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn parse_input(value: &Value, index: usize) -> Result<IsfInput, IsfParseError> {
    let object = value
        .as_object()
        .ok_or_else(|| IsfParseError::InvalidInputs(format!("entry {index} is not an object")))?;
    let name = ci_get(object, "NAME")
        .and_then(Value::as_str)
        .ok_or_else(|| IsfParseError::InvalidInputs(format!("entry {index} has no NAME")))?;
    let type_ = ci_get(object, "TYPE")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let default = ci_get(object, "DEFAULT").cloned().unwrap_or(Value::Null);
    let min = number(ci_get(object, "MIN"));
    let max = number(ci_get(object, "MAX"));
    let values = ci_get(object, "VALUES")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let labels = strings(ci_get(object, "LABELS"));
    let label = ci_get(object, "LABEL")
        .and_then(Value::as_str)
        .map(str::to_string);
    Ok(IsfInput {
        name: name.to_string(),
        type_,
        default,
        min,
        max,
        values,
        labels,
        label,
    })
}

fn parse_pass(value: &Value, index: usize) -> Result<IsfPass, IsfParseError> {
    let object = value
        .as_object()
        .ok_or_else(|| IsfParseError::InvalidPasses(format!("entry {index} is not an object")))?;
    let target = ci_get(object, "TARGET")
        .and_then(Value::as_str)
        .map(str::to_string);
    let width = size_expr(ci_get(object, "WIDTH"));
    let height = size_expr(ci_get(object, "HEIGHT"));
    let persistent = ci_get(object, "PERSISTENT").is_some_and(truthy);
    let float = ci_get(object, "FLOAT").is_some_and(truthy);
    // ISF v2 defaults CLEAR to true. `ISFParser` ignores the key entirely
    // (the vendored renderer never clears), but the runtime honours it.
    let clear = ci_get(object, "CLEAR").is_none_or(truthy);
    Ok(IsfPass {
        target,
        width,
        height,
        persistent,
        float,
        clear,
    })
}

fn declared_version(metadata: &Value) -> Option<f32> {
    match metadata.get("ISFVSN")? {
        Value::String(text) => text.trim().parse::<f32>().ok(),
        Value::Number(number) => number.as_f64().map(|value| value as f32),
        _ => None,
    }
}

/// `ISFParser.inferISFVersion` (`ISFParser.js:274-288`) reduced to a boolean:
/// `PERSISTENT_BUFFERS` or any `vv_` identifier means v1.
fn has_v1_markers(metadata: &Value, fragment: &str, vertex: Option<&str>) -> bool {
    let persistent_buffers = metadata.get("PERSISTENT_BUFFERS").is_some_and(truthy);
    persistent_buffers
        || fragment.contains("vv_FragNormCoord")
        || vertex.is_some_and(|source| {
            source.contains("vv_vertShaderInit") || source.contains("vv_FragNormCoord")
        })
}

/// `ISFUpgrader.convertFragment` (`ISFUpgrader.js:3-22`).
///
/// Folds `PERSISTENT_BUFFERS` into the matching passes, removes the key from
/// the metadata and renames `vv_FragNormCoord` → `isf_FragNormCoord`.
/// Returns whether the metadata changed.
pub fn upgrade_fragment(metadata: &mut Value, passes: &mut [IsfPass], source: &mut String) -> bool {
    let mut changed = false;
    if let Some(object) = metadata.as_object_mut()
        && let Some(Value::Array(names)) = object.remove("PERSISTENT_BUFFERS")
    {
        for pass in passes.iter_mut() {
            let Some(target) = pass.target.as_deref() else {
                continue;
            };
            if names.iter().any(|name| name.as_str() == Some(target)) {
                pass.persistent = true;
            }
        }
        changed = true;
    }
    let renamed = source.replace("vv_FragNormCoord", "isf_FragNormCoord");
    if renamed != *source {
        *source = renamed;
        changed = true;
    }
    changed
}

/// `ISFUpgrader.convertVertex` (`ISFUpgrader.js:24-36`).
pub fn upgrade_vertex(source: &mut String) -> bool {
    let renamed = source
        .replace("vv_vertShaderInit", "isf_vertShaderInit")
        .replace("vv_FragNormCoord", "isf_FragNormCoord");
    let changed = renamed != *source;
    *source = renamed;
    changed
}

/// Parse an ISF fragment source (and optional vertex sidecar).
///
/// This never touches GLSL: it extracts the metadata, normalises the inputs
/// and passes, and applies the v1 upgrade when needed. The returned
/// [`IsfDocument::fragment_source`] is the body with the header removed.
///
/// # Errors
///
/// [`IsfParseError::InvalidJson`] for a malformed header,
/// [`IsfParseError::InvalidInputs`] / [`IsfParseError::InvalidPasses`] for a
/// malformed list. A source with no header is **not** an error: it becomes a
/// single-pass generator with empty metadata, exactly like `ISFParser`, whose
/// `MetadataExtractor` throws (and `parse` then reports an error) only for
/// genuinely malformed JSON.
pub fn parse_isf(
    fragment_source: &str,
    vertex_source: Option<&str>,
) -> Result<IsfDocument, IsfParseError> {
    let (mut metadata, body) = match extract_header(fragment_source) {
        Some(split) => (parse_metadata(split.metadata)?, split.body.to_string()),
        None => (Value::Object(Map::new()), fragment_source.to_string()),
    };
    if !metadata.is_object() {
        metadata = Value::Object(Map::new());
    }

    let declared = declared_version(&metadata);
    let markers = has_v1_markers(&metadata, fragment_source, vertex_source);
    let version = match declared {
        Some(version) if version > 0.0 && version < 2.0 => version,
        // JS would infer 1 here; the explicit metadata says 2 (or something
        // unparseable) but the source uses v1 markers, so it still needs the
        // upgrade.
        Some(_) if markers => 1.0,
        Some(version) if version > 0.0 => version,
        Some(_) => 2.0,
        None if markers => 1.0,
        None => 2.0,
    };
    let upgraded = version < 2.0;

    let inputs = match metadata.get("INPUTS") {
        Some(Value::Array(entries)) => entries
            .iter()
            .enumerate()
            .map(|(index, entry)| parse_input(entry, index))
            .collect::<Result<Vec<_>, _>>()?,
        Some(_) => {
            return Err(IsfParseError::InvalidInputs(
                "INPUTS is not an array".into(),
            ));
        }
        None => Vec::new(),
    };

    let mut passes = match metadata.get("PASSES") {
        Some(Value::Array(entries)) if !entries.is_empty() => entries
            .iter()
            .enumerate()
            .map(|(index, entry)| parse_pass(entry, index))
            .collect::<Result<Vec<_>, _>>()?,
        Some(Value::Array(_)) | None => vec![IsfPass::default()],
        Some(_) => {
            return Err(IsfParseError::InvalidPasses(
                "PASSES is not an array".into(),
            ));
        }
    };

    let mut fragment = body;
    let mut vertex = vertex_source.map(str::to_string);
    if upgraded {
        upgrade_fragment(&mut metadata, &mut passes, &mut fragment);
        if let Some(vertex) = vertex.as_mut() {
            upgrade_vertex(vertex);
        }
    }

    Ok(IsfDocument {
        version,
        declared_version: declared,
        metadata,
        inputs,
        passes,
        fragment_source: fragment,
        vertex_source: vertex,
        upgraded,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIMPLE: &str = r#"/*{
        "CREDIT": "by modV",
        "ISFVSN": "2",
        "DESCRIPTION": "a test",
        "MODV_CATEGORY": "3D Scenes",
        "INPUTS": [
            { "NAME": "bright", "TYPE": "float", "MIN": 0.0, "MAX": 2.0, "DEFAULT": 1.5 },
            { "NAME": "inputImage", "TYPE": "image" }
        ],
        "PASSES": [
            { "TARGET": "buffer", "WIDTH": "max(floor($WIDTH*0.5),1.0)", "PERSISTENT": true },
            { }
        ]
    }*/

    void main() { gl_FragColor = IMG_THIS_PIXEL(inputImage) * bright; }
"#;

    #[test]
    fn extracts_metadata_and_body() {
        let doc = parse_isf(SIMPLE, None).expect("simple shader parses");
        assert_eq!(doc.metadata["CREDIT"], "by modV");
        assert_eq!(doc.metadata["DESCRIPTION"], "a test");
        assert_eq!(doc.version, 2.0);
        assert_eq!(doc.declared_version, Some(2.0));
        assert!(!doc.upgraded);
        assert_eq!(doc.inputs.len(), 2);
        assert_eq!(doc.inputs[0].name, "bright");
        assert_eq!(doc.inputs[0].type_, "float");
        assert_eq!(doc.inputs[0].min, Some(0.0));
        assert_eq!(doc.inputs[0].max, Some(2.0));
        assert_eq!(doc.inputs[0].default, serde_json::json!(1.5));
        assert_eq!(doc.inputs[1].type_, "image");
        assert!(!doc.inputs[1].has_default());
        assert!(doc.fragment_source.contains("void main()"));
        assert!(!doc.fragment_source.contains("ISFVSN"));
        assert!(!doc.fragment_source.contains("/*"));
    }

    #[test]
    fn parses_passes_with_implicit_final_pass() {
        let doc = parse_isf(SIMPLE, None).expect("parses");
        assert_eq!(doc.passes.len(), 2);
        assert_eq!(doc.passes[0].target.as_deref(), Some("buffer"));
        assert_eq!(
            doc.passes[0].width.as_deref(),
            Some("max(floor($WIDTH*0.5),1.0)")
        );
        assert_eq!(doc.passes[0].height, None, "missing HEIGHT uses $HEIGHT");
        assert!(doc.passes[0].persistent);
        assert!(!doc.passes[0].float);
        assert!(doc.passes[0].clear);
        assert_eq!(doc.passes[1].target, None);
        assert_eq!(doc.passes[1].width, None);
    }

    #[test]
    fn headerless_source_is_a_single_implicit_pass() {
        let doc = parse_isf("void main() { gl_FragColor = vec4(1.0); }", None).expect("parses");
        assert_eq!(doc.version, 2.0);
        assert!(doc.inputs.is_empty());
        assert_eq!(doc.passes, vec![IsfPass::default()]);
        assert!(doc.metadata.is_object());
        assert!(doc.fragment_source.contains("gl_FragColor"));
    }

    #[test]
    fn explicit_v1_is_upgraded() {
        let source = r#"/*{
            "ISFVSN": "1",
            "PERSISTENT_BUFFERS": ["lastData"],
            "PASSES": [ { "TARGET": "lastData" }, { "TARGET": "writeData" } ]
        }*/
        void main() { gl_FragColor = vec4(vv_FragNormCoord, 0.0, 1.0); }
        "#;
        let vertex = "void main() { vv_vertShaderInit(); }";
        let doc = parse_isf(source, Some(vertex)).expect("v1 parses");
        assert_eq!(doc.version, 1.0);
        assert!(doc.upgraded);
        assert!(doc.passes[0].persistent);
        assert!(!doc.passes[1].persistent);
        assert!(doc.metadata.get("PERSISTENT_BUFFERS").is_none());
        assert!(doc.fragment_source.contains("isf_FragNormCoord"));
        assert!(!doc.fragment_source.contains("vv_FragNormCoord"));
        let vertex = doc.vertex_source.expect("vertex kept");
        assert!(vertex.contains("isf_vertShaderInit"));
        assert!(!vertex.contains("vv_vertShaderInit"));
    }

    #[test]
    fn v1_markers_force_the_upgrade_without_an_isfvsn() {
        let source = "/*{ \"PASSES\": [ { \"TARGET\": \"a\" } ] }*/\nvoid main() { gl_FragColor = vec4(vv_FragNormCoord, 0.0, 1.0); }";
        let doc = parse_isf(source, None).expect("parses");
        assert_eq!(doc.version, 1.0);
        assert!(doc.upgraded);
        assert!(doc.fragment_source.contains("isf_FragNormCoord"));
    }

    #[test]
    fn isfvsn_number_is_accepted() {
        let doc = parse_isf("/*{ \"ISFVSN\": 2.0 }*/\nvoid main() {}", None).expect("parses");
        assert_eq!(doc.version, 2.0);
        assert_eq!(doc.declared_version, Some(2.0));
    }

    #[test]
    fn lowercase_persistent_key_is_honoured() {
        let source = r#"/*{
            "PASSES": [
                { "TARGET": "acc", "persistent": true, "float": true },
                { "TARGET": "out", "Persistent": false }
            ]
        }*/
        void main() {}"#;
        let doc = parse_isf(source, None).expect("parses");
        assert!(doc.passes[0].persistent, "lowercase persistent must count");
        assert!(doc.passes[0].float, "lowercase float must count");
        assert!(!doc.passes[1].persistent);
    }

    #[test]
    fn numeric_width_height_are_normalised() {
        let source = r#"/*{ "PASSES": [ { "TARGET": "p", "WIDTH": 1, "HEIGHT": 2, "FLOAT": true } ] }*/
        void main() {}"#;
        let doc = parse_isf(source, None).expect("parses");
        assert_eq!(doc.passes[0].width.as_deref(), Some("1"));
        assert_eq!(doc.passes[0].height.as_deref(), Some("2"));
        assert!(doc.passes[0].float);
    }

    #[test]
    fn zero_width_falls_back_to_the_destination_size() {
        let source = r#"/*{ "PASSES": [ { "TARGET": "p", "WIDTH": 0 } ] }*/\nvoid main() {}"#;
        let doc = parse_isf(source, None).expect("parses");
        assert_eq!(doc.passes[0].width, None);
    }

    #[test]
    fn clear_false_is_honoured() {
        let source = r#"/*{ "PASSES": [ { "TARGET": "p", "CLEAR": false } ] }*/\nvoid main() {}"#;
        let doc = parse_isf(source, None).expect("parses");
        assert!(!doc.passes[0].clear);
    }

    #[test]
    fn malformed_json_is_an_error() {
        let source = "/*{ \"INPUTS\": [ oops ] }*/\nvoid main() {}";
        let error = parse_isf(source, None).expect_err("must fail");
        assert!(matches!(error, IsfParseError::InvalidJson(_)), "{error:?}");
        assert!(error.to_string().contains("invalid ISF metadata JSON"));
    }

    #[test]
    fn non_object_metadata_is_an_error() {
        let error = parse_isf("/*[1, 2, 3]*/\nvoid main() {}", None).expect_err("must fail");
        assert!(
            matches!(error, IsfParseError::MetadataNotAnObject),
            "{error:?}"
        );
    }

    #[test]
    fn inputs_must_be_an_array() {
        let error = parse_isf("/*{ \"INPUTS\": {} }*/\nvoid main() {}", None).expect_err("fails");
        assert!(matches!(error, IsfParseError::InvalidInputs(_)));
    }

    #[test]
    fn input_without_a_name_is_an_error() {
        let source = "/*{ \"INPUTS\": [ { \"TYPE\": \"float\" } ] }*/\nvoid main() {}";
        let error = parse_isf(source, None).expect_err("fails");
        assert!(matches!(error, IsfParseError::InvalidInputs(_)));
        assert!(error.to_string().contains("NAME"));
    }

    #[test]
    fn filter_type_matches_the_javascript() {
        let doc = parse_isf(SIMPLE, None).expect("parses");
        assert_eq!(doc.filter_type(), "filter");
        let transition = r#"/*{ "INPUTS": [
            { "NAME": "startImage", "TYPE": "image" },
            { "NAME": "endImage", "TYPE": "image" },
            { "NAME": "progress", "TYPE": "float" }
        ] }*/
        void main() {}"#;
        let doc = parse_isf(transition, None).expect("parses");
        assert_eq!(doc.filter_type(), "transition");
        let generator = parse_isf("void main() {}", None).expect("parses");
        assert_eq!(generator.filter_type(), "generator");
    }

    #[test]
    fn has_audio_tracks_audio_inputs() {
        let source = r#"/*{ "INPUTS": [
            { "NAME": "spectrum", "TYPE": "audioFFT" },
            { "NAME": "wave", "TYPE": "audio" },
            { "NAME": "gain", "TYPE": "float" }
        ] }*/
        void main() {}"#;
        assert!(parse_isf(source, None).expect("parses").has_audio());
        assert!(!parse_isf(SIMPLE, None).expect("parses").has_audio());
    }

    #[test]
    fn values_and_labels_are_kept_for_long_inputs() {
        let source = r#"/*{ "INPUTS": [ {
            "NAME": "mode", "TYPE": "long", "DEFAULT": 1,
            "VALUES": [0, 1, 2], "LABELS": ["a", "b", "c"]
        } ] }*/
        void main() {}"#;
        let doc = parse_isf(source, None).expect("parses");
        assert_eq!(doc.inputs[0].values.len(), 3);
        assert_eq!(doc.inputs[0].labels, vec!["a", "b", "c"]);
        assert_eq!(doc.inputs[0].default, serde_json::json!(1));
    }
}
