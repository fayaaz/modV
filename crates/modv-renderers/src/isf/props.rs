//! ISF input → modV prop synthesis and module metadata.
//!
//! Port of `setupModule` in
//! `src/renderer/src/application/renderers/isf/index.js:181-281` and of
//! `getIsfGalleryCategory` / `normalizeModuleCategory` in
//! `src/renderer/src/application/module-category.js`.
//!
//! # Prop table (`index.js:206-275`)
//!
//! | ISF `TYPE` | Prop type | Field synthesis |
//! |------------|-----------|-----------------|
//! | `float` | `float` | `label = LABEL \|\| NAME`, `default = DEFAULT ?? 0.0`, `min`, `max`, `step: 0.01` |
//! | `bool` | `bool` | `default: Boolean(DEFAULT)` |
//! | `long` | `enum` | `label = NAME` (**no** `LABEL` fallback), `enum` from `VALUES` + labels from `LABELS`, `selected: value === DEFAULT` |
//! | `color` | `vec4` | `default: DEFAULT` |
//! | `point2D` | `vec2` | `default: DEFAULT \|\| [0, 0]`, `min`, `max` |
//! | `image` | `texture` | also sets `meta.previewWithOutput = true` |
//! | `event` | `event` | label only |
//! | anything else | — | no prop |
//!
//! The `long` case has two quirks that are preserved deliberately because they
//! are visible in the UI: the prop key is `NAME` but the *label* is also
//! `NAME`, ignoring `LABEL`, and `LABELS[idx]` may be absent for a
//! `VALUES`/`LABELS` mismatch (the JavaScript then renders an `undefined`
//! label; this port uses an empty string).
//!
//! # Metadata table (`index.js:184-192`)
//!
//! | Header key | `meta` field |
//! |------------|--------------|
//! | `CREDIT` | `author` |
//! | `DESCRIPTION` | `description` |
//! | `VSN` | `version` |
//! | `MODV_CATEGORY` | `category`, normalised through [`normalize_module_category`] |
//! | `ISFVSN` (effective) | `isf_version` |
//!
//! The category aliases are ported verbatim from `module-category.js:1-13`;
//! an unknown or missing category yields `None`, which leaves the Gallery's
//! type-based fallback in charge (`Gallery.vue`: ISF defaults to Effects).

#![forbid(unsafe_code)]

use indexmap::IndexMap;
use serde_json::Value;

use super::parser::{IsfDocument, IsfInput};

/// `CATEGORY_ALIASES` from `module-category.js:1-13`, verbatim.
pub const CATEGORY_ALIASES: &[(&str, &str)] = &[
    ("effect", "effects"),
    ("effects", "effects"),
    ("2d", "2d-scenes"),
    ("2d-scene", "2d-scenes"),
    ("2d-scenes", "2d-scenes"),
    ("3d", "3d-scenes"),
    ("3d-scene", "3d-scenes"),
    ("3d-scenes", "3d-scenes"),
    ("three", "three"),
    ("three-js", "three"),
    ("threejs", "three"),
];

/// The prop type an ISF input becomes. Matches the strings
/// `store/modules/dataTypes.js` and the module prop schema use.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IsfPropType {
    /// `float`
    Float,
    /// `bool`
    Bool,
    /// `enum` (from an ISF `long`)
    Enum,
    /// `vec4` (from an ISF `color`)
    Vec4,
    /// `vec2` (from an ISF `point2D`)
    Vec2,
    /// `texture` (from an ISF `image`)
    Texture,
    /// `event`
    Event,
}

impl IsfPropType {
    /// The prop `type` string the Vue prop panels expect.
    pub fn as_str(self) -> &'static str {
        match self {
            IsfPropType::Float => "float",
            IsfPropType::Bool => "bool",
            IsfPropType::Enum => "enum",
            IsfPropType::Vec4 => "vec4",
            IsfPropType::Vec2 => "vec2",
            IsfPropType::Texture => "texture",
            IsfPropType::Event => "event",
        }
    }
}

