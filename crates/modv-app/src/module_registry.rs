//! The combined module registry (M3): shader modules plus the ported 2D
//! modules.
//!
//! M2 only knew the shader registry ([`modv_renderers::shader::all_modules`]);
//! `modules/makeActiveModule` therefore turned every 2D module into an
//! "unregistered" record and the compositor logged and skipped it. M3 adds the
//! 15 hand-written 2D modules ([`modv_renderers::two_d::all_modules`]) to the
//! same lookup table so
//!
//! * `modules/makeActiveModule` produces a real record with `meta.type: "2d"`
//!   and the module's prop defaults, and
//! * the compositor can tell a shader module (pipeline slot) from a 2D module
//!   (canvas facade instance) by name.
//!
//! ## Two prop-metadata gaps in [`TwoDProp`]
//!
//! `TwoDProp` carries `name`, `prop_type`, `default`, `min`, `max`, `label`,
//! `enum_members`, `component` and `step` — everything the controls need — but
//! **not** the JS descriptor's `strict`/`abs` coercion flags, and
//! [`TwoDModuleMeta`] has no `previewWithOutput`. Rather than change
//! `modv-renderers` in M3, this file keeps the handful of JS declarations that
//! set them in [`TWO_D_PROP_FLAGS`] / [`TWO_D_PREVIEW_WITH_OUTPUT`]. Both
//! tables are exhaustive against the corpus (`grep -n "strict:\\|abs:"
//! renderers/2d/modules/*.js`) and are the intended home for the flags once
//! `TwoDProp`/`TwoDModuleMeta` grow the fields.
//!
//! ## Prop coercion
//!
//! [`ModuleRegistry::prop_schema`] maps a [`TwoDProp`] onto the same
//! [`PropSchema`] the shader path uses, so `modules/updateProp`
//! (`modules.js:481-562`) runs one coercion pipeline for both module types:
//! `type` is the JS `dataTypes` key (`TwoDProp::prop_type`), `min`/`max` come
//! from the descriptor, and `strict`/`abs` from the flags table above.
//!
//! ## Default props and `data`
//!
//! [`ModuleRegistry::default_props`] is the JS `getPropDefault`
//! (`utils/get-prop-default.js`): the descriptor's `default` verbatim, in
//! declaration order. The 2D modules keep their mutable state in the
//! [`TwoDModule`] instance (the port of the JS `data` object), so the store's
//! `data` starts empty — see the M3 report and the compositor's module docs.
//!
//! ## Known name mismatch
//!
//! Two ported 2D modules register under a normalised name (`X Drips`,
//! `Texture 2D`) while the JS Gallery calls them `X-Drips` and `Texture2d`.
//! Presets written by the JS app therefore miss those two modules until the
//! names are reconciled in `modv-renderers` (out of scope for M3).

use serde_json::{Map, Value, json};

use modv_engine::props::PropSchema;
use modv_renderers::shader::uniforms::{PropKind, ShaderProp};
use modv_renderers::shader::{ShaderModule, all_modules as all_shader_modules};
use modv_renderers::two_d::modules::default_props as two_d_default_props;
use modv_renderers::two_d::{
    TwoDModule, TwoDModuleMeta, TwoDProp, all_modules as all_two_d_modules,
};

/// Which renderer owns a registered module.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModuleKind {
    /// A `modv-renderers::shader` module (fragment WGSL, pipeline slot).
    Shader,
    /// A `modv-renderers::two_d` module (canvas facade instance).
    TwoD,
}

/// A registered 2D module: its static metadata and prop descriptors, factored
/// out of the module instance so the store can answer without building one.
#[derive(Clone, Debug, PartialEq)]
pub struct TwoDDefinition {
    /// `meta` (`name`, `author`, `version`, `category`, `audioFeatures`).
    pub meta: TwoDModuleMeta,
    /// Prop descriptors in declaration order.
    pub props: Vec<TwoDProp>,
}

/// The JS 2D props whose descriptor sets `strict: true` (clamp to
/// `[min, max]`). Exhaustive against `renderers/2d/modules/*.js`.
const TWO_D_PROP_FLAGS: &[(&str, &str, bool, bool)] = &[
    // (module, prop, strict, abs)
    ("Ball", "amount", true, false),              // Ball.js:21
    ("Ball", "size", false, true),                // Ball.js:46
    ("Ball", "intensity", false, true),           // Ball.js:56
    ("Line", "linesToShow", true, false),         // Line.js:14
    ("Line", "spacing", false, true),             // Line.js:22
    ("Line", "lineWidth", false, true),           // Line.js:30
    ("Line", "speed", false, true),               // Line.js:54
    ("Concentrics", "strokeWeight", true, false), // Concentrics.js:50
    ("Polygon", "strokeWeight", true, false),     // Polygon.js:36
    ("Text", "strokeSize", false, true),          // Text.js:55
    ("Waveform", "strokeWeight", false, true),    // Waveform.js:19
    ("X Drips", "maxDripHeight", false, true),    // X-Drips.js:28
];

