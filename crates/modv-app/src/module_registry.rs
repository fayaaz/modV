//! The combined module registry (M3 + M4): shader modules, the ported 2D
//! modules and the ported Point10 scenes.
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
//! M4 adds the Point10 scenes ([`modv_renderers::point10::all_scenes`]). They
//! are registered as **`meta.type: "2d"`** modules under their JS wrapper name
//! (`Point10 <Scene>`, `renderers/2d/modules/Point10*.js`) so Scene documents,
//! presets and the Gallery keep working unchanged, but they render through
//! their own scene path in the compositor (see [`ModuleKind::Point10`]). The
//! Gallery `category`, `audioFeatures` and prop schema come straight from each
//! scene's [`modv_renderers::point10::Point10Scene::meta`]/`props` report.
//!
//! ## Point10 prop binding
//!
//! [`modv_renderers::point10::Point10Scene`] deliberately has no prop binder:
//! the scene schemas live in `modv-renderers` while the resolved prop values
//! live in the engine store. The vendored scenes do expose an inherent
//! `apply_props(getter)` (the `sceneProps.js` binder), so
//! [`Point10SceneKind`] wraps the four concrete batch-1 scenes, delegates the
//! trait, and adds [`Point10SceneKind::apply_props`]. The compositor needs
//! that wrapper to push prop changes into a live scene; the trait-object form
//! the plan asks for is [`ModuleRegistry::create_point10`].
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
use modv_renderers::point10::chromatic::wave::ChromaticWave;
use modv_renderers::point10::scenes::particle_field::ParticleField;
use modv_renderers::point10::scenes::spiral_particles::SpiralParticles;
use modv_renderers::point10::scenes::waveform::Waveform;
use modv_renderers::point10::{
    Point10Scene, SceneMeta, SceneProp, ScenePropKind, all_scenes as all_point10_scenes,
};
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
    /// A `modv-renderers::point10` scene (own target texture + composite).
    ///
    /// Registered with `meta.type: "2d"` (the JS `createPoint10Module` meta) so
    /// Scene documents and presets resolve it, but rendered through the Point10
    /// path in the compositor, never the 2D canvas facade.
    Point10,
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

/// A registered Point10 scene: the metadata and prop schema the scene reports,
/// factored out of the GPU-backed instance.
///
/// Unlike [`TwoDDefinition`] there is no separate descriptor type: the scene's
/// [`SceneMeta`]/[`SceneProp`] values *are* the JS `meta`/`props` contract
/// (`adapter.js:857-871`), so they are stored verbatim.
#[derive(Clone, Debug, PartialEq)]
pub struct Point10Definition {
    /// `meta` (`name`, `author`, `version`, `module_type`, `category`,
    /// `audio_features`).
    pub meta: SceneMeta,
    /// The scene's own schema preceded by the responsiveness levers
    /// ([`modv_renderers::point10::scene_props_with_levers`]).
    pub props: Vec<SceneProp>,
}

/// The app-side wrapper around the concrete batch-1 Point10 scenes.
///
/// [`modv_renderers::point10::Point10Scene`] is the render contract and has no
/// prop binder, while every ported scene exposes an inherent
/// `apply_props(getter)` (the `sceneProps.js` binder). This enum keeps that
/// capability: it delegates every trait method to the live scene and adds
/// [`Point10SceneKind::apply_props`], which the compositor calls once per frame.
///
/// A new scene must be added here *and* to
/// [`modv_renderers::point10::all_scenes`]; `every_registered_scene_constructs`
/// fails when the two drift.
pub enum Point10SceneKind {
    /// `Point10 Chromatic Wave` (`chromatic/wave.rs`).
    ChromaticWave(ChromaticWave),
    /// `Point10 Particle Field` (`scenes/particle_field.rs`).
    ParticleField(ParticleField),
    /// `Point10 Spiral Particles` (`scenes/spiral_particles.rs`).
    SpiralParticles(SpiralParticles),
    /// `Point10 Waveform` (`scenes/waveform.rs`).
    Waveform(Waveform),
}

impl Point10SceneKind {
    /// Build a fresh scene for a registered wrapper name.
    ///
    /// `None` when `name` is not a ported scene — the compositor then skips the
    /// module exactly like an unregistered 2D module.
    pub fn create(name: &str) -> Option<Self> {
        Some(match name {
            "Point10 Chromatic Wave" => Self::ChromaticWave(ChromaticWave::new()),
            "Point10 Particle Field" => Self::ParticleField(ParticleField::new()),
            "Point10 Spiral Particles" => Self::SpiralParticles(SpiralParticles::new()),
            "Point10 Waveform" => Self::Waveform(Waveform::new()),
            _ => return None,
        })
    }

