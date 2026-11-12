//! Scene hover previews — the engine side of WS-8 ("Scene preview (hover)")
//! and WS-14.
//!
//! The Scenes grid hovers a cell and asks
//! `window.engine.requestScenePreview({ sceneId, rowIndex, width, height })`,
//! which reaches the engine as a `preview/scene` request. The supervisor
//! cannot render (it owns no GPU device), so it queues the request and the
//! window loop answers it through [`render_scene_preview`] after
//! `render_frame`; the reply is `{ width, height, pixels }` with `pixels` the
//! base64 of a tightly packed `width × height` RGBA8 frame.
//!
//! ## What one request renders
//!
//! The legacy worker rendered a hover from the Scene's **saved** Group/Module
//! snapshots (`worker/scene-preview-data.js`), never from the live Groups, so
//! hovering could not disturb the running composition. The port keeps that
//! contract:
//!
//! 1. [`Store::scene_preview_document`] resolves the Scene document (including
//!    unsaved/edited Scenes) and the current row-name mapping.
//! 2. [`build_scene_preview_state`] materialises **one isolated
//!    [`EngineState`]**: the single Group at `rowIndex` with fresh Group and
//!    Module ids, the registry's meta/prop defaults plus the snapshot's saved
//!    values (`modv_engine::scene_preview_data::get_scene_row_preview`). The
//!    live store, its modules and its inputs are never touched.
//! 3. That state renders through a private [`Compositor`] at the requested
//!    size — the same group/module compositing path the live loop uses, so a
//!    module's `alpha`/`compositeOperation` and every renderer kind
//!    (shader/2D/Point10/ISF) behave exactly like the composition. The Group
//!    is the cell's only layer, so it inherits nothing; it clears every
//!    request (transparent background, and a self-sampling 2D module sees
//!    frame one, like Gallery thumbnails) and composites at alpha 1/`normal`
//!    because the legacy preview painted the cell's Modules straight onto the
//!    preview canvas without applying the Group's own alpha/blend.
//! 4. `Compositor::read_main_rgba8` returns the tightly packed RGBA8 frame.
//!
//! ## Cache
//!
//! Slots are keyed by `(sceneId, rowIndex, width, height)` and kept in a small
//! LRU ([`MAX_CACHED_SCENE_PREVIEWS`]) so a 30 fps hover re-renders without
//! re-materialising the isolated state. Each slot also stores the Scene-list
//! revision it was built from ([`Store::scene_list_revision`]); a Scene edit,
//! import, removal or row rename bumps that revision, so the next request
//! rebuilds instead of showing a stale frame. Evicted slots dispose exactly
//! their own compositor instances (their ids are fresh per materialisation);
//! every other cached cell's shader/2D/Point10/ISF instances stay alive, and a
//! render keeps them out of the private compositor's prune (R-10).
//!
//! ## Cost
//!
//! A cold request pays the isolated state build plus the module-kind
//! initialisation (shader pipeline / 2D `init` / Point10 scene init / ISF
//! compile) exactly like a Gallery thumbnail. A warm request renders one frame
//! into the preview-sized targets and reads `width × height × 4` bytes back
//! (≤ 675 KiB at 480 × 270); there is no multi-frame history. The window loop
//! answers at most four preview requests per frame, shared with
//! `preview/module`, so a burst of hovers cannot stall presentation.

use std::sync::Arc;

use serde_json::{Map, Value, json};

use modv_engine::engine_state::{EngineState, GROUP_ENABLED, Group, InheritFrom, ModuleInstance};
use modv_engine::props::coerce_prop_value;
use modv_engine::scene_preview_data::get_scene_row_preview;
use modv_render::wgpu;

use crate::compositor::{Compositor, FrameInfo, RetainedInstances};
use crate::module_preview::PreviewError;
use crate::module_registry::ModuleRegistry;
use crate::store_ops::Store;

/// Largest edge a Scene preview may request.
///
/// The UI already caps a cell at 480 × 270 (`scenePreviewSize` in
/// `Scenes.vue`), so no request needs to exceed 480 on either edge; the cap
/// bounds the GPU targets and the read-back at 480² × 4 bytes.
pub const MAX_SCENE_PREVIEW_DIMENSION: u32 = 480;

/// Number of `(scene, row, width, height)` preview states kept alive.
///
/// A hover shows one cell at a time and the grid rows are short; four covers
/// back-and-forth hovering without accumulating isolated module instances.
pub const MAX_CACHED_SCENE_PREVIEWS: usize = 4;