/// The JS 2D modules whose `meta` sets `previewWithOutput: true`.
const TWO_D_PREVIEW_WITH_OUTPUT: &[&str] = &[
    "Grid Stretch", // GridStretch.js:7
    "Pixelate",     // Pixelate.js:9
];

/// Both built-in registries, with the metadata the store and the compositor
/// need.
pub struct ModuleRegistry {
    shader: Vec<ShaderModule>,
    two_d: Vec<TwoDDefinition>,
}

impl ModuleRegistry {
    /// The 37 built-in shader modules plus the 15 ported 2D modules.
    pub fn builtin() -> Self {
        let shader = all_shader_modules();
        // `all_modules()` builds one default instance per module (all cheap,
        // no GPU work) so the descriptors can be copied out once.
        let two_d = all_two_d_modules()
            .into_iter()
            .map(|module| TwoDDefinition {
                meta: module.meta(),
                props: module.props(),
            })
            .collect();
        Self { shader, two_d }
    }

    /// The shader modules, in Gallery registration order.
    pub fn shader_modules(&self) -> &[ShaderModule] {
        &self.shader
    }

    /// The 2D module definitions, in registration order.
    pub fn two_d_definitions(&self) -> &[TwoDDefinition] {
        &self.two_d
    }

    /// Look up a shader module by its registration name (`$moduleName`).
    pub fn shader_module(&self, name: &str) -> Option<&ShaderModule> {
        self.shader.iter().find(|module| module.name == name)
    }

    /// Look up a 2D module definition by its registration name.
    pub fn two_d_definition(&self, name: &str) -> Option<&TwoDDefinition> {
        self.two_d.iter().find(|module| module.meta.name == name)
    }

    /// Which renderer owns `name`, or `None` when it is not registered.
    pub fn kind(&self, name: &str) -> Option<ModuleKind> {
        if self
            .shader_modules()
            .iter()
            .any(|module| module.name == name)
        {
            Some(ModuleKind::Shader)
        } else if self
            .two_d_definitions()
            .iter()
            .any(|module| module.meta.name == name)
        {
            Some(ModuleKind::TwoD)
        } else {
            None
        }
    }

    /// The registry-authored `meta` fields of a known module, or `None` when
    /// the name is not registered.
    ///
    /// This is the JS spread of `moduleDefinition.meta`
    /// (`modules.js:247-249`): `name`, `author`, `version`, `type`, plus
    /// `category`, `audioFeatures` and `previewWithOutput` when declared. The
    /// JS defaults (`alpha`, `enabled`, `compositeOperation`), the caller's
    /// `moduleMeta` and `isGallery` are applied by the store afterwards.
    pub fn meta_fields(&self, name: &str) -> Option<Map<String, Value>> {
        match self.kind(name)? {
            ModuleKind::Shader => {
                let definition = self.shader_module(name)?;
                let mut meta = Map::new();
                meta.insert("name".to_string(), json!(definition.name));
                meta.insert("author".to_string(), json!(definition.meta_author));
                meta.insert("version".to_string(), json!(definition.meta_version));
                meta.insert("type".to_string(), json!("shader"));
                if let Some(category) = definition.category {
                    meta.insert("category".to_string(), json!(category));
                }
                if !definition.audio_features.is_empty() {
                    meta.insert(
                        "audioFeatures".to_string(),
                        json!(definition.audio_features),
                    );
                }
                meta.insert(
                    "previewWithOutput".to_string(),
                    json!(definition.preview_with_output),
                );
                Some(meta)
            }
            ModuleKind::TwoD => {
                let definition = self.two_d_definition(name)?;
                let mut meta = Map::new();
                meta.insert("name".to_string(), json!(definition.meta.name));
                // The JS `meta` object only carries `author`/`version` when the
                // module declares them (`Text.js` declares neither); a spread
                // therefore leaves the keys absent rather than empty.
                if !definition.meta.author.is_empty() {
                    meta.insert("author".to_string(), json!(definition.meta.author));
                }
                if !definition.meta.version.is_empty() {
                    meta.insert("version".to_string(), json!(definition.meta.version));
                }
                meta.insert("type".to_string(), json!("2d"));
                if let Some(category) = definition.meta.category {
                    meta.insert("category".to_string(), json!(category));
                }
                if !definition.meta.audio_features.is_empty() {
                    meta.insert(
                        "audioFeatures".to_string(),
                        json!(definition.meta.audio_features),
                    );
                }
                if TWO_D_PREVIEW_WITH_OUTPUT.contains(&definition.meta.name) {
                    meta.insert("previewWithOutput".to_string(), json!(true));
                }
                Some(meta)
            }
        }
    }