    /// Push the resolved modV props into the scene.
    ///
    /// The getter is the engine's prop bag (`modules.active[id].props`): each
    /// schema entry falls back to its default and clamps to its range, exactly
    /// like `sceneProps.js: readSceneProps` does once per frame
    /// (`adapter.js:797-800`).
    pub fn apply_props(&mut self, get: &mut dyn FnMut(&str) -> Option<f32>) {
        match self {
            Self::ChromaticWave(scene) => scene.apply_props(|key| get(key)),
            Self::ParticleField(scene) => scene.apply_props(|key| get(key)),
            Self::SpiralParticles(scene) => scene.apply_props(|key| get(key)),
            Self::Waveform(scene) => scene.apply_props(|key| get(key)),
        }
    }
}

/// Every trait method delegates to the concrete scene.
impl Point10Scene for Point10SceneKind {
    fn name(&self) -> &'static str {
        match self {
            Self::ChromaticWave(scene) => scene.name(),
            Self::ParticleField(scene) => scene.name(),
            Self::SpiralParticles(scene) => scene.name(),
            Self::Waveform(scene) => scene.name(),
        }
    }

    fn init(&mut self, ctx: &mut modv_renderers::point10::SceneContext) {
        match self {
            Self::ChromaticWave(scene) => scene.init(ctx),
            Self::ParticleField(scene) => scene.init(ctx),
            Self::SpiralParticles(scene) => scene.init(ctx),
            Self::Waveform(scene) => scene.init(ctx),
        }
    }

    fn resize(&mut self, width: u32, height: u32) {
        match self {
            Self::ChromaticWave(scene) => scene.resize(width, height),
            Self::ParticleField(scene) => scene.resize(width, height),
            Self::SpiralParticles(scene) => scene.resize(width, height),
            Self::Waveform(scene) => scene.resize(width, height),
        }
    }

    fn update(&mut self, dt: f32, audio: &modv_renderers::point10::audio_bridge::AudioBridge) {
        match self {
            Self::ChromaticWave(scene) => scene.update(dt, audio),
            Self::ParticleField(scene) => scene.update(dt, audio),
            Self::SpiralParticles(scene) => scene.update(dt, audio),
            Self::Waveform(scene) => scene.update(dt, audio),
        }
    }

    fn render(
        &mut self,
        encoder: &mut modv_render::wgpu::CommandEncoder,
        target: &modv_render::wgpu::TextureView,
    ) {
        match self {
            Self::ChromaticWave(scene) => scene.render(encoder, target),
            Self::ParticleField(scene) => scene.render(encoder, target),
            Self::SpiralParticles(scene) => scene.render(encoder, target),
            Self::Waveform(scene) => scene.render(encoder, target),
        }
    }

    fn dispose(&mut self) {
        match self {
            Self::ChromaticWave(scene) => scene.dispose(),
            Self::ParticleField(scene) => scene.dispose(),
            Self::SpiralParticles(scene) => scene.dispose(),
            Self::Waveform(scene) => scene.dispose(),
        }
    }

    fn props(&self) -> Vec<SceneProp> {
        match self {
            Self::ChromaticWave(scene) => scene.props(),
            Self::ParticleField(scene) => scene.props(),
            Self::SpiralParticles(scene) => scene.props(),
            Self::Waveform(scene) => scene.props(),
        }
    }

    fn meta(&self) -> SceneMeta {
        match self {
            Self::ChromaticWave(scene) => scene.meta(),
            Self::ParticleField(scene) => scene.meta(),
            Self::SpiralParticles(scene) => scene.meta(),
            Self::Waveform(scene) => scene.meta(),
        }
    }
}

impl std::fmt::Debug for Point10SceneKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Point10SceneKind")
            .field(&self.name())
            .finish()
    }
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

/// All three built-in registries, with the metadata the store and the
/// compositor need.
pub struct ModuleRegistry {
    shader: Vec<ShaderModule>,
    two_d: Vec<TwoDDefinition>,
    point10: Vec<Point10Definition>,
}