/// Clamp one requested preview edge into `1..=MAX_SCENE_PREVIEW_DIMENSION`.
pub fn clamp_scene_preview_dimension(value: u32) -> u32 {
    value.clamp(1, MAX_SCENE_PREVIEW_DIMENSION)
}

/// Cache/debug key for one Scene preview slot.
///
/// The UI's runtime key is `sceneId:rowIndex`; the size joins it here because
/// the isolated state and its render targets are size-dependent.
#[cfg_attr(not(test), allow(dead_code))]
pub fn scene_preview_key(scene_id: &str, row_index: usize, width: u32, height: u32) -> String {
    format!("{scene_id}:{row_index}:{width}x{height}")
}

/// One cached isolated Scene preview and the revision it was built from.
struct ScenePreviewSlot {
    /// The Scene the slot was materialised from.
    scene_id: String,
    /// The grid row the slot renders.
    row_index: usize,
    /// Target width the slot was created for.
    width: u32,
    /// Target height the slot was created for.
    height: u32,
    /// [`Store::scene_list_revision`] at materialisation time.
    revision: u64,
    /// The isolated Group + Modules (never shared with the live store).
    state: EngineState,
}

impl ScenePreviewSlot {
    /// Whether this slot serves `scene_id`/`row` at the requested size.
    fn matches(&self, scene_id: &str, row_index: usize, width: u32, height: u32) -> bool {
        self.scene_id == scene_id
            && self.row_index == row_index
            && self.width == width
            && self.height == height
    }

    /// Add this slot's fresh Group/Module ids to `retained`.
    ///
    /// Every materialisation uses a fresh serial, so a slot's ids name exactly
    /// its own compositor instances; the preview keeps them alive while
    /// another cell renders.
    fn retain_instances(&self, retained: &mut RetainedInstances) {
        for group in &self.state.groups {
            retained.retain_group(group.id.clone());
        }
        for id in self.state.modules.keys() {
            retained.retain_module(id.clone());
        }
    }
}

/// The isolated Scene-preview renderer, owned by the compositor.
///
/// It owns a private [`Compositor`] so the preview gets the live loop's exact
/// group/module compositing semantics without sharing any texture, pipeline
/// slot or module instance with the main composition.
pub struct ScenePreviews {
    device: wgpu::Device,
    queue: wgpu::Queue,
    registry: Arc<ModuleRegistry>,
    /// Created on the first request so a headless run never allocates it.
    compositor: Option<Box<Compositor>>,
    /// Cache, most recently used first.
    slots: Vec<ScenePreviewSlot>,
    /// Isolated states built so far (tests/diagnostics).
    materialisations: u64,
    /// Monotonic serial used to keep every materialisation's ids fresh.
    serial: u64,
}