    /// The coercion schema for one of a module's props.
    ///
    /// Shader props never set `strict` (`renderers/shader/modules/*.js`), so
    /// their schemas keep `strict: false`; 2D props take `type`/`min`/`max`
    /// from [`TwoDProp`] and `strict`/`abs` from [`TWO_D_PROP_FLAGS`].
    pub fn prop_schema(&self, module_name: &str, prop: &str) -> Option<PropSchema> {
        if let Some(module) = self.shader_module(module_name) {
            let declared = module.props.iter().find(|declared| declared.name == prop)?;
            return Some(schema_for_shader(declared));
        }

        let definition = self.two_d_definition(module_name)?;
        let declared = definition
            .props
            .iter()
            .find(|declared| declared.name == prop)?;
        Some(schema_for_two_d(definition.meta.name, declared))
    }

    /// The default prop values for a module, as the `props` object of a new
    /// active module (`getPropDefault`, `utils/get-prop-default.js`).
    pub fn default_props(&self, module_name: &str) -> Option<Value> {
        if let Some(module) = self.shader_module(module_name) {
            let mut props = Map::new();
            for prop in &module.props {
                props.insert(prop.name.to_string(), default_shader_prop_value(prop));
            }
            return Some(Value::Object(props));
        }

        let definition = self.two_d_definition(module_name)?;
        Some(two_d_default_props(&definition.props))
    }

    /// Build a fresh 2D module instance for `name`.
    ///
    /// `TwoDModule` is not `Clone` and its state is per instance, so the
    /// compositor asks for a new one when a module first draws. `all_modules()`
    /// constructs the 15 default structs (no GPU work) and the match is
    /// returned; activation/first draw is rare enough that this does not
    /// matter.
    pub fn create_two_d(&self, name: &str) -> Option<Box<dyn TwoDModule>> {
        all_two_d_modules()
            .into_iter()
            .find(|module| module.meta().name == name)
    }
}

/// The `dataTypes` key for a shader prop kind.
fn prop_kind_name(kind: PropKind) -> &'static str {
    match kind {
        PropKind::Float => "float",
        PropKind::Int => "int",
        PropKind::Bool => "bool",
        PropKind::Enum => "enum",
        PropKind::Vec2 => "vec2",
        PropKind::Vec3 => "vec3",
        PropKind::Vec4 => "vec4",
        PropKind::Color => "color",
    }
}

/// Build a [`PropSchema`] from a shader prop declaration.
fn schema_for_shader(prop: &ShaderProp) -> PropSchema {
    PropSchema {
        type_: prop_kind_name(prop.kind).to_string(),
        min: Some(prop.min),
        max: Some(prop.max),
        // JS shader props never set `strict`.
        strict: false,
        abs: false,
        default: Some(default_shader_prop_value(prop)),
    }
}

/// Build a [`PropSchema`] from a 2D prop descriptor.
fn schema_for_two_d(module_name: &str, prop: &TwoDProp) -> PropSchema {
    let (strict, abs) = two_d_prop_flags(module_name, prop.name);
    PropSchema {
        type_: prop.prop_type.to_string(),
        min: prop.min,
        max: prop.max,
        strict,
        abs,
        default: Some(prop.default.clone()),
    }
}

/// The `(strict, abs)` coercion flags of a 2D prop, from [`TWO_D_PROP_FLAGS`].
fn two_d_prop_flags(module_name: &str, prop: &str) -> (bool, bool) {
    TWO_D_PROP_FLAGS
        .iter()
        .find(|(module, name, ..)| *module == module_name && *name == prop)
        .map(|(_, _, strict, abs)| (*strict, *abs))
        .unwrap_or((false, false))
}