impl ModuleRegistry {
    /// The 37 built-in shader modules, the 15 ported 2D modules and the ported
    /// Point10 scenes.
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
        // Point10 scenes report their own meta/schema; no GPU work happens on
        // construction (pipelines and textures are built in `init`).
        let point10 = all_point10_scenes()
            .iter()
            .map(|scene| Point10Definition {
                meta: scene.meta(),
                props: scene.props(),
            })
            .collect();
        Self {
            shader,
            two_d,
            point10,
        }
    }

    /// The shader modules, in Gallery registration order.
    pub fn shader_modules(&self) -> &[ShaderModule] {
        &self.shader
    }

    /// The 2D module definitions, in registration order.
    pub fn two_d_definitions(&self) -> &[TwoDDefinition] {
        &self.two_d
    }

    /// The Point10 scene definitions, in `all_scenes()` order.
    pub fn point10_definitions(&self) -> &[Point10Definition] {
        &self.point10
    }

    /// Look up a shader module by its registration name (`$moduleName`).
    pub fn shader_module(&self, name: &str) -> Option<&ShaderModule> {
        self.shader.iter().find(|module| module.name == name)
    }

    /// Look up a 2D module definition by its registration name.
    pub fn two_d_definition(&self, name: &str) -> Option<&TwoDDefinition> {
        self.two_d.iter().find(|module| module.meta.name == name)
    }

    /// Look up a Point10 scene definition by its wrapper module name
    /// (`"Point10 Chromatic Wave"`).
    pub fn point10_definition(&self, name: &str) -> Option<&Point10Definition> {
        self.point10.iter().find(|scene| scene.meta.name == name)
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
        } else if self
            .point10_definitions()
            .iter()
            .any(|scene| scene.meta.name == name)
        {
            Some(ModuleKind::Point10)
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
            ModuleKind::Point10 => {
                let definition = self.point10_definition(name)?;
                // `createPoint10Module` always spreads all five meta fields
                // (`adapter.js:857-866`), so unlike the 2D path none of them is
                // optional here.
                let mut meta = Map::new();
                meta.insert("name".to_string(), json!(definition.meta.name));
                meta.insert("author".to_string(), json!(definition.meta.author));
                meta.insert("version".to_string(), json!(definition.meta.version));
                meta.insert("type".to_string(), json!(definition.meta.module_type));
                if !definition.meta.category.is_empty() {
                    meta.insert("category".to_string(), json!(definition.meta.category));
                }
                if !definition.meta.audio_features.is_empty() {
                    meta.insert(
                        "audioFeatures".to_string(),
                        json!(definition.meta.audio_features),
                    );
                }
                Some(meta)
            }
        }
    }

    /// The coercion schema for one of a module's props.
    ///
    /// Shader props never set `strict` (`renderers/shader/modules/*.js`), so
    /// their schemas keep `strict: false`; 2D props take `type`/`min`/`max`
    /// from [`TwoDProp`] and `strict`/`abs` from [`TWO_D_PROP_FLAGS`]; Point10
    /// props take both from the scene's [`SceneProp`] schema.
    pub fn prop_schema(&self, module_name: &str, prop: &str) -> Option<PropSchema> {
        if let Some(module) = self.shader_module(module_name) {
            let declared = module.props.iter().find(|declared| declared.name == prop)?;
            return Some(schema_for_shader(declared));
        }

        if let Some(definition) = self.two_d_definition(module_name) {
            let declared = definition
                .props
                .iter()
                .find(|declared| declared.name == prop)?;
            return Some(schema_for_two_d(definition.meta.name, declared));
        }

        let definition = self.point10_definition(module_name)?;
        let declared = definition
            .props
            .iter()
            .find(|declared| declared.name == prop)?;
        Some(schema_for_point10(declared))
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

        if let Some(definition) = self.two_d_definition(module_name) {
            return Some(two_d_default_props(&definition.props));
        }

        let definition = self.point10_definition(module_name)?;
        let mut props = Map::new();
        for prop in &definition.props {
            props.insert(prop.name.to_string(), default_scene_prop_value(prop));
        }
        Some(Value::Object(props))
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

    /// Build a fresh Point10 scene for `name`.
    ///
    /// Returns `None` when `name` is not a registered scene, matching
    /// [`ModuleRegistry::create_two_d`]. The concrete wrapper is returned (not
    /// a trait object) so the compositor can call
    /// [`Point10SceneKind::apply_props`]; it also implements [`Point10Scene`],
    /// so [`ModuleRegistry::create_point10`] hands out the trait-object form.
    pub fn create_point10_instance(&self, name: &str) -> Option<Point10SceneKind> {
        self.point10_definition(name)?;
        Point10SceneKind::create(name)
    }

    /// Build a fresh Point10 scene as the render-contract trait object.
    ///
    /// This is the `Box<dyn Point10Scene>` form of
    /// [`ModuleRegistry::create_point10_instance`]. The compositor itself uses
    /// the concrete wrapper because the trait has no prop binder (see the
    /// module docs); this form exists for generic consumers.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn create_point10(&self, name: &str) -> Option<Box<dyn Point10Scene>> {
        self.create_point10_instance(name)
            .map(|scene| Box::new(scene) as Box<dyn Point10Scene>)
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

/// Build a [`PropSchema`] from a Point10 scene prop declaration.
///
/// The JS scene schemas always declare `min`/`max` and clamp on read
/// (`sceneProps.js: readSceneProps`), so the engine marks the numeric props
/// `strict`: an out-of-range `modules/updateProp` (a hand-edited preset, an
/// input link) is stored already clamped instead of only being clamped when the
/// scene reads it. Booleans are `type: "bool"` and go through the plain
/// `!= false` read, exactly like `props.useAgc`.
fn schema_for_point10(prop: &SceneProp) -> PropSchema {
    PropSchema {
        type_: scene_prop_kind_name(prop.kind).to_string(),
        min: Some(f64::from(prop.min)),
        max: Some(f64::from(prop.max)),
        strict: prop.kind == ScenePropKind::Float,
        abs: false,
        default: Some(default_scene_prop_value(prop)),
    }
}

/// The `dataTypes` key for a Point10 prop kind.
fn scene_prop_kind_name(kind: ScenePropKind) -> &'static str {
    match kind {
        ScenePropKind::Float => "float",
        ScenePropKind::Bool => "bool",
    }
}