impl ScenePreviews {
    /// Build the preview renderer. No GPU targets are allocated until the
    /// first request.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, registry: Arc<ModuleRegistry>) -> Self {
        Self {
            device: device.clone(),
            queue: queue.clone(),
            registry,
            compositor: None,
            slots: Vec::new(),
            materialisations: 0,
            serial: 0,
        }
    }

    /// Drop the private compositor (releasing every module instance and
    /// texture) and every isolated state.
    pub fn reset(&mut self) {
        self.slots.clear();
        if let Some(mut compositor) = self.compositor.take() {
            compositor.reset();
        }
    }

    /// Number of cached isolated states (test/diagnostic use).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn cached_slots(&self) -> usize {
        self.slots.len()
    }

    /// Isolated states materialised so far (test/diagnostic use).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn materialisations(&self) -> u64 {
        self.materialisations
    }

    /// Module instances the private compositor currently holds, across every
    /// cached cell (test/diagnostic use).
    ///
    /// Rendering one cell must not change this beyond that cell's own
    /// instances: the cached cells' shader/2D/Point10/ISF instances survive a
    /// hover move (R-10).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn live_instances(&self) -> usize {
        self.compositor
            .as_ref()
            .map(|compositor| compositor.live_module_instances())
            .unwrap_or(0)
    }

    /// Render one frame of the Scene cell `(scene_id, row_index)` at
    /// `width × height` and return tightly packed RGBA8 pixels.
    ///
    /// The size is clamped with [`clamp_scene_preview_dimension`]. `revision`
    /// is the store's Scene-list revision: an entry built from an older
    /// revision is discarded, so an edited Scene never shows a stale frame.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        scene: &Value,
        row_names: &[String],
        scene_id: &str,
        row_index: usize,
        width: u32,
        height: u32,
        revision: u64,
        frame: &FrameInfo,
    ) -> Result<Vec<u8>, PreviewError> {
        let width = clamp_scene_preview_dimension(width);
        let height = clamp_scene_preview_dimension(height);

        let index = match self.slots.iter().position(|slot| {
            slot.matches(scene_id, row_index, width, height) && slot.revision == revision
        }) {
            Some(index) => {
                if index > 0 {
                    let slot = self.slots.remove(index);
                    self.slots.insert(0, slot);
                }
                0
            }
            None => self.create_slot(
                scene, row_names, scene_id, row_index, width, height, revision,
            )?,
        };

        let device = self.device.clone();
        let queue = self.queue.clone();
        let registry = Arc::clone(&self.registry);
        // Keep every cached cell's instances: `prune` would otherwise dispose
        // the ones the state being rendered does not reference, i.e. all the
        // other cached cells (R-10a).
        let mut retained = RetainedInstances::new();
        for slot in &self.slots {
            slot.retain_instances(&mut retained);
        }
        let compositor = self
            .compositor
            .get_or_insert_with(|| Box::new(Compositor::with_registry(&device, &queue, registry)));
        compositor.resize(width, height);
        compositor
            .render_frame_retaining(&self.slots[index].state, frame, &retained)
            .map_err(|error| PreviewError::Render(error.to_string()))?;
        compositor
            .read_main_rgba8()
            .map_err(|error| PreviewError::Render(error.to_string()))
    }

    /// Materialise (and front-insert) the isolated state for one Scene cell,
    /// evicting the least recently used entry when the cache is full.
    #[allow(clippy::too_many_arguments)]
    fn create_slot(
        &mut self,
        scene: &Value,
        row_names: &[String],
        scene_id: &str,
        row_index: usize,
        width: u32,
        height: u32,
        revision: u64,
    ) -> Result<usize, PreviewError> {
        // A slot for the same key but an older revision is replaced below;
        // drop it first so the LRU never holds two entries for one key.
        if let Some(position) = self
            .slots
            .iter()
            .position(|slot| slot.matches(scene_id, row_index, width, height))
        {
            self.slots.remove(position);
        }

        let serial = self.serial;
        let state = build_scene_preview_state(&self.registry, scene, row_index, row_names, serial)
            .ok_or_else(|| PreviewError::SceneRowEmpty {
                scene_id: scene_id.to_string(),
                row: row_index,
            })?;

        self.serial = self.serial.wrapping_add(1);
        self.materialisations = self.materialisations.wrapping_add(1);

        let evicted = if self.slots.len() >= MAX_CACHED_SCENE_PREVIEWS {
            self.slots.pop()
        } else {
            None
        };
        // Dispose exactly the evicted cell's instances (they are keyed by its
        // fresh materialisation ids). Resetting the private compositor here
        // would invalidate every still-cached cell's shader/2D/Point10/ISF
        // instances and make an alternating hover re-init modules on each poll
        // (R-10b).
        if let Some(evicted) = evicted
            && let Some(compositor) = &mut self.compositor
        {
            let mut ids = RetainedInstances::new();
            evicted.retain_instances(&mut ids);
            compositor.dispose_instances(&ids);
        }

        self.slots.insert(
            0,
            ScenePreviewSlot {
                scene_id: scene_id.to_string(),
                row_index,
                width,
                height,
                revision,
                state,
            },
        );
        Ok(0)
    }
}