/// The JSON default for a shader prop declaration.
///
/// `ShaderProp` carries one `f64` default (every in-tree prop is a scalar
/// float); vector and colour kinds fan that number out to their lanes so the
/// shape is still a valid value for the type. `bool`/`int`/`enum` are cast to
/// their canonical scalar form, mirroring `dataTypes.js`.
fn default_shader_prop_value(prop: &ShaderProp) -> Value {
    match prop.kind {
        PropKind::Float => json!(prop.default),
        PropKind::Int | PropKind::Enum => json!(prop.default.round() as i64),
        PropKind::Bool => json!(prop.default != 0.0),
        PropKind::Vec2 => json!([prop.default, prop.default]),
        PropKind::Vec3 => json!([prop.default, prop.default, prop.default]),
        PropKind::Vec4 => json!([prop.default, prop.default, prop.default, prop.default]),
        PropKind::Color => json!({
            "r": prop.default,
            "g": prop.default,
            "b": prop.default,
            "a": prop.default,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_knows_both_module_kinds() {
        let registry = ModuleRegistry::builtin();
        assert_eq!(registry.kind("Bands"), Some(ModuleKind::Shader));
        assert_eq!(registry.kind("Ball"), Some(ModuleKind::TwoD));
        assert_eq!(registry.kind("Not A Module"), None);
        assert!(registry.shader_modules().len() >= 37);
        assert_eq!(registry.two_d_definitions().len(), 15);
    }

    #[test]
    fn two_d_meta_fields_match_the_js_descriptors() {
        let registry = ModuleRegistry::builtin();
        let ball = registry.meta_fields("Ball").expect("Ball is registered");
        assert_eq!(ball["name"], json!("Ball"));
        assert_eq!(ball["author"], json!("2xAA"));
        assert_eq!(ball["version"], json!("1.0.1"));
        assert_eq!(ball["type"], json!("2d"));
        assert_eq!(ball["audioFeatures"], json!(["zcr", "rms"]));
        assert!(ball.get("category").is_none());
        assert!(ball.get("previewWithOutput").is_none());

        // Text declares neither author nor version (`Text.js`), so the spread
        // leaves them out.
        let text = registry.meta_fields("Text").expect("Text is registered");
        assert!(text.get("author").is_none());
        assert!(text.get("version").is_none());

        // Effects-category modules carry `category`; the two JS descriptors
        // that set `previewWithOutput` keep it.
        let pixelate = registry
            .meta_fields("Pixelate")
            .expect("Pixelate is registered");
        assert_eq!(pixelate["category"], json!("effects"));
        assert_eq!(pixelate["previewWithOutput"], json!(true));
        assert!(
            registry
                .meta_fields("Smear")
                .unwrap()
                .get("previewWithOutput")
                .is_none()
        );
    }

    #[test]
    fn two_d_default_props_are_the_descriptor_defaults() {
        let registry = ModuleRegistry::builtin();
        let props = registry
            .default_props("Ball")
            .expect("Ball has prop defaults");
        assert_eq!(props["amount"], json!(10));
        assert_eq!(props["speed"], json!(2.0));
        assert_eq!(props["wrap"], json!(false));
        // The tween default is the raw palette control object, as in JS.
        assert!(props["color"]["data"].is_array());
        assert_eq!(props["color"]["duration"], json!(10000));
    }

    #[test]
    fn two_d_prop_schema_carries_the_js_coercion_flags() {
        let registry = ModuleRegistry::builtin();

        // Polygon.js:29-37 — `strict: true` clamps, `type: "int"` rounds.
        let stroke = registry
            .prop_schema("Polygon", "strokeWeight")
            .expect("Polygon declares strokeWeight");
        assert_eq!(stroke.type_, "int");
        assert_eq!(stroke.min, Some(1.0));
        assert_eq!(stroke.max, Some(20.0));
        assert!(stroke.strict);

        // Ball.js:38-48 — `size` is bounded but takes `abs`, not `strict`.
        let size = registry
            .prop_schema("Ball", "size")
            .expect("Ball declares size");
        assert!(!size.strict);
        assert!(size.abs);

        // A bounded prop with no flags stays unclamped.
        let speed = registry
            .prop_schema("Ball", "speed")
            .expect("Ball declares speed");
        assert!(!speed.strict && !speed.abs);
        assert_eq!(speed.max, Some(20.0));

        assert!(registry.prop_schema("Ball", "nope").is_none());
        assert!(registry.prop_schema("Not A Module", "amount").is_none());
    }

    #[test]
    fn create_two_d_builds_the_named_instance() {
        let registry = ModuleRegistry::builtin();
        let module = registry.create_two_d("Smear").expect("Smear is registered");
        assert_eq!(module.meta().name, "Smear");
        assert!(registry.create_two_d("Bands").is_none());
    }
}