/// The JSON default for a Point10 prop declaration.
///
/// `SceneProp` stores the default as an `f32`; going through the shortest
/// round-trip decimal keeps the JSON readable and identical to the JS literals
/// (`0.05` stays `0.05`, not `0.05000000074505806`). Whole numbers are written
/// as floats (`1.0`) so every float prop has the same JSON number shape as the
/// shader and 2D default paths.
fn default_scene_prop_value(prop: &SceneProp) -> Value {
    match prop.kind {
        ScenePropKind::Bool => json!(prop.default != 0.0),
        ScenePropKind::Float => {
            let text = prop.default.to_string();
            let text = if text.contains(['.', 'e', 'E']) {
                text
            } else {
                format!("{text}.0")
            };
            serde_json::from_str(&text).unwrap_or_else(|_| json!(f64::from(prop.default)))
        }
    }
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

    // ————————————————— Point10 scenes (M4) —————————————————

    #[test]
    fn registry_knows_the_point10_scenes() {
        let registry = ModuleRegistry::builtin();
        assert_eq!(
            registry.kind("Point10 Chromatic Wave"),
            Some(ModuleKind::Point10)
        );
        assert_eq!(registry.kind("Point10 Waveform"), Some(ModuleKind::Point10));
        // Point10 names are their own kind, not 2D modules.
        assert_eq!(registry.kind("Ball"), Some(ModuleKind::TwoD));
        assert_eq!(registry.kind("Point10 Not A Scene"), None);
        assert_eq!(registry.point10_definitions().len(), 4);
    }

    #[test]
    fn point10_definitions_match_the_wrapper_modules() {
        let registry = ModuleRegistry::builtin();
        let names: Vec<&str> = registry
            .point10_definitions()
            .iter()
            .map(|definition| definition.meta.name)
            .collect();
        assert_eq!(
            names,
            vec![
                "Point10 Chromatic Wave",
                "Point10 Particle Field",
                "Point10 Spiral Particles",
                "Point10 Waveform",
            ]
        );

        for definition in registry.point10_definitions() {
            assert_eq!(definition.meta.author, "Point10");
            assert_eq!(definition.meta.version, "1.0.0");
            // The JS wrappers register as `2d` modules so Scene documents and
            // presets keep resolving them (`adapter.js:857-866`).
            assert_eq!(definition.meta.module_type, "2d");
            assert_eq!(definition.meta.audio_features, &["bass", "mid", "treble"]);
            assert!(!definition.props.is_empty());

            // The levers come first and the scene schema follows
            // (`adapter.js:868-871`).
            let levers: Vec<&str> = definition.props[..6].iter().map(|prop| prop.name).collect();
            assert_eq!(
                levers,
                vec![
                    "gain",
                    "smoothing",
                    "bassGain",
                    "midGain",
                    "trebleGain",
                    "useAgc"
                ]
            );
        }

        // Only the fullscreen 2D scene overrides the default category
        // (`Point10ChromaticWave.js:7`).
        let category = |name: &str| {
            registry
                .point10_definition(name)
                .expect("registered")
                .meta
                .category
        };
        assert_eq!(category("Point10 Chromatic Wave"), "2d-scenes");
        assert_eq!(category("Point10 Particle Field"), "3d-scenes");
        assert_eq!(category("Point10 Spiral Particles"), "3d-scenes");
        assert_eq!(category("Point10 Waveform"), "3d-scenes");
    }

    #[test]
    fn point10_meta_fields_match_the_js_wrapper() {
        let registry = ModuleRegistry::builtin();
        let meta = registry
            .meta_fields("Point10 Chromatic Wave")
            .expect("registered");
        assert_eq!(meta["name"], json!("Point10 Chromatic Wave"));
        assert_eq!(meta["author"], json!("Point10"));
        assert_eq!(meta["version"], json!("1.0.0"));
        assert_eq!(meta["type"], json!("2d"));
        assert_eq!(meta["category"], json!("2d-scenes"));
        assert_eq!(meta["audioFeatures"], json!(["bass", "mid", "treble"]));

        let waveform = registry
            .meta_fields("Point10 Waveform")
            .expect("registered");
        assert_eq!(waveform["category"], json!("3d-scenes"));
    }

    #[test]
    fn point10_default_props_are_the_scene_defaults() {
        let registry = ModuleRegistry::builtin();

        // `sceneProps.js: CHROMATIC_LINE_PROPS` + the levers.
        let props = registry
            .default_props("Point10 Chromatic Wave")
            .expect("registered");
        assert_eq!(props["gain"], json!(1.0));
        assert_eq!(props["smoothing"], json!(0.0));
        assert_eq!(props["bassGain"], json!(1.0));
        assert_eq!(props["useAgc"], json!(true));
        assert_eq!(props["distortion"], json!(0.05));
        assert_eq!(props["amplitude"], json!(1.0));
        assert_eq!(props["glow"], json!(1.0));

        let waveform = registry
            .default_props("Point10 Waveform")
            .expect("registered");
        assert_eq!(waveform["height"], json!(1.0));
        assert_eq!(waveform["speed"], json!(1.0));
        assert_eq!(waveform["opacity"], json!(1.0));

        // Every registered scene has a default for every declared prop.
        for definition in registry.point10_definitions() {
            let props = registry
                .default_props(definition.meta.name)
                .expect("defaults");
            for prop in &definition.props {
                assert!(
                    props.get(prop.name).is_some(),
                    "{} is missing a default for {}",
                    definition.meta.name,
                    prop.name
                );
            }
        }
    }

    #[test]
    fn point10_prop_schema_clamps_like_the_scene_binder() {
        let registry = ModuleRegistry::builtin();

        let gain = registry
            .prop_schema("Point10 Chromatic Wave", "gain")
            .expect("gain is declared");
        assert_eq!(gain.type_, "float");
        assert_eq!(gain.min, Some(0.0));
        assert_eq!(gain.max, Some(4.0));
        assert!(gain.strict);

        let use_agc = registry
            .prop_schema("Point10 Waveform", "useAgc")
            .expect("useAgc is declared");
        assert_eq!(use_agc.type_, "bool");
        assert!(!use_agc.strict);
        assert_eq!(use_agc.default, Some(json!(true)));

        assert!(
            registry
                .prop_schema("Point10 Waveform", "notAProp")
                .is_none()
        );
    }

    #[test]
    fn create_point10_builds_the_named_scene() {
        let registry = ModuleRegistry::builtin();

        let scene = registry
            .create_point10_instance("Point10 Waveform")
            .expect("Waveform is registered");
        assert_eq!(scene.name(), "Point10 Waveform");

        let boxed = registry
            .create_point10("Point10 Waveform")
            .expect("Waveform is registered");
        assert_eq!(boxed.name(), "Point10 Waveform");

        assert!(registry.create_point10("Bands").is_none());
        assert!(registry.create_point10_instance("Not A Scene").is_none());
    }

    #[test]
    fn every_registered_scene_constructs() {
        let registry = ModuleRegistry::builtin();
        for definition in registry.point10_definitions() {
            assert!(
                registry.create_point10(definition.meta.name).is_some(),
                "{} is registered but has no constructor in Point10SceneKind",
                definition.meta.name
            );
        }
    }
}