/// Resolve a Scene from the store and render one hover-preview frame.
///
/// This is the whole engine side of a `preview/scene` request: it looks the
/// Scene up in the store (so unsaved/edited Scenes are visible), refuses an
/// unknown id with [`PreviewError::SceneNotFound`], and delegates to
/// [`Compositor::render_scene_preview`].
pub fn render_scene_preview(
    compositor: &mut Compositor,
    store: &Store,
    scene_id: &str,
    row_index: usize,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, PreviewError> {
    let Some((scene, row_names)) = store.scene_preview_document(scene_id) else {
        return Err(PreviewError::SceneNotFound(scene_id.to_string()));
    };

    compositor.render_scene_preview(
        &scene,
        &row_names,
        scene_id,
        row_index,
        width,
        height,
        store.scene_list_revision(),
    )
}

/// Build the isolated state a Scene hover preview renders.
///
/// Only the Group at `row_index` is materialised, with the Module snapshots it
/// points at (`get_scene_row_preview`, the port of `scene-preview-data.js`).
/// Every id is prefixed with `generation` so two materialisations can never
/// share a shader slot, ISF renderer or Point10 scene instance.
///
/// Returns `None` for an empty/out-of-range row, a Group whose Module ids all
/// lack snapshots, or a row whose Modules are all unregistered — the caller
/// answers `scene_row_empty`.
pub fn build_scene_preview_state(
    registry: &ModuleRegistry,
    scene: &Value,
    row_index: usize,
    row_names: &[String],
    generation: u64,
) -> Option<EngineState> {
    let preview = get_scene_row_preview(scene, row_index, row_names)?;

    let group_id = format!("scene-preview:{generation}:group");
    let mut group = Group {
        id: group_id,
        name: preview
            .group
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("Scene preview")
            .to_string(),
        hidden: false,
        modules: Vec::new(),
        enabled: GROUP_ENABLED,
        // The legacy preview painted the cell's Modules straight onto the
        // preview canvas: the Group's own alpha/compositeOperation never
        // applied, so the isolated Group composites at full opacity/normal.
        alpha: 1.0,
        // The cell is the only layer: nothing above it to inherit.
        inherit: false,
        inherit_from: InheritFrom::Previous(-1),
        pipeline: false,
        // Clear every request: a transparent background, and a 2D module that
        // samples the canvas sees frame one (like Gallery thumbnails).
        clearing: true,
        composite_operation: "normal".to_string(),
        scene_id: None,
        scene_row: Some(row_index),
        transition_ghost: None,
        draw_to_canvas_id: None,
        alpha_input_id: None,
        enabled_input_id: None,
        clearing_input_id: None,
        inherit_input_id: None,
        composite_operation_input_id: None,
        pipeline_input_id: None,
        extra: Map::new(),
    };

    let mut state = EngineState::new();
    for (index, snapshot) in preview.modules.iter().enumerate() {
        let Some(module) = build_scene_preview_module(registry, snapshot, generation, index) else {
            // An unregistered Module is skipped, exactly like
            // `scene-preview.js` (no renderer means nothing to draw).
            continue;
        };
        group.modules.push(module.id.clone());
        state.modules.insert(module.id.clone(), module);
    }

    if group.modules.is_empty() {
        return None;
    }

    state.groups.push(group);
    Some(state)
}

/// Build one isolated preview [`ModuleInstance`] from a Scene module snapshot.
///
/// The registry supplies the meta fields and prop defaults; the snapshot's
/// `meta`/`props`/`data` are layered on top (`createPreviewModule`,
/// `scene-preview-data.js:49-93`). Props are coerced through the registry
/// schema so a preview renders the same values the live apply would.
fn build_scene_preview_module(
    registry: &ModuleRegistry,
    snapshot: &Value,
    generation: u64,
    index: usize,
) -> Option<ModuleInstance> {
    let object = snapshot.as_object()?;
    let module_name = object
        .get("$moduleName")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
        .or_else(|| {
            object
                .get("meta")
                .and_then(|meta| meta.get("name"))
                .and_then(Value::as_str)
        })?
        .to_string();

    // Unregistered Module names have no renderer; skip them like the JS
    // preview does when the definition lookup fails.
    let mut meta = registry.meta_fields(&module_name)?;
    meta.insert("name".to_string(), json!(module_name));
    meta.insert("alpha".to_string(), json!(1));
    meta.insert("enabled".to_string(), json!(true));
    meta.insert("compositeOperation".to_string(), json!("normal"));
    if let Some(overrides) = object.get("meta").and_then(Value::as_object) {
        for (key, value) in overrides {
            meta.insert(key.clone(), value.clone());
        }
    }
    meta.insert("isGallery".to_string(), json!(true));
    meta.insert("scenePreview".to_string(), json!(true));

    let mut props = registry
        .default_props(&module_name)
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    if let Some(overrides) = object.get("props").and_then(Value::as_object) {
        for (prop, value) in overrides {
            let coerced = registry
                .prop_schema(&module_name, prop)
                .map(|schema| coerce_prop_value(&schema, value))
                .unwrap_or_else(|| value.clone());
            props.insert(prop.clone(), coerced);
        }
    }

    Some(ModuleInstance {
        id: format!("scene-preview:{generation}:module:{index}:{module_name}"),
        module_name,
        meta: Value::Object(meta),
        props: Value::Object(props),
        data: object.get("data").cloned().unwrap_or_else(|| json!({})),
        dollar_props: json!({}),
        status: Vec::new(),
        extra: Map::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use modv_engine::scene_rows::normalize_row_names;
    use modv_render::{Engine, EngineConfig};

    use crate::compositor::FrameInfo;
    use crate::module_preview::base64_encode;

    /// A tiny valid Scene document: one `Foreground` row with `Ball`.
    fn ball_scene() -> Value {
        json!({
            "id": "scene-ball",
            "name": "Ball Scene",
            "enabled": true,
            "groups": [
                {
                    "id": "snapshot-group",
                    "name": "Foreground",
                    "modules": ["snapshot-module"],
                    "alpha": 0.4,
                    "clearing": false,
                    "compositeOperation": "multiply",
                }
            ],
            "modules": {
                "snapshot-module": {
                    "$id": "snapshot-module",
                    "$moduleName": "Ball",
                    "meta": {"name": "Ball", "type": "2d", "alpha": 0.75},
                    "props": {"size": 12},
                    "data": {}
                }
            }
        })
    }

    fn try_engine() -> Option<Engine> {
        match Engine::new(&EngineConfig {
            force_fallback_adapter: true,
        }) {
            Ok(engine) => Some(engine),
            Err(error) => {
                eprintln!("skipping Scene preview pixel test: no adapter ({error})");
                None
            }
        }
    }

    fn frame_info() -> FrameInfo {
        FrameInfo::default()
    }

    fn lit_pixels(pixels: &[u8]) -> usize {
        pixels.chunks_exact(4).filter(|pixel| pixel[3] > 0).count()
    }

    /// Materialise a seeded `Store` with the default Scene list.
    fn seeded_store() -> Store {
        let mut store = Store::new();
        store
            .dispatch("scenes/ensureDefaultScenes", json!({}))
            .expect("default Scenes seed");
        store
    }

    fn scene_by_name<'a>(store: &'a Store, name: &str) -> &'a Value {
        store
            .scenes_state()
            .scenes
            .iter()
            .find(|scene| scene.get("name").and_then(Value::as_str) == Some(name))
            .unwrap_or_else(|| panic!("default Scene \"{name}\" is not seeded"))
    }

    #[test]
    fn dimensions_clamp_to_one_and_the_maximum() {
        assert_eq!(clamp_scene_preview_dimension(0), 1);
        assert_eq!(clamp_scene_preview_dimension(1), 1);
        assert_eq!(clamp_scene_preview_dimension(240), 240);
        assert_eq!(clamp_scene_preview_dimension(480), 480);
        assert_eq!(clamp_scene_preview_dimension(481), 480);
        assert_eq!(clamp_scene_preview_dimension(u32::MAX), 480);
    }

    #[test]
    fn cache_key_includes_the_scene_row_and_size() {
        assert_eq!(
            scene_preview_key("scene-1", 2, 240, 135),
            "scene-1:2:240x135"
        );
        assert_ne!(
            scene_preview_key("scene-1", 2, 240, 135),
            scene_preview_key("scene-1", 3, 240, 135)
        );
    }

    #[test]
    fn builds_one_isolated_group_and_its_modules() {
        let registry = ModuleRegistry::builtin();
        let state = build_scene_preview_state(&registry, &ball_scene(), 0, &[], 7)
            .expect("the Ball row materialises");

        assert_eq!(state.groups.len(), 1);
        let group = &state.groups[0];
        assert_eq!(group.name, "Foreground");
        assert_eq!(group.modules.len(), 1);
        // The preview Group is transparent and non-inheriting; its own saved
        // alpha/blend are intentionally not applied (legacy semantics).
        assert_eq!(group.alpha, 1.0);
        assert_eq!(group.composite_operation, "normal");
        assert!(group.clearing);
        assert!(!group.inherit);
        assert!(group.inherit_from.is_previous());

        let module = state
            .modules
            .get(&group.modules[0])
            .expect("the module instance is in the isolated state");
        assert_eq!(module.module_name, "Ball");
        assert_eq!(module.id, "scene-preview:7:module:0:Ball");
        // Registry meta defaults, then the snapshot's overrides; gallery
        // flags keep every preview off the live composition.
        assert_eq!(module.meta["type"], json!("2d"));
        assert_eq!(module.meta["alpha"], json!(0.75));
        assert_eq!(module.meta["enabled"], json!(true));
        assert_eq!(module.meta["isGallery"], json!(true));
        assert_eq!(module.meta["scenePreview"], json!(true));
        // Defaults plus the saved prop value (coerced through the schema).
        assert_eq!(module.props["size"].as_f64(), Some(12.0));
    }

    #[test]
    fn materialisations_use_fresh_ids() {
        let registry = ModuleRegistry::builtin();
        let first =
            build_scene_preview_state(&registry, &ball_scene(), 0, &[], 1).expect("first build");
        let second =
            build_scene_preview_state(&registry, &ball_scene(), 0, &[], 2).expect("second build");

        assert_ne!(first.groups[0].id, second.groups[0].id);
        assert_ne!(first.groups[0].modules[0], second.groups[0].modules[0]);
    }

    #[test]
    fn empty_or_unknown_rows_materialise_nothing() {
        let registry = ModuleRegistry::builtin();
        let scene = json!({
            "groups": [{"id": "hole", "name": "Foreground", "modules": []}],
            "modules": {}
        });

        assert!(build_scene_preview_state(&registry, &scene, 0, &[], 0).is_none());
        assert!(build_scene_preview_state(&registry, &scene, 9, &[], 0).is_none());
        // A row whose only Module is not registered has nothing to draw.
        assert!(
            build_scene_preview_state(
                &registry,
                &json!({
                    "groups": [{"name": "Foreground", "modules": ["m"]}],
                    "modules": {"m": {"$moduleName": "definitely not registered"}}
                }),
                0,
                &[],
                0
            )
            .is_none()
        );
    }

    #[test]
    fn named_rows_resolve_through_the_row_name_mapping() {
        let registry = ModuleRegistry::builtin();
        let scene = json!({
            "groups": [
                {"name": "Effects", "modules": ["m-effects"]},
                {"name": "Background", "modules": ["m-background"]},
            ],
            "modules": {
                "m-effects": {"$moduleName": "Ball", "props": {}},
                "m-background": {"$moduleName": "Ball", "props": {}},
            }
        });
        let row_names = normalize_row_names(Some(&json!(["", "", "Background"])));

        let state = build_scene_preview_state(&registry, &scene, 2, &row_names, 3)
            .expect("the named row resolves");
        assert_eq!(state.groups[0].name, "Background");
    }

    #[test]
    fn unknown_scene_ids_do_not_resolve() {
        let store = Store::new();
        assert!(store.scene_preview_document("missing").is_none());
    }

    #[test]
    fn scene_list_revision_advances_on_scene_mutations() {
        let mut store = Store::new();
        let before = store.scene_list_revision();

        store
            .dispatch("scenes/ensureDefaultScenes", json!({}))
            .expect("default Scenes seed");
        let seeded = store.scene_list_revision();
        assert!(seeded > before, "seeding bumps the revision");

        store
            .dispatch("scenes/setRowNames", json!({"rowNames": ["Effects"]}))
            .expect("row names update");
        assert!(
            store.scene_list_revision() > seeded,
            "a row-name change invalidates previews too"
        );
    }

    #[test]
    fn scene_preview_renders_a_seeded_default_scene_at_both_sizes() {
        let Some(engine) = try_engine() else {
            return;
        };
        let store = seeded_store();
        // Seeded Scenes carry a generated id; resolve it through the store
        // exactly like the window loop does for a request.
        let id = scene_id(scene_by_name(&store, "Lone Chrome"));

        for (width, height) in [(64_u32, 36_u32), (240, 135)] {
            let mut compositor = Compositor::new(&engine.device, &engine.queue);
            let pixels = render_scene_preview(&mut compositor, &store, &id, 0, width, height)
                .expect("Lone Chrome preview renders");

            assert_eq!(
                pixels.len(),
                (width * height * 4) as usize,
                "tightly packed RGBA8 at {width}x{height}"
            );
            assert!(
                lit_pixels(&pixels) > 0,
                "Lone Chrome drew nothing at {width}x{height}"
            );
        }
    }

    /// The generated id of a seeded Scene document.
    fn scene_id(scene: &Value) -> String {
        scene
            .get("id")
            .and_then(Value::as_str)
            .expect("seeded Scenes carry an id")
            .to_string()
    }

    #[test]
    fn scene_preview_keeps_the_background_transparent() {
        let Some(engine) = try_engine() else {
            return;
        };
        let registry = Arc::new(ModuleRegistry::builtin());
        let device = engine.device.clone();
        let queue = engine.queue.clone();
        let mut previews = ScenePreviews::new(&device, &queue, Arc::clone(&registry));

        let pixels = previews
            .render(
                &ball_scene(),
                &[],
                "scene-ball",
                0,
                64,
                64,
                0,
                &frame_info(),
            )
            .expect("Ball preview renders");

        assert_eq!(pixels.len(), (64 * 64 * 4) as usize);
        assert!(
            pixels.chunks_exact(4).any(|pixel| pixel[3] == 0),
            "the cell background must stay transparent"
        );
        assert!(lit_pixels(&pixels) > 0, "Ball drew nothing");
    }

    #[test]
    fn scene_preview_reports_unknown_scenes_and_empty_rows() {
        let Some(engine) = try_engine() else {
            return;
        };
        let mut compositor = Compositor::new(&engine.device, &engine.queue);
        let store = seeded_store();
        let scene = scene_by_name(&store, "Lone Chrome");
        let id = scene_id(scene);

        assert!(matches!(
            render_scene_preview(&mut compositor, &store, "definitely missing", 0, 32, 32),
            Err(PreviewError::SceneNotFound(_))
        ));
        assert_eq!(
            PreviewError::SceneNotFound("x".to_string()).code(),
            "scene_not_found"
        );

        let error = render_scene_preview(&mut compositor, &store, &id, 99, 32, 32)
            .expect_err("an unknown row cannot render");
        assert!(matches!(error, PreviewError::SceneRowEmpty { .. }));
        assert_eq!(error.code(), "scene_row_empty");
    }

    #[test]
    fn scene_preview_clamps_requested_sizes() {
        let Some(engine) = try_engine() else {
            return;
        };
        let registry = Arc::new(ModuleRegistry::builtin());
        let device = engine.device.clone();
        let queue = engine.queue.clone();
        let mut previews = ScenePreviews::new(&device, &queue, registry);

        let tiny = previews
            .render(&ball_scene(), &[], "scene-ball", 0, 0, 0, 0, &frame_info())
            .expect("clamped preview renders");
        assert_eq!(tiny.len(), 4, "0x0 clamps to 1x1");

        let big = previews
            .render(
                &ball_scene(),
                &[],
                "scene-ball",
                0,
                u32::MAX,
                900,
                0,
                &frame_info(),
            )
            .expect("oversized preview renders");
        assert_eq!(
            big.len(),
            (MAX_SCENE_PREVIEW_DIMENSION * MAX_SCENE_PREVIEW_DIMENSION * 4) as usize,
            "both edges clamp to the 480 cap"
        );
    }

    #[test]
    fn scene_preview_cache_reuses_the_isolated_state_and_invalidates_on_edits() {
        let Some(engine) = try_engine() else {
            return;
        };
        let mut store = seeded_store();
        let scene = scene_by_name(&store, "Lone Chrome").clone();
        let id = scene_id(&scene);
        let mut compositor = Compositor::new(&engine.device, &engine.queue);

        render_scene_preview(&mut compositor, &store, &id, 0, 32, 18).expect("first render");
        render_scene_preview(&mut compositor, &store, &id, 0, 32, 18).expect("warm render");
        assert_eq!(
            compositor.scene_previews().materialisations(),
            1,
            "a warm request reuses the cached isolated state"
        );
        assert_eq!(compositor.scene_previews().cached_slots(), 1);

        // Editing the Scene (here: upserting the same document through the
        // load action) bumps the Scene-list revision, so the cache rebuilds
        // instead of serving a stale frame.
        let before = store.scene_list_revision();
        store
            .dispatch(
                "scenes/loadSceneFromJson",
                json!({"scene": scene, "persist": false}),
            )
            .expect("Scene upsert");
        assert!(store.scene_list_revision() > before);

        render_scene_preview(&mut compositor, &store, &id, 0, 32, 18).expect("post-edit render");
        assert_eq!(
            compositor.scene_previews().materialisations(),
            2,
            "a revision bump materialises a fresh state"
        );
    }

    #[test]
    fn scene_preview_cache_is_bounded_and_reset_disposes_it() {
        let Some(engine) = try_engine() else {
            return;
        };
        let registry = Arc::new(ModuleRegistry::builtin());
        let device = engine.device.clone();
        let queue = engine.queue.clone();
        let mut previews = ScenePreviews::new(&device, &queue, registry);

        for index in 0..(MAX_CACHED_SCENE_PREVIEWS + 2) {
            let mut scene = ball_scene();
            scene["id"] = json!(format!("scene-{index}"));
            previews
                .render(
                    &scene,
                    &[],
                    &format!("scene-{index}"),
                    0,
                    16,
                    16,
                    0,
                    &frame_info(),
                )
                .expect("preview renders");
        }

        assert_eq!(previews.cached_slots(), MAX_CACHED_SCENE_PREVIEWS);
        assert_eq!(
            previews.materialisations(),
            (MAX_CACHED_SCENE_PREVIEWS + 2) as u64
        );

        previews.reset();
        assert_eq!(previews.cached_slots(), 0);
        assert!(previews.compositor.is_none());
    }

    #[test]
    fn cache_keeps_the_other_cells_instances_while_a_hover_moves() {
        let Some(engine) = try_engine() else {
            return;
        };
        let registry = Arc::new(ModuleRegistry::builtin());
        let device = engine.device.clone();
        let queue = engine.queue.clone();
        let mut previews = ScenePreviews::new(&device, &queue, registry);

        previews
            .render(&ball_scene(), &[], "scene-a", 0, 16, 16, 0, &frame_info())
            .expect("cell A renders");
        let one_cell = previews.live_instances();
        assert!(one_cell > 0, "cell A allocated a module instance");

        // Rendering cell B must not dispose A's instance (the old prune keyed
        // off the rendered state only, so it did exactly that).
        previews
            .render(&ball_scene(), &[], "scene-b", 0, 16, 16, 0, &frame_info())
            .expect("cell B renders");
        assert_eq!(
            previews.live_instances(),
            one_cell * 2,
            "cell A's instance survives cell B's render"
        );

        // Hovering back is a cache hit that reuses A's instance.
        previews
            .render(&ball_scene(), &[], "scene-a", 0, 16, 16, 0, &frame_info())
            .expect("cell A renders warm");
        assert_eq!(previews.materialisations(), 2, "A is not re-materialised");
        assert_eq!(
            previews.live_instances(),
            one_cell * 2,
            "A reuses the instance it materialised"
        );
    }

    #[test]
    fn eviction_disposes_only_the_evicted_cells_instances() {
        let Some(engine) = try_engine() else {
            return;
        };
        let registry = Arc::new(ModuleRegistry::builtin());
        let device = engine.device.clone();
        let queue = engine.queue.clone();
        let mut previews = ScenePreviews::new(&device, &queue, registry);

        for index in 0..(MAX_CACHED_SCENE_PREVIEWS + 1) {
            let mut scene = ball_scene();
            scene["id"] = json!(format!("scene-{index}"));
            previews
                .render(
                    &scene,
                    &[],
                    &format!("scene-{index}"),
                    0,
                    16,
                    16,
                    0,
                    &frame_info(),
                )
                .expect("preview renders");
        }

        assert_eq!(previews.cached_slots(), MAX_CACHED_SCENE_PREVIEWS);
        // One instance per cached cell: eviction disposed the evicted cell's
        // instance, not the whole private compositor's set.
        assert_eq!(
            previews.live_instances(),
            MAX_CACHED_SCENE_PREVIEWS,
            "evicting one slot must keep the other cached cells' instances"
        );
    }

    #[test]
    fn scene_preview_pixels_base64_round_trip_through_the_response_shape() {
        // The supervisor sends `base64_encode(pixels)` and the renderer's
        // `atob` decoder expects padded RFC 4648 text; the module preview's
        // vectors already cover the alphabet, this pins the frame length.
        let pixels = vec![0u8; (2 * 2 * 4) as usize];
        let encoded = base64_encode(&pixels);
        assert_eq!(encoded, "AAAAAAAAAAAAAAAAAAAAAA==");
        assert_eq!(base64_encode(&pixels).len(), pixels.len().div_ceil(3) * 4);
    }

    #[test]
    fn warm_scene_preview_cost_is_reported() {
        let Some(engine) = try_engine() else {
            return;
        };
        let store = seeded_store();
        let id = scene_id(scene_by_name(&store, "Lone Chrome"));
        let mut compositor = Compositor::new(&engine.device, &engine.queue);

        // Cold: materialise + first Point10 init.
        render_scene_preview(&mut compositor, &store, &id, 0, 240, 135).expect("cold render");

        const FRAMES: u32 = 20;
        let started = std::time::Instant::now();
        for _ in 0..FRAMES {
            render_scene_preview(&mut compositor, &store, &id, 0, 240, 135).expect("warm render");
        }
        let per_frame = started.elapsed().as_secs_f64() * 1000.0 / f64::from(FRAMES);
        eprintln!("scene preview: {per_frame:.2} ms/frame warm at 240x135 (lavapipe/GPU)");

        // The spec target is a few milliseconds; keep a generous ceiling so a
        // slow shared GPU does not make the suite flaky while still catching a
        // path that re-materialises or reads back far too much per request.
        assert!(
            per_frame < 100.0,
            "warm Scene preview costs {per_frame:.1} ms/frame"
        );
        assert_eq!(
            compositor.scene_previews().materialisations(),
            1,
            "warm renders never re-materialise"
        );
    }
}