/// One member of an `enum` prop (`input.VALUES.map(...)`).
#[derive(Clone, Debug, PartialEq)]
pub struct EnumMember {
    /// `LABELS[idx]` (empty when the header omits it).
    pub label: String,
    /// `VALUES[idx]`.
    pub value: Value,
    /// `value === DEFAULT` — drives the initial selection.
    pub selected: bool,
}

/// A synthesised module prop.
#[derive(Clone, Debug, PartialEq)]
pub struct IsfProp {
    /// Prop type.
    pub type_: IsfPropType,
    /// Panel label.
    pub label: String,
    /// Initial value, when the JavaScript sets one.
    pub default: Option<Value>,
    /// Minimum (float/point2D only).
    pub min: Option<f64>,
    /// Maximum (float/point2D only).
    pub max: Option<f64>,
    /// Slider step (float only, always `0.01`).
    pub step: Option<f64>,
    /// Enum members (long only).
    pub enum_members: Option<Vec<EnumMember>>,
}

impl IsfProp {
    fn new(type_: IsfPropType, label: String) -> Self {
        Self {
            type_,
            label,
            default: None,
            min: None,
            max: None,
            step: None,
            enum_members: None,
        }
    }
}

/// Module metadata derived from the ISF header (`index.js:184-192`).
#[derive(Clone, Debug, PartialEq)]
pub struct IsfMeta {
    /// `CREDIT`.
    pub author: Option<String>,
    /// `DESCRIPTION`.
    pub description: Option<String>,
    /// `VSN`.
    pub version: Option<String>,
    /// `MODV_CATEGORY`, normalised; `None` when missing or unknown.
    pub category: Option<String>,
    /// `true` when the shader has an `image` input — the Gallery then renders
    /// the module's output instead of a static thumbnail.
    pub preview_with_output: bool,
    /// The effective `ISFVSN` (`meta.isfVersion` in the JS).
    pub isf_version: f32,
    /// `ISFParser`'s inferred flavour: `filter`, `transition` or `generator`.
    pub filter_type: &'static str,
}

/// `normalizeModuleCategory` from `module-category.js:15-26`, verbatim.
///
/// Lowercase, trim, collapse every run of whitespace/underscore to a single
/// `-`, then map through [`CATEGORY_ALIASES`]. Unknown keys and empty strings
/// return `None`.
pub fn normalize_module_category(category: Option<&str>) -> Option<String> {
    let category = category?;
    if category.trim().is_empty() {
        return None;
    }
    let mut key = String::with_capacity(category.len());
    let mut pending_separator = false;
    for character in category.chars() {
        if character.is_whitespace() || character == '_' {
            pending_separator = !key.is_empty();
            continue;
        }
        if pending_separator {
            key.push('-');
            pending_separator = false;
        }
        key.extend(character.to_lowercase());
    }
    let key = key.trim().to_string();
    CATEGORY_ALIASES
        .iter()
        .find(|(alias, _)| *alias == key)
        .map(|(_, normalised)| (*normalised).to_string())
}

fn label_of(input: &IsfInput) -> String {
    input.label.clone().unwrap_or_else(|| input.name.clone())
}

/// JavaScript `Boolean(value)` (`!!value`).
///
/// Mirrors the JS the engine must match: only `null`, `false`, `0`, `NaN`, `""`
/// and `undefined` are falsy. In particular `1` and `"1"` are true and `"false"`
/// is true, which is why `serde_json::Value::as_bool()` is the wrong coercion
/// for ISF `bool`/`event` props.
pub fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(number) => number.as_f64().is_some_and(|value| value != 0.0),
        Value::String(text) => !text.is_empty(),
        // JavaScript objects and arrays are always truthy, including `[]`/`{}`.
        Value::Array(_) | Value::Object(_) => true,
    }
}

/// Synthesise module props from the parsed inputs, preserving declaration
/// order (`moduleDefinition.props[name] = ...` in the JS).
pub fn synthesise_props(doc: &IsfDocument) -> IndexMap<String, IsfProp> {
    let mut props = IndexMap::new();
    for input in &doc.inputs {
        let prop = match input.type_.as_str() {
            "float" => {
                let mut prop = IsfProp::new(IsfPropType::Float, label_of(input));
                prop.default = Some(if input.has_default() {
                    input.default.clone()
                } else {
                    Value::from(0.0)
                });
                prop.min = input.min;
                prop.max = input.max;
                prop.step = Some(0.01);
                prop
            }
            "bool" => {
                let mut prop = IsfProp::new(IsfPropType::Bool, label_of(input));
                prop.default = Some(Value::Bool(truthy(&input.default)));
                prop
            }
            "long" => {
                // JS quirk: the label is NAME, not LABEL.
                let mut prop = IsfProp::new(IsfPropType::Enum, input.name.clone());
                prop.enum_members = Some(
                    input
                        .values
                        .iter()
                        .enumerate()
                        .map(|(index, value)| EnumMember {
                            label: input.labels.get(index).cloned().unwrap_or_default(),
                            value: value.clone(),
                            selected: *value == input.default,
                        })
                        .collect(),
                );
                prop
            }
            "color" => {
                let mut prop = IsfProp::new(IsfPropType::Vec4, label_of(input));
                if input.has_default() {
                    prop.default = Some(input.default.clone());
                }
                prop
            }
            "point2D" => {
                let mut prop = IsfProp::new(IsfPropType::Vec2, label_of(input));
                prop.default = Some(if truthy(&input.default) {
                    input.default.clone()
                } else {
                    Value::from(vec![0.0, 0.0])
                });
                prop.min = input.min;
                prop.max = input.max;
                prop
            }
            "image" => IsfProp::new(IsfPropType::Texture, label_of(input)),
            "event" => IsfProp::new(IsfPropType::Event, label_of(input)),
            _ => continue,
        };
        props.insert(input.name.clone(), prop);
    }
    props
}

fn string_field(metadata: &Value, key: &str) -> Option<String> {
    match metadata.get(key)? {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

/// Module metadata from the ISF header.
pub fn meta_from_document(doc: &IsfDocument) -> IsfMeta {
    let preview_with_output = doc.inputs.iter().any(|input| input.type_ == "image");
    IsfMeta {
        author: string_field(&doc.metadata, "CREDIT"),
        description: string_field(&doc.metadata, "DESCRIPTION"),
        version: string_field(&doc.metadata, "VSN"),
        category: normalize_module_category(
            doc.metadata.get("MODV_CATEGORY").and_then(Value::as_str),
        ),
        preview_with_output,
        isf_version: doc.version,
        filter_type: doc.filter_type(),
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
    fn float_props_match_the_renderer_table() {
        let source = r#"/*{ "INPUTS": [
            { "NAME": "bright", "TYPE": "float", "MIN": -1.0, "MAX": 1.0, "DEFAULT": 0.25 },
            { "NAME": "bare", "TYPE": "float" }
        ] }*/
        void main() {}"#;
        let props = synthesise_props(&doc(source));
        assert_eq!(props.len(), 2);
        let bright = &props["bright"];
        assert_eq!(bright.type_, IsfPropType::Float);
        assert_eq!(bright.type_.as_str(), "float");
        assert_eq!(bright.label, "bright");
        assert_eq!(bright.default, Some(Value::from(0.25)));
        assert_eq!(bright.min, Some(-1.0));
        assert_eq!(bright.max, Some(1.0));
        assert_eq!(bright.step, Some(0.01));
        assert_eq!(props["bare"].default, Some(Value::from(0.0)));
        assert_eq!(props["bare"].min, None);
    }

    #[test]
    fn bool_props_use_javascript_truthiness() {
        let source = r#"/*{ "INPUTS": [
            { "NAME": "on", "TYPE": "bool", "DEFAULT": true },
            { "NAME": "off", "TYPE": "bool", "DEFAULT": false },
            { "NAME": "missing", "TYPE": "bool" },
            { "NAME": "zero", "TYPE": "bool", "DEFAULT": 0 }
        ] }*/
        void main() {}"#;
        let props = synthesise_props(&doc(source));
        assert_eq!(props["on"].default, Some(Value::Bool(true)));
        assert_eq!(props["off"].default, Some(Value::Bool(false)));
        assert_eq!(props["missing"].default, Some(Value::Bool(false)));
        assert_eq!(props["zero"].default, Some(Value::Bool(false)));
    }

    #[test]
    fn long_props_become_enums_with_a_selected_member() {
        let source = r#"/*{ "INPUTS": [ {
            "NAME": "mode", "LABEL": "Mode (ignored)", "TYPE": "long", "DEFAULT": 2,
            "VALUES": [0, 2, 5], "LABELS": ["Zero", "Two", "Five"]
        } ] }*/
        void main() {}"#;
        let props = synthesise_props(&doc(source));
        let prop = &props["mode"];
        assert_eq!(prop.type_, IsfPropType::Enum);
        assert_eq!(prop.label, "mode", "the long case ignores LABEL");
        let members = prop.enum_members.as_ref().expect("enum members");
        assert_eq!(members.len(), 3);
        assert_eq!(members[0].label, "Zero");
        assert!(!members[0].selected);
        assert!(members[1].selected, "DEFAULT 2 selects the second value");
        assert!(!members[2].selected);
        assert_eq!(members[2].value, Value::from(5));
    }

    #[test]
    fn long_props_tolerate_missing_labels() {
        let source = r#"/*{ "INPUTS": [ {
            "NAME": "mode", "TYPE": "long", "VALUES": [0, 1], "LABELS": ["only"]
        } ] }*/
        void main() {}"#;
        let props = synthesise_props(&doc(source));
        let members = props["mode"].enum_members.as_ref().expect("members");
        assert_eq!(members[1].label, "");
    }

    #[test]
    fn color_props_are_vec4_with_the_header_default() {
        let source = r#"/*{ "INPUTS": [
            { "NAME": "tint", "TYPE": "color", "DEFAULT": [0.5, 0.25, 0.0, 1.0] },
            { "NAME": "plain", "TYPE": "color" }
        ] }*/
        void main() {}"#;
        let props = synthesise_props(&doc(source));
        assert_eq!(props["tint"].type_, IsfPropType::Vec4);
        assert_eq!(
            props["tint"].default,
            Some(serde_json::json!([0.5, 0.25, 0.0, 1.0]))
        );
        assert_eq!(props["plain"].default, None);
    }

    #[test]
    fn point2d_props_default_to_the_origin() {
        let source = r#"/*{ "INPUTS": [
            { "NAME": "centre", "TYPE": "point2D", "DEFAULT": [0.25, 0.75], "MIN": 0.0, "MAX": 1.0 },
            { "NAME": "origin", "TYPE": "point2D" }
        ] }*/
        void main() {}"#;
        let props = synthesise_props(&doc(source));
        assert_eq!(props["centre"].type_, IsfPropType::Vec2);
        assert_eq!(
            props["centre"].default,
            Some(serde_json::json!([0.25, 0.75]))
        );
        assert_eq!(props["centre"].min, Some(0.0));
        assert_eq!(props["centre"].max, Some(1.0));
        assert_eq!(props["origin"].default, Some(serde_json::json!([0.0, 0.0])));
    }

    #[test]
    fn image_and_event_props_have_no_defaults() {
        let source = r#"/*{ "INPUTS": [
            { "NAME": "inputImage", "TYPE": "image" },
            { "NAME": "restartNow", "TYPE": "event" }
        ] }*/
        void main() {}"#;
        let doc = doc(source);
        let props = synthesise_props(&doc);
        assert_eq!(props["inputImage"].type_, IsfPropType::Texture);
        assert_eq!(props["inputImage"].default, None);
        assert_eq!(props["restartNow"].type_, IsfPropType::Event);
        assert_eq!(props["restartNow"].default, None);
        assert!(meta_from_document(&doc).preview_with_output);
    }

    #[test]
    fn labels_fall_back_to_the_input_name() {
        let source = r#"/*{ "INPUTS": [
            { "NAME": "labelled", "TYPE": "float", "LABEL": "A label" },
            { "NAME": "unlabelled", "TYPE": "float" },
            { "NAME": "colour", "TYPE": "color", "LABEL": "Tint" }
        ] }*/
        void main() {}"#;
        let props = synthesise_props(&doc(source));
        assert_eq!(props["labelled"].label, "A label");
        assert_eq!(props["unlabelled"].label, "unlabelled");
        assert_eq!(props["colour"].label, "Tint");
    }

    #[test]
    fn unknown_input_types_produce_no_prop() {
        let source = r#"/*{ "INPUTS": [ { "NAME": "weird", "TYPE": "quantum" } ] }*/
        void main() {}"#;
        assert!(synthesise_props(&doc(source)).is_empty());
    }

    #[test]
    fn prop_order_is_declaration_order() {
        let source = r#"/*{ "INPUTS": [
            { "NAME": "z", "TYPE": "float" },
            { "NAME": "a", "TYPE": "float" },
            { "NAME": "m", "TYPE": "image" }
        ] }*/
        void main() {}"#;
        let props = synthesise_props(&doc(source));
        assert_eq!(
            props.keys().cloned().collect::<Vec<_>>(),
            vec!["z", "a", "m"]
        );
    }

    #[test]
    fn meta_maps_credit_description_version_and_category() {
        let source = r#"/*{
            "CREDIT": "by VIDVOX",
            "DESCRIPTION": "a test shader",
            "VSN": "1.2",
            "ISFVSN": "2",
            "MODV_CATEGORY": "3D Scenes",
            "CATEGORIES": ["Generator"],
            "INPUTS": [ { "NAME": "inputImage", "TYPE": "image" } ]
        }*/
        void main() {}"#;
        let meta = meta_from_document(&doc(source));
        assert_eq!(meta.author.as_deref(), Some("by VIDVOX"));
        assert_eq!(meta.description.as_deref(), Some("a test shader"));
        assert_eq!(meta.version.as_deref(), Some("1.2"));
        assert_eq!(meta.category.as_deref(), Some("3d-scenes"));
        assert!(meta.preview_with_output);
        assert_eq!(meta.isf_version, 2.0);
        assert_eq!(meta.filter_type, "filter");
    }

    #[test]
    fn meta_without_a_header_is_empty() {
        let meta = meta_from_document(&doc("void main() {}"));
        assert_eq!(meta.author, None);
        assert_eq!(meta.description, None);
        assert_eq!(meta.version, None);
        assert_eq!(meta.category, None);
        assert!(!meta.preview_with_output);
    }

    #[test]
    fn category_aliases_match_module_category_js() {
        assert_eq!(
            normalize_module_category(Some("3D Scenes")).as_deref(),
            Some("3d-scenes")
        );
        assert_eq!(
            normalize_module_category(Some("3d_scene")).as_deref(),
            Some("3d-scenes")
        );
        assert_eq!(
            normalize_module_category(Some("2D")).as_deref(),
            Some("2d-scenes")
        );
        assert_eq!(
            normalize_module_category(Some("effect")).as_deref(),
            Some("effects")
        );
        assert_eq!(
            normalize_module_category(Some("  EFFECTS  ")).as_deref(),
            Some("effects")
        );
        assert_eq!(
            normalize_module_category(Some("three-js")).as_deref(),
            Some("three")
        );
        assert_eq!(
            normalize_module_category(Some("ThreeJS")).as_deref(),
            Some("three")
        );
        assert_eq!(normalize_module_category(Some("Blur")), None);
        assert_eq!(normalize_module_category(Some("   ")), None);
        assert_eq!(normalize_module_category(Some("")), None);
        assert_eq!(normalize_module_category(None), None);
    }

    #[test]
    fn category_aliases_are_the_documented_table() {
        assert_eq!(CATEGORY_ALIASES.len(), 11);
        for (alias, value) in CATEGORY_ALIASES {
            assert!(!alias.is_empty());
            assert!(matches!(
                *value,
                "effects" | "2d-scenes" | "3d-scenes" | "three"
            ));
        }
    }
}
