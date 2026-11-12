//! The versioned default Scene definitions (the data port).
//!
//! Source of truth: `src/renderer/src/application/worker/store/default-scenes.js`
//! (4067 lines, data plus ~420 lines of seeding/migration logic). Per WS-8
//! ("Default Scenes") the data is a **data** port: the array is stored as
//! `data/default-scenes.json` and embedded with `include_str!`, so both the
//! Rust engine and JavaScript tooling can read it.
//!
//! The logic ported here:
//!
//! - [`get_default_scene_names_up_to`] (`default-scenes.js:171-177`)
//! - [`get_scenes_to_seed`] (`default-scenes.js:179-197`) — only newly
//!   introduced Scenes, matched by name, so existing installs receive later
//!   batches without duplicates.
//! - [`get_default_scenes_to_remove`] (`default-scenes.js:233-286`) — seeded
//!   retired Scenes (`REMOVED_DEFAULT_SCENES`).
//! - [`get_default_scenes_to_replace`] (`default-scenes.js:288-417`) — rebuild
//!   Scenes that use a removed Module, apply the rename table, and refresh the
//!   definitions that changed in the immediately previous defaults version
//!   (`REFRESHED_DEFAULT_SCENES`).
//!
//! The version-keyed rename/removal table for *future* defaults bumps lives in
//! [`crate::default_scene_migrations`].
//!
//! ## Data shape
//!
//! A default Scene definition is **not** a Scene document: it has no `id` and
//! its `modules` are definitions (`name`, `props`, `meta`) rather than module
//! instances. The scenes store builds the real document on first run
//! (`scenes.js:328-380`, `buildSceneSnapshot`) and generates ids then. The
//! JSON therefore carries `name`, `introducedVersion`, `minSeconds`,
//! `maxSeconds`, `groups` and optional `inputLinks`.
//!
//! Line numbers refer to the tree at commit `8a512b6e` (branch `rust-hybrid`).

use std::collections::HashSet;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Bumped when a new batch of default Scenes ships
/// (`default-scenes.js:39`; `tests/unit/default-scenes.test.js:85-87` pins it).
pub const DEFAULTS_VERSION: i64 = 12;

/// The version of the original six Scenes (`default-scenes.js:38`).
pub const LEGACY_DEFAULTS_VERSION: i64 = 1;

/// Modules removed from the defaults whose Scenes must be rebuilt
/// (`default-scenes.js:44`).
pub const REMOVED_DEFAULT_SCENE_MODULES: &[&str] = &["Point10 Cube Floor"];

/// Default Scenes retired without a replacement (`default-scenes.js:49-58`).
pub const REMOVED_DEFAULT_SCENES: &[&str] = &[
    "Ferrofluid Pulse",
    "Kaleidoscope Cathedral",
    "Slit Scan Horizon",
    "Spiral Particles",
    "Spectral Bloom",
    "Fractured Kaleidoscope",
    "Broken Mirror Ball",
    "Roto Zoom Riot",
];

/// Scenes whose definitions changed in the current defaults version and are
/// rebuilt in place for installs on the previous version
/// (`default-scenes.js:109-159`).
pub const REFRESHED_DEFAULT_SCENES: &[&str] = &[
    "Mycelium Bloom",
    "Chromatic Waveform",
    "Geometric Tunnel",
    "Burning Mycelium Circle",
    "Chromatic Cascade",
    "Smoke Tunnel",
    "Digital Black Hole",
    "Mirror Chamber",
    "Solar Flare",
    "Water Ripple Dreams",
    "Waveform Drive",
    "Rolling Fire Bloom",
    "Strobe Riot",
    "Prism Reliquary",
    "Spiral Static",
    "Ultimate Drift",
    "Glitch Bloom Cathedral",
    "Hex Bloom",
    "VHS Reliquary",
    "It Is Coming",
    "It Twirls At Night",
    "Around Plus Aurora",
    "Around Water Garden",
    "Attractor Bloom",
    "Attractor Void",
    "Boxy Boulevard",
    "Boxy Arcade",
    "DVD Afterglow",
    "DVD Nightmare",
    "Light Tunnel Runner",
    "Light Tunnel Choir",
    "Square Tunnel Signal",
    "Wormhole Descent",
    "Wormhole Cathedral",
    "ASCII Sunbeam",
    "Terminal Bloom",
    "Halftone Harbor",
    "Crosshatch Cipher",
    "Dither District",
    "Mosaic Meridian",
    "Circuit Ritual",
    "Hexagon Halo",
    "Etched Aurora",
    "Brickwork Beat",
    "Digital Crystal Tunnel",
    "MBOX Light Tunnel",
    "Rave.exe",
    "Soft Bands",
    "Quiet Bloom",
];

/// One default Scene definition.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SceneDocument {
    /// Display name; unique in the default set.
    pub name: String,
    /// The defaults version that introduced this Scene.
    #[serde(rename = "introducedVersion")]
    pub introduced_version: i64,
    /// Preferred minimum dwell, in seconds (the Scene timing override).
    #[serde(
        rename = "minSeconds",
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::engine_state::serialize_minimal_option_f64"
    )]
    pub min_seconds: Option<f64>,
    /// Preferred maximum dwell, in seconds.
    #[serde(
        rename = "maxSeconds",
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::engine_state::serialize_minimal_option_f64"
    )]
    pub max_seconds: Option<f64>,
    /// The Group stack, base layer first.
    #[serde(default)]
    pub groups: Vec<SceneGroup>,
    /// Input controls pinned to audio before the automatic links are created.
    #[serde(rename = "inputLinks", default, skip_serializing_if = "Vec::is_empty")]
    pub input_links: Vec<SceneInputLink>,
    /// Unknown keys, kept verbatim for round-trip fidelity.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// One Group in a default Scene definition.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SceneGroup {
    /// Group name (`Background`, `Foreground`, `Effects`, ...).
    pub name: String,
    /// Whether the Group inherits the frame above it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherit: Option<bool>,
    /// Whether the Group clears its target before drawing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clearing: Option<bool>,
    /// Group opacity.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::engine_state::serialize_minimal_option_f64"
    )]
    pub alpha: Option<f64>,
    /// Composite operation token.
    #[serde(
        rename = "compositeOperation",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub composite_operation: Option<String>,
    /// Module definitions in draw order.
    #[serde(default)]
    pub modules: Vec<SceneModuleDefinition>,
    /// Unknown keys, kept verbatim.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// One Module definition inside a default Scene Group.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SceneModuleDefinition {
    /// The Module registry name (`Bloom`, `Point10 Mycelium`, ...).
    pub name: String,
    /// Prop overrides applied on top of the registered defaults. Absent when
    /// the definition only sets `meta`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub props: Option<Map<String, Value>>,
    /// Meta overrides (`alpha`, `compositeOperation`, ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Map<String, Value>>,
    /// Unknown keys, kept verbatim.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// One audio-pinned control in a default Scene definition.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SceneInputLink {
    /// The Group the control lives on.
    pub group: String,
    /// The Module inside the Group, when the control is a prop.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module: Option<String>,
    /// `alpha`, `enabled`, `compositeOperation` or a numeric prop name.
    pub control: String,
    /// Meyda feature (`bass`, `mid`, `treble`, `rms`, ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feature: Option<String>,
    /// `"tween"` for BPM-driven controls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Link range lower bound.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::engine_state::serialize_minimal_option_f64"
    )]
    pub min: Option<f64>,
    /// Link range upper bound.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::engine_state::serialize_minimal_option_f64"
    )]
    pub max: Option<f64>,
    /// Feature smoothing amount.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::engine_state::serialize_minimal_option_f64"
    )]
    pub smoothing: Option<f64>,
    /// BPM subdivision for a tween link.
    #[serde(
        rename = "bpmDivision",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub bpm_division: Option<i64>,
    /// Unknown keys, kept verbatim.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl SceneDocument {
    /// The Group with this name, if any.
    pub fn group(&self, name: &str) -> Option<&SceneGroup> {
        self.groups.iter().find(|group| group.name == name)
    }

    /// Every Module name in the Scene, in draw order.
    pub fn module_names(&self) -> Vec<&str> {
        self.groups
            .iter()
            .flat_map(|group| group.modules.iter().map(|module| module.name.as_str()))
            .collect()
    }

    /// Module names in one Group (empty when the Group is missing).
    pub fn modules_in_group(&self, name: &str) -> Vec<&str> {
        self.group(name)
            .map(|group| {
                group
                    .modules
                    .iter()
                    .map(|module| module.name.as_str())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The input link pinned to `module`/`control`, if any.
    pub fn link(&self, module: &str, control: &str) -> Option<&SceneInputLink> {
        self.input_links
            .iter()
            .find(|link| link.module.as_deref() == Some(module) && link.control == control)
    }
}

/// A column rename in the defaults (`default-scenes.js:60-100`).
struct SceneRename {
    old_name: &'static str,
    new_name: &'static str,
    requires_removed_module: bool,
}

/// `RENAMED_DEFAULT_SCENES` (`default-scenes.js:60-100`).
const RENAMED_DEFAULT_SCENES: &[SceneRename] = &[
    SceneRename {
        old_name: "Cube Floor Drive",
        new_name: "Waveform Drive",
        requires_removed_module: true,
    },
    SceneRename {
        old_name: "VHS Afterglow",
        new_name: "Digital Black Hole",
        requires_removed_module: false,
    },
    SceneRename {
        old_name: "It Comes At Night",
        new_name: "It Twirls At Night",
        requires_removed_module: false,
    },
    SceneRename {
        old_name: "Neon Grid",
        new_name: "Smoke Tunnel",
        requires_removed_module: false,
    },
    SceneRename {
        old_name: "Chromatic Sphere",
        new_name: "Chromatic Waveform",
        requires_removed_module: false,
    },
    SceneRename {
        old_name: "Twirl Tunnel",
        new_name: "Golden Spiral Tunnel",
        requires_removed_module: false,
    },
    SceneRename {
        old_name: "Twirl Reactor",
        new_name: "Reactor Glow",
        requires_removed_module: false,
    },
    SceneRename {
        old_name: "Twirl Grid",
        new_name: "Polygon Grid",
        requires_removed_module: false,
    },
    SceneRename {
        old_name: "Twirled Feedback",
        new_name: "Feedback Bloom",
        requires_removed_module: false,
    },
];

/// A retired Scene the caller should drop.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneRemoval {
    /// The existing Scene id (opaque; any JSON value the store holds).
    pub id: Value,
    /// The existing Scene name.
    pub name: String,
}

/// A seeded Scene whose payload should be rebuilt from the current
/// definition while keeping the existing id/name.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneReplacement {
    /// The existing Scene id.
    pub id: Value,
    /// The existing Scene name.
    pub name: String,
    /// The current default definition to rebuild from.
    pub definition: &'static SceneDocument,
}

/// What [`get_default_scenes_to_replace`] wants the caller to do.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReplacementPlan {
    /// Rebuild these Scenes in place (same id/name).
    pub replacements: Vec<SceneReplacement>,
    /// Retired/renamed Scenes to remove.
    pub removals: Vec<SceneRemoval>,
    /// New definitions to seed in place of a renamed Scene.
    pub additions: Vec<&'static SceneDocument>,
}

/// `sceneKey` (`default-scenes.js:161-164`): trim + lower-case.
pub fn scene_key(name: &str) -> String {
    name.trim().to_lowercase()
}

/// The embedded default Scene definitions, parsed once.
pub fn default_scenes() -> &'static [SceneDocument] {
    static DEFAULT_SCENES: OnceLock<Vec<SceneDocument>> = OnceLock::new();

    DEFAULT_SCENES
        .get_or_init(|| {
            serde_json::from_str::<Vec<SceneDocument>>(include_str!("../data/default-scenes.json"))
                .expect("embedded data/default-scenes.json is a valid Scene list")
        })
        .as_slice()
}

/// The default definition with this name (case-insensitive), if any
/// (`getDefaultDefinition`, `default-scenes.js:217-221`).
pub fn get_default_definition(name: &str) -> Option<&'static SceneDocument> {
    let key = scene_key(name);

    default_scenes()
        .iter()
        .find(|scene| scene_key(&scene.name) == key)
}

/// `getIntroducedVersion` (`default-scenes.js:166-169`): a missing/invalid
/// version counts as the legacy version.
fn introduced_version(scene: &SceneDocument) -> i64 {
    scene.introduced_version
}

/// `getDefaultSceneNamesUpTo` (`default-scenes.js:171-177`).
pub fn get_default_scene_names_up_to(version: i64) -> Vec<String> {
    default_scenes()
        .iter()
        .filter(|scene| introduced_version(scene) <= version)
        .map(|scene| scene.name.clone())
        .collect()
}

/// `getScenesToSeed` (`default-scenes.js:179-197`).
///
/// `defaults_version` is the version the install last seeded (`0` for a fresh
/// install, [`LEGACY_DEFAULTS_VERSION`] for an install predating the version
/// field). Only Scenes introduced after it that are neither already seeded nor
/// already present by name are returned; retired Scenes are absent from the
/// data, so they can never be offered.
pub fn get_scenes_to_seed(
    defaults_version: i64,
    seeded_names: &[String],
    existing_names: &[String],
) -> Vec<&'static SceneDocument> {
    let seeded: HashSet<String> = seeded_names.iter().map(|name| scene_key(name)).collect();
    let existing: HashSet<String> = existing_names.iter().map(|name| scene_key(name)).collect();

    default_scenes()
        .iter()
        .filter(|scene| {
            let key = scene_key(&scene.name);

            introduced_version(scene) > defaults_version
                && !seeded.contains(&key)
                && !existing.contains(&key)
        })
        .collect()
}

/// `sceneUsesRemovedModule` (`default-scenes.js:199-215`): any serialised
/// Module whose `$moduleName` or `meta.name` is in the removed set.
fn scene_uses_removed_module(scene: &Value, removed: &HashSet<&str>) -> bool {
    let Some(modules) = scene.get("modules").and_then(Value::as_object) else {
        return false;
    };

    modules.values().any(|module| {
        if !module.is_object() {
            return false;
        }

        let name = module
            .get("$moduleName")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
            .or_else(|| {
                module
                    .get("meta")
                    .and_then(|meta| meta.get("name"))
                    .and_then(Value::as_str)
                    .filter(|name| !name.is_empty())
            });

        name.is_some_and(|name| removed.contains(name))
    })
}

fn removal_from(scene: &Value) -> SceneRemoval {
    SceneRemoval {
        id: scene.get("id").cloned().unwrap_or(Value::Null),
        name: scene
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    }
}

/// `getDefaultScenesToRemove` (`default-scenes.js:233-286`).
///
/// `seeded_names` of `None` means "do not filter" (the JS `undefined` case);
/// passing the list only removes Scenes that were seeded from the defaults, so
/// a user Scene that happens to share a retired name is preserved. Nothing is
/// removed once the install is on [`DEFAULTS_VERSION`].
pub fn get_default_scenes_to_remove(
    defaults_version: i64,
    scenes: &[Value],
    seeded_names: Option<&[String]>,
) -> Vec<SceneRemoval> {
    get_default_scenes_to_remove_with(
        defaults_version,
        scenes,
        seeded_names,
        REMOVED_DEFAULT_SCENES,
    )
}

/// [`get_default_scenes_to_remove`] with an explicit retired-name list.
pub fn get_default_scenes_to_remove_with(
    defaults_version: i64,
    scenes: &[Value],
    seeded_names: Option<&[String]>,
    removed_names: &[&str],
) -> Vec<SceneRemoval> {
    let mut result = Vec::new();

    if defaults_version >= DEFAULTS_VERSION {
        return result;
    }

    let names: HashSet<String> = removed_names.iter().map(|name| scene_key(name)).collect();

    if names.is_empty() {
        return result;
    }

    let seeded = seeded_names.map(|names| {
        names
            .iter()
            .map(|name| scene_key(name))
            .collect::<HashSet<String>>()
    });
    let mut seen: HashSet<String> = HashSet::new();

    for scene in scenes {
        let Some(name) = scene.get("name").and_then(Value::as_str) else {
            continue;
        };

        let key = scene_key(name);

        if !names.contains(&key) || seen.contains(&key) {
            continue;
        }

        if let Some(seeded) = &seeded {
            if !seeded.contains(&key) {
                continue;
            }
        }

        seen.insert(key);
        result.push(removal_from(scene));
    }

    result
}

/// `getDefaultScenesToReplace` (`default-scenes.js:288-417`).
///
/// - Scenes in the rename table are removed + re-added under their new name
///   (Cube Floor Drive only when it still references the removed Module).
/// - Scenes that still reference a removed Module are rebuilt in place.
/// - When the install is on the immediately previous defaults version, the
///   [`REFRESHED_DEFAULT_SCENES`] definitions are rebuilt in place as well.
///
/// `seeded_names` of `None` means "do not filter"; anything else only touches
/// Scenes seeded from the defaults, so user edits are preserved.
pub fn get_default_scenes_to_replace(
    defaults_version: i64,
    scenes: &[Value],
    removed_module_names: Option<&[&str]>,
    seeded_names: Option<&[String]>,
) -> ReplacementPlan {
    get_default_scenes_to_replace_with(
        defaults_version,
        scenes,
        removed_module_names.unwrap_or(REMOVED_DEFAULT_SCENE_MODULES),
        seeded_names,
        REFRESHED_DEFAULT_SCENES,
    )
}

/// [`get_default_scenes_to_replace`] with explicit removed-module and refresh
/// name lists.
pub fn get_default_scenes_to_replace_with(
    defaults_version: i64,
    scenes: &[Value],
    removed_module_names: &[&str],
    seeded_names: Option<&[String]>,
    refresh_names: &[&str],
) -> ReplacementPlan {
    let mut result = ReplacementPlan::default();

    if defaults_version >= DEFAULTS_VERSION {
        return result;
    }

    let removed: HashSet<&str> = removed_module_names
        .iter()
        .copied()
        .filter(|name| !name.is_empty())
        .collect();
    let seeded = seeded_names.map(|names| {
        names
            .iter()
            .map(|name| scene_key(name))
            .collect::<HashSet<String>>()
    });
    let is_seeded = |key: &str| seeded.as_ref().is_none_or(|seeded| seeded.contains(key));

    // First occurrence wins, matching the JS `existingByKey` map.
    let mut existing_by_key: Vec<(String, &Value)> = Vec::new();

    for scene in scenes {
        let Some(name) = scene.get("name").and_then(Value::as_str) else {
            continue;
        };

        let key = scene_key(name);

        if !existing_by_key.iter().any(|(existing, _)| *existing == key) {
            existing_by_key.push((key, scene));
        }
    }

    let find_existing = |key: &str| -> Option<&Value> {
        existing_by_key
            .iter()
            .find(|(existing, _)| existing == key)
            .map(|(_, scene)| *scene)
    };

    let mut consumed_keys: HashSet<String> = HashSet::new();

    for rename in RENAMED_DEFAULT_SCENES {
        let key = scene_key(rename.old_name);
        let Some(existing) = find_existing(&key) else {
            continue;
        };

        if !is_seeded(&key) {
            continue;
        }

        if rename.requires_removed_module
            && (removed.is_empty() || !scene_uses_removed_module(existing, &removed))
        {
            continue;
        }

        let Some(definition) = get_default_definition(rename.new_name) else {
            continue;
        };

        result.removals.push(removal_from(existing));
        result.additions.push(definition);
        consumed_keys.insert(key);
    }

    if !removed.is_empty() {
        for definition in default_scenes() {
            let key = scene_key(&definition.name);

            if consumed_keys.contains(&key) {
                continue;
            }

            let Some(existing) = find_existing(&key) else {
                continue;
            };

            if !scene_uses_removed_module(existing, &removed) {
                continue;
            }

            result.replacements.push(SceneReplacement {
                id: existing.get("id").cloned().unwrap_or(Value::Null),
                name: existing
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                definition,
            });
        }
    }

    // Rebuild the Scenes whose definitions changed in this version. Only
    // installs that seeded the immediately previous defaults set are
    // refreshed, so a long-neglected library is not silently rewritten.
    let refresh: HashSet<String> = refresh_names.iter().map(|name| scene_key(name)).collect();

    if defaults_version >= DEFAULTS_VERSION - 1 && !refresh.is_empty() {
        let mut replaced_keys: HashSet<String> = result
            .replacements
            .iter()
            .map(|replacement| scene_key(&replacement.name))
            .collect();
        let removed_keys: HashSet<String> = result
            .removals
            .iter()
            .map(|removal| scene_key(&removal.name))
            .collect();

        for definition in default_scenes() {
            let key = scene_key(&definition.name);

            if !refresh.contains(&key)
                || replaced_keys.contains(&key)
                || removed_keys.contains(&key)
                || consumed_keys.contains(&key)
            {
                continue;
            }

            let Some(existing) = find_existing(&key) else {
                continue;
            };

            if !is_seeded(&key) {
                continue;
            }

            result.replacements.push(SceneReplacement {
                id: existing.get("id").cloned().unwrap_or(Value::Null),
                name: existing
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                definition,
            });
            replaced_keys.insert(key);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    //! Port of `tests/unit/default-scenes.test.js` (Vitest) plus data-integrity
    //! checks for the JSON conversion.
    use super::*;
    use serde_json::json;

    const GROUP_NAMES: [&str; 3] = ["Background", "Foreground", "Effects"];

    fn batch(version: i64) -> Vec<&'static SceneDocument> {
        default_scenes()
            .iter()
            .filter(|scene| scene.introduced_version == version)
            .collect()
    }

    fn scene_by_name(name: &str) -> &'static SceneDocument {
        default_scenes()
            .iter()
            .find(|scene| scene.name == name)
            .unwrap_or_else(|| panic!("missing default scene {name}"))
    }

    fn has_link(scene: &SceneDocument, module: &str) -> bool {
        scene
            .input_links
            .iter()
            .any(|link| link.module.as_deref() == Some(module))
    }

    fn seeded(name: &str) -> Value {
        json!({"id": format!("{name}-id"), "name": name, "modules": {}})
    }

    // ————————————————— data integrity —————————————————

    #[test]
    fn bumps_the_defaults_version_to_12() {
        // default-scenes.test.js:85-87
        assert_eq!(DEFAULTS_VERSION, 12);
        assert_eq!(LEGACY_DEFAULTS_VERSION, 1);
    }

    #[test]
    fn ships_at_least_54_default_scenes_with_unique_names() {
        // default-scenes.test.js:89-96
        assert!(default_scenes().len() >= 54);

        let keys: Vec<String> = default_scenes()
            .iter()
            .map(|scene| scene_key(&scene.name))
            .collect();

        assert_eq!(keys.iter().collect::<HashSet<_>>().len(), keys.len());
        assert!(keys.iter().all(|key| !key.is_empty()));
    }

    #[test]
    fn every_scene_has_a_name_version_and_groups() {
        // Data-port integrity: default definitions have no `id` (the scenes
        // store generates one when it builds the snapshot), so the check is
        // name + introducedVersion + a non-empty Group stack.
        for scene in default_scenes() {
            assert!(!scene.name.trim().is_empty());
            assert!(scene.introduced_version >= 1);
            assert!(!scene.groups.is_empty(), "{} has no groups", scene.name);
            assert!(
                scene
                    .groups
                    .iter()
                    .all(|group| !group.name.trim().is_empty())
            );
            assert!(
                scene
                    .groups
                    .iter()
                    .flat_map(|group| &group.modules)
                    .all(|module| !module.name.trim().is_empty())
            );
        }
    }

    #[test]
    fn the_embedded_json_round_trips_through_the_typed_model() {
        let raw: Vec<Value> =
            serde_json::from_str(include_str!("../data/default-scenes.json")).unwrap();
        let typed: Vec<Value> = default_scenes()
            .iter()
            .map(|scene| serde_json::to_value(scene).unwrap())
            .collect();

        for (index, (typed_scene, raw_scene)) in typed.iter().zip(raw.iter()).enumerate() {
            if typed_scene != raw_scene {
                let path = first_difference(typed_scene, raw_scene, format!("scene[{index}]"));
                panic!("typed default scenes diverge from the embedded JSON at {path}");
            }
        }
        assert_eq!(typed.len(), raw.len());
    }

    fn first_difference(left: &Value, right: &Value, path: String) -> String {
        match (left, right) {
            (Value::Object(left_map), Value::Object(right_map)) => {
                for (key, left_value) in left_map {
                    match right_map.get(key) {
                        Some(right_value) => {
                            if left_value != right_value {
                                return first_difference(
                                    left_value,
                                    right_value,
                                    format!("{path}.{key}"),
                                );
                            }
                        }
                        None => return format!("{path}.{key} (missing on the right)"),
                    }
                }
                for key in right_map.keys() {
                    if !left_map.contains_key(key) {
                        return format!("{path}.{key} (missing in the typed model)");
                    }
                }
                path
            }
            (Value::Array(left_items), Value::Array(right_items)) => {
                for (index, (left_item, right_item)) in
                    left_items.iter().zip(right_items.iter()).enumerate()
                {
                    if left_item != right_item {
                        return first_difference(left_item, right_item, format!("{path}[{index}]"));
                    }
                }
                if left_items.len() != right_items.len() {
                    return format!(
                        "{path} (array length {} vs {})",
                        left_items.len(),
                        right_items.len()
                    );
                }
                path
            }
            _ => format!("{path}: {left:?} != {right:?}"),
        }
    }

    #[test]
    fn keeps_the_original_scenes_on_their_original_introduced_version() {
        // default-scenes.test.js:98-110
        let legacy: Vec<&SceneDocument> = default_scenes()
            .iter()
            .filter(|scene| scene.introduced_version < 3)
            .collect();

        assert!(legacy.len() >= 15);
        assert!(
            legacy
                .iter()
                .all(|scene| scene.introduced_version == 1 || scene.introduced_version == 2)
        );
    }

    #[test]
    fn adds_at_least_nine_scenes_in_the_version_3_batch() {
        // default-scenes.test.js:112-114
        assert!(batch(3).len() >= 9);
    }

    #[test]
    fn adds_at_least_twelve_varied_point10_scenes_in_the_version_4_batch() {
        // default-scenes.test.js:116-119
        assert!(batch(4).len() >= 12);
    }

    #[test]
    fn uses_the_previously_unused_point10_scenes_in_the_version_4_batch() {
        // default-scenes.test.js:120-136
        let unused = [
            "Point10 Around Plus",
            "Point10 Attractor Particles",
            "Point10 Boxy",
            "Point10 DVD Room",
            "Point10 It Is Coming For You",
            "Point10 Light Tunnel",
            "Point10 Square Tunnel",
            "Point10 Wormhole",
        ];
        let names: HashSet<&str> = batch(4)
            .iter()
            .flat_map(|scene| scene.module_names())
            .collect();

        for name in unused {
            assert!(names.contains(name), "version 4 batch misses {name}");
        }
    }

    #[test]
    fn does_not_use_any_removed_or_retired_module() {
        // default-scenes.test.js:138-144, 284-290, 511-517, 630-644
        let banned = [
            "Point10 Ring Of Fire",
            "Point10 Cube Floor",
            "Point10 Disco Ball",
            "Radial Replicate",
            "RGB Halftone",
            "Dirty Lens",
            "Webcam",
            "Bands",
            "Kaleidoscope",
            "Kaleidoscope Tile",
            "Waveform",
            "Point10 Waveform",
            "Bar",
        ];

        for name in banned {
            let offenders: Vec<&str> = default_scenes()
                .iter()
                .filter(|scene| scene.module_names().contains(&name))
                .map(|scene| scene.name.as_str())
                .collect();

            assert!(offenders.is_empty(), "{name} used by {offenders:?}");
        }
    }

    #[test]
    fn pins_some_version_4_alpha_controls_to_bass() {
        // default-scenes.test.js:146-173
        let with_alpha_links: Vec<&SceneDocument> = batch(4)
            .into_iter()
            .filter(|scene| scene.input_links.iter().any(|link| link.control == "alpha"))
            .collect();

        assert!(with_alpha_links.len() >= 8);

        for scene in with_alpha_links {
            for link in scene
                .input_links
                .iter()
                .filter(|link| link.control == "alpha")
            {
                assert!(GROUP_NAMES.contains(&link.group.as_str()), "{}", scene.name);
                assert!(link.feature.is_some(), "{}", scene.name);
                assert!(
                    link.min.unwrap_or(0.0) < link.max.unwrap_or(0.0),
                    "{}",
                    scene.name
                );
                assert!(scene.group(&link.group).is_some(), "{}", scene.name);

                if let Some(module) = &link.module {
                    assert!(
                        scene
                            .modules_in_group(&link.group)
                            .contains(&module.as_str()),
                        "{} misses {module}",
                        scene.name
                    );
                }
            }
        }
    }

    #[test]
    fn adds_a_varied_version_5_filter_batch_with_ascii_art() {
        // default-scenes.test.js:175-192
        let filters = [
            "ASCII Art",
            "CMYK Halftone",
            "Color Invert",
            "Color Posterize",
            "Dither-Bayer",
            "Dot Screen",
            "Edge Trace",
            "Edges",
            "Emboss",
            "Pixellate",
            "RGB Halftone",
        ];
        let v5 = batch(5);
        assert!(v5.len() >= 8);

        let ascii_scenes = v5
            .iter()
            .filter(|scene| scene.module_names().contains(&"ASCII Art"))
            .count();
        assert!(ascii_scenes >= 2);

        let used: HashSet<&str> = v5
            .iter()
            .flat_map(|scene| scene.module_names())
            .filter(|name| filters.contains(name))
            .collect();
        assert!(used.len() >= 8);
        assert!(used.contains("ASCII Art"));
    }

    #[test]
    fn adds_a_version_6_digital_crystal_tunnel_scene() {
        // default-scenes.test.js:194-208
        let scene = batch(6)
            .into_iter()
            .find(|scene| {
                scene
                    .modules_in_group("Background")
                    .contains(&"digital-crystal-tunnel")
            })
            .expect("Digital Crystal Tunnel");

        assert_eq!(scene.name, "Digital Crystal Tunnel");
        assert!(scene.min_seconds.unwrap() < scene.max_seconds.unwrap());
        for group in GROUP_NAMES {
            assert!(scene.group(group).is_some(), "{group}");
        }
    }

    #[test]
    fn adds_a_version_6_mbox_light_tunnel_scene_with_audio_reactive_layers() {
        // default-scenes.test.js:210-235
        let scene = scene_by_name("MBOX Light Tunnel");

        assert_eq!(scene.introduced_version, 6);
        assert!(scene.min_seconds.unwrap() < scene.max_seconds.unwrap());
        assert!(scene.modules_in_group("Background").contains(&"MBOX3"));
        assert!(
            scene
                .modules_in_group("Foreground")
                .contains(&"Point10 Light Tunnel")
        );

        let alpha_link = scene
            .input_links
            .iter()
            .find(|link| {
                link.group == "Foreground"
                    && link.module.as_deref() == Some("Point10 Light Tunnel")
                    && link.control == "alpha"
                    && link
                        .feature
                        .as_deref()
                        .is_some_and(|feature| !feature.is_empty())
            })
            .expect("Foreground alpha link");

        assert!(alpha_link.min.unwrap() < alpha_link.max.unwrap());
    }

    #[test]
    fn drives_the_mbox_hue_with_a_beat_tween_and_scale_with_bass() {
        // default-scenes.test.js:237-282
        let scene = scene_by_name("MBOX Light Tunnel");
        let mbox = scene
            .group("Background")
            .and_then(|group| group.modules.iter().find(|module| module.name == "MBOX3"))
            .expect("MBOX3");

        let props = mbox.props.as_ref().expect("MBOX3 props");
        assert!(props["dthresh"].as_f64().unwrap() <= 0.1);
        assert!(props["speed"].as_f64().unwrap() <= 30.0);

        let links: Vec<&SceneInputLink> = scene
            .input_links
            .iter()
            .filter(|link| link.group == "Background" && link.module.as_deref() == Some("MBOX3"))
            .collect();

        let scale = links
            .iter()
            .find(|link| link.control == "SCALE")
            .expect("SCALE link");
        assert_ne!(scale.source.as_deref(), Some("tween"));
        assert_eq!(scale.feature.as_deref(), Some("bass"));
        assert!(scale.min.unwrap() < scale.max.unwrap());

        let hue = links
            .iter()
            .find(|link| link.control == "dhue")
            .expect("dhue link");
        assert_eq!(hue.source.as_deref(), Some("tween"));
        assert!(hue.min.unwrap() < hue.max.unwrap());

        assert!(links.iter().all(|link| link.control != "speed"));
    }

    #[test]
    fn renames_cube_floor_drive_to_waveform_drive() {
        // default-scenes.test.js:292-305
        let scene = scene_by_name("Waveform Drive");
        assert_eq!(scene.introduced_version, 2);
        assert!(
            !default_scenes()
                .iter()
                .any(|scene| scene.name == "Cube Floor Drive")
        );
        assert!(scene.modules_in_group("Background").is_empty());
    }

    #[test]
    fn adds_the_version_7_circular_mountain_scene() {
        // default-scenes.test.js:307-359
        assert!(batch(7).len() >= 1);

        let scene = scene_by_name("Circular Mountain");
        assert_eq!(scene.introduced_version, 7);
        assert!(scene.min_seconds.unwrap() < scene.max_seconds.unwrap());

        for group in GROUP_NAMES {
            assert!(scene.group(group).is_some(), "{group}");
        }

        assert_eq!(
            scene.modules_in_group("Background"),
            vec!["fractal-mountain-tunnel"]
        );
        let foreground = scene.modules_in_group("Foreground");
        for module in ["Circular Screen", "Corner Color Tint", "badtv"] {
            assert!(foreground.contains(&module), "Foreground misses {module}");
        }
        assert_eq!(scene.modules_in_group("Effects"), vec!["Bloom"]);

        let speed = scene.link("fractal-mountain-tunnel", "speed").unwrap();
        assert_eq!(speed.group, "Background");
        assert_eq!(speed.feature.as_deref(), Some("bass"));
        assert_eq!(speed.min, Some(0.0));
        assert_eq!(speed.max, Some(1.0));

        let alpha = scene.link("Corner Color Tint", "alpha").unwrap();
        assert_eq!(alpha.group, "Foreground");
        assert_eq!(alpha.feature.as_deref(), Some("rms"));
        assert_eq!(alpha.min, Some(0.0));
        assert_eq!(alpha.max, Some(2.0));

        let scan = scene.link("badtv", "scanLineIntensity").unwrap();
        assert_eq!(scan.feature.as_deref(), Some("treble"));
    }

    #[test]
    fn leaves_empty_effects_rows_where_radial_replicate_was_removed() {
        // default-scenes.test.js:377-384
        for name in ["Hex Bloom", "Hexagon Halo"] {
            assert!(
                scene_by_name(name).modules_in_group("Effects").is_empty(),
                "{name}"
            );
        }
    }

    #[test]
    fn uses_circular_screen_posterize_and_ascii_art_in_effects_groups() {
        // default-scenes.test.js:394-411
        let effects: Vec<Vec<&str>> = default_scenes()
            .iter()
            .map(|scene| scene.modules_in_group("Effects"))
            .collect();

        assert!(
            effects
                .iter()
                .any(|names| names.contains(&"Circular Screen"))
        );
        assert!(
            effects.iter().any(|names| {
                names.contains(&"Posterize") || names.contains(&"Color Posterize")
            })
        );
        assert!(effects.iter().any(|names| names.contains(&"ASCII Art")));
    }

    #[test]
    fn lets_a_scene_use_any_subset_of_the_group_roles() {
        // default-scenes.test.js:421-456
        for scene in default_scenes() {
            let group_names: Vec<&str> = scene
                .groups
                .iter()
                .map(|group| group.name.as_str())
                .collect();

            assert!(!group_names.is_empty());
            assert!(group_names.len() <= GROUP_NAMES.len());
            assert_eq!(
                group_names.iter().collect::<HashSet<_>>().len(),
                group_names.len()
            );
            assert!(group_names.iter().all(|name| GROUP_NAMES.contains(name)));

            for (index, group) in scene.groups.iter().enumerate() {
                // The first Group is the base layer; later Groups inherit the
                // frame produced by the row above.
                assert_eq!(group.inherit, Some(index > 0), "{}", scene.name);
                assert!(group.clearing.is_some());
                assert!(group.alpha.is_some());
                assert!(group.composite_operation.is_some());
            }

            if scene.name != "Soft Bands" {
                assert!(!scene.module_names().is_empty(), "{}", scene.name);
            }
        }
    }

    #[test]
    fn leaves_rows_empty_rather_than_replacing_a_removed_waveform_or_bar() {
        // default-scenes.test.js:467-481
        let empty: Vec<String> = default_scenes()
            .iter()
            .flat_map(|scene| {
                scene
                    .groups
                    .iter()
                    .filter(|group| group.modules.is_empty())
                    .map(move |group| format!("{}:{}", scene.name, group.name))
            })
            .collect();

        for expected in [
            "Mycelium Bloom:Foreground",
            "Waveform Drive:Background",
            "Terminal Bloom:Foreground",
        ] {
            assert!(empty.contains(&expected.to_string()), "missing {expected}");
        }
    }

    #[test]
    fn gives_every_scene_a_sensible_duration() {
        // default-scenes.test.js:483-489
        for scene in default_scenes() {
            let min = scene.min_seconds.expect("minSeconds");
            let max = scene.max_seconds.expect("maxSeconds");
            assert!(min.is_finite() && max.is_finite(), "{}", scene.name);
            assert!(min < max, "{}", scene.name);
        }
    }

    #[test]
    fn keeps_twirl_only_in_it_twirls_at_night() {
        // default-scenes.test.js:501-509
        let twirl: Vec<&str> = default_scenes()
            .iter()
            .filter(|scene| scene.module_names().contains(&"Twirl"))
            .map(|scene| scene.name.as_str())
            .collect();

        assert_eq!(twirl, vec!["It Twirls At Night"]);
    }

    #[test]
    fn adds_a_version_8_rave_exe_scene() {
        // default-scenes.test.js:1078-1099
        let scene = scene_by_name("Rave.exe");
        assert_eq!(scene.introduced_version, 8);
        assert!(scene.modules_in_group("Foreground").contains(&"rave.exe"));

        let kick = scene.link("rave.exe", "kick").unwrap();
        assert_eq!(kick.group, "Foreground");
        assert_eq!(kick.feature.as_deref(), Some("bass"));
        assert!(scene.min_seconds < scene.max_seconds);
    }

    #[test]
    fn updates_digital_black_hole_and_it_twirls_at_night() {
        // default-scenes.test.js:867-1012 (spot checks)
        let black_hole = scene_by_name("Digital Black Hole");
        assert_eq!(
            black_hole.modules_in_group("Foreground"),
            vec!["Singularity Rain"]
        );
        assert!(
            black_hole
                .modules_in_group("Effects")
                .contains(&"VHS Glitch")
        );
        let alpha = black_hole.link("Singularity Rain", "alpha").unwrap();
        assert_eq!(alpha.feature.as_deref(), Some("bass"));
        assert_eq!(alpha.max, Some(1.0));

        let twirls = scene_by_name("It Twirls At Night");
        assert_eq!(
            twirls.modules_in_group("Foreground"),
            vec!["Point10 It Is Coming For You"]
        );
        assert!(twirls.modules_in_group("Effects").contains(&"Twirl"));
        assert!(twirls.modules_in_group("Effects").contains(&"Bloom"));
    }

    #[test]
    fn adds_the_version_9_sparse_scenes() {
        // default-scenes.test.js:1130-1199
        let v9 = batch(9);
        assert!(v9.len() >= 5);

        let background_only = v9
            .iter()
            .filter(|scene| scene.groups.len() == 1 && scene.groups[0].name == "Background")
            .count();
        let foreground_only = v9
            .iter()
            .filter(|scene| scene.groups.len() == 1 && scene.groups[0].name == "Foreground")
            .count();
        assert!(background_only >= 2);
        assert!(foreground_only >= 1);

        let rolling = scene_by_name("Rolling Fire Bloom");
        assert!(rolling.modules_in_group("Foreground").is_empty());
        assert_eq!(
            rolling.modules_in_group("Background"),
            vec!["Point10 Rolling Fire"]
        );
        assert_eq!(rolling.modules_in_group("Effects"), vec!["Bloom"]);

        let water_garden = scene_by_name("Around Water Garden");
        assert!(water_garden.modules_in_group("Foreground").is_empty());
        assert!(!has_link(water_garden, "Bands"));

        let mosaic = scene_by_name("Mosaic Meridian");
        assert_eq!(
            mosaic.modules_in_group("Background"),
            vec!["Point10 Chromatic Wave"]
        );
        assert_eq!(mosaic.modules_in_group("Effects"), vec!["Color Posterize"]);

        let soft_bands = scene_by_name("Soft Bands");
        assert_eq!(
            soft_bands
                .groups
                .iter()
                .map(|group| group.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Background"]
        );
        assert!(soft_bands.modules_in_group("Background").is_empty());

        let quiet = scene_by_name("Quiet Bloom");
        assert_eq!(
            quiet
                .groups
                .iter()
                .map(|group| group.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Background", "Effects"]
        );
        assert_eq!(quiet.modules_in_group("Effects"), vec!["Bloom"]);
    }

    #[test]
    fn chains_an_effect_inside_a_foreground_group() {
        // default-scenes.test.js:1201-1222
        let effects = [
            "Bloom",
            "Chromatic Abberation",
            "Vignette",
            "Mirror",
            "VHS Glitch",
            "RGB Halftone",
        ];
        let chained: Vec<&SceneDocument> = batch(9)
            .into_iter()
            .filter(|scene| {
                let names = scene.modules_in_group("Foreground");
                names.iter().any(|name| !effects.contains(name))
                    && names.iter().any(|name| effects.contains(name))
            })
            .collect();

        assert!(!chained.is_empty());
        assert!(chained.iter().any(|scene| {
            scene
                .modules_in_group("Foreground")
                .contains(&"Chromatic Abberation")
        }));
    }

    #[test]
    fn keeps_strobe_subtle_and_synced_to_the_beat() {
        // default-scenes.test.js:541-564
        let scene = scene_by_name("Strobe Riot");
        let strobe = scene
            .group("Effects")
            .and_then(|group| group.modules.iter().find(|module| module.name == "Strobe"))
            .expect("Strobe");

        let alpha = strobe
            .meta
            .as_ref()
            .and_then(|meta| meta.get("alpha"))
            .and_then(Value::as_f64)
            .expect("strobe alpha");
        assert!(alpha <= 0.25);
        assert!(
            strobe.props.as_ref().unwrap()["strobeRate"]
                .as_f64()
                .unwrap()
                >= 0.4
        );

        let link = scene
            .link("Strobe", "strobeRate")
            .expect("strobe rate link");
        assert_eq!(link.group, "Effects");
        assert_eq!(link.source.as_deref(), Some("tween"));
        assert!(link.min.unwrap() < link.max.unwrap());
        assert!(link.bpm_division.unwrap() > 0);
    }

    #[test]
    fn keeps_rgb_shift_and_digital_glitch_in_effects_groups() {
        // default-scenes.test.js:566-628
        let scenes = [
            "Prism Reliquary",
            "Glitch Bloom Cathedral",
            "VHS Reliquary",
            "It Is Coming",
            "It Twirls At Night",
            "Boxy Arcade",
            "DVD Nightmare",
            "Rave.exe",
        ];
        let glitches = ["Point10 RGB Shift", "Point10 Digital Glitch"];

        for name in scenes {
            let scene = scene_by_name(name);
            assert!(
                !scene
                    .modules_in_group("Foreground")
                    .iter()
                    .any(|module| glitches.contains(module)),
                "{name} has a glitch in Foreground"
            );
            assert!(
                scene
                    .modules_in_group("Effects")
                    .iter()
                    .any(|module| glitches.contains(module)),
                "{name} misses a glitch in Effects"
            );
            // The background moved into Foreground when the glitch effect was
            // the only foreground layer.
            assert_eq!(
                scene
                    .groups
                    .iter()
                    .map(|group| group.name.as_str())
                    .collect::<Vec<_>>(),
                vec!["Foreground", "Effects"],
                "{name}"
            );
            assert!(
                !scene
                    .input_links
                    .iter()
                    .any(|link| link.group == "Background"),
                "{name}"
            );
        }
    }

    // ————————————————— seeding —————————————————

    #[test]
    fn seeds_newer_batches_for_an_install_on_version_2() {
        // default-scenes.test.js:659-672
        let selected = get_scenes_to_seed(2, &get_default_scene_names_up_to(2), &Vec::new());
        let newer: Vec<&str> = default_scenes()
            .iter()
            .filter(|scene| scene.introduced_version > 2)
            .map(|scene| scene.name.as_str())
            .collect();

        assert_eq!(
            selected
                .iter()
                .map(|scene| scene.name.as_str())
                .collect::<Vec<_>>(),
            newer
        );
    }

    #[test]
    fn seeds_the_version_3_batch_for_an_install_on_version_2() {
        // default-scenes.test.js:674-687
        let selected = get_scenes_to_seed(3, &get_default_scene_names_up_to(3), &Vec::new());
        let newer: Vec<&str> = default_scenes()
            .iter()
            .filter(|scene| scene.introduced_version > 3)
            .map(|scene| scene.name.as_str())
            .collect();

        assert_eq!(
            selected
                .iter()
                .map(|scene| scene.name.as_str())
                .collect::<Vec<_>>(),
            newer
        );
    }

    #[test]
    fn seeds_the_version_6_batch_for_an_install_on_version_5() {
        // default-scenes.test.js:689-702
        let selected = get_scenes_to_seed(5, &get_default_scene_names_up_to(5), &Vec::new());
        let newer: Vec<&str> = default_scenes()
            .iter()
            .filter(|scene| scene.introduced_version > 5)
            .map(|scene| scene.name.as_str())
            .collect();

        assert_eq!(
            selected
                .iter()
                .map(|scene| scene.name.as_str())
                .collect::<Vec<_>>(),
            newer
        );
    }

    #[test]
    fn offers_no_default_batch_to_an_install_on_the_latest_version() {
        // default-scenes.test.js:704-712
        let selected = get_scenes_to_seed(
            DEFAULTS_VERSION,
            &get_default_scene_names_up_to(DEFAULTS_VERSION),
            &Vec::new(),
        );

        assert!(selected.is_empty());
    }

    #[test]
    fn does_not_offer_removed_scenes_to_fresh_installs() {
        // default-scenes.test.js:1116-1127
        let selected = get_scenes_to_seed(0, &Vec::new(), &Vec::new());
        let names: Vec<&str> = selected.iter().map(|scene| scene.name.as_str()).collect();

        for removed in REMOVED_DEFAULT_SCENES {
            assert!(!names.contains(removed), "{removed} was offered");
        }
    }

    #[test]
    fn skips_scenes_already_seeded_or_present_by_name() {
        // default-scenes.js:185-195
        let seeded = vec!["Mycelium Bloom".to_string()];
        let existing = vec!["  GEOMETRIC TUNNEL ".to_string()];
        let selected = get_scenes_to_seed(0, &seeded, &existing);
        let names: Vec<&str> = selected.iter().map(|scene| scene.name.as_str()).collect();

        assert!(!names.contains(&"Mycelium Bloom"));
        assert!(!names.contains(&"Geometric Tunnel"));
        assert!(names.contains(&"Smoke Room"));
    }

    // ————————————————— removal migration —————————————————

    #[test]
    fn removes_seeded_retired_scenes_for_installs_upgrading_to_version_8() {
        // default-scenes.test.js:1444-1468
        let names = [
            "Ferrofluid Pulse",
            "Kaleidoscope Cathedral",
            "Slit Scan Horizon",
            "Spiral Particles",
            "Spectral Bloom",
            "Fractured Kaleidoscope",
            "Broken Mirror Ball",
        ];
        let scenes: Vec<Value> = names
            .iter()
            .enumerate()
            .map(|(index, name)| seeded_with_id(name, &format!("id-{index}")))
            .collect();
        let seeded_names: Vec<String> = names.iter().map(|name| name.to_string()).collect();

        let result = get_default_scenes_to_remove(7, &scenes, Some(&seeded_names));

        assert_eq!(
            result
                .iter()
                .map(|removal| removal.name.as_str())
                .collect::<Vec<_>>(),
            names
        );
        assert_eq!(
            result
                .iter()
                .map(|removal| removal.id.clone())
                .collect::<Vec<_>>(),
            scenes
                .iter()
                .map(|scene| scene["id"].clone())
                .collect::<Vec<_>>()
        );
    }

    fn seeded_with_id(name: &str, id: &str) -> Value {
        json!({"id": id, "name": name, "modules": {}})
    }

    #[test]
    fn removes_roto_zoom_riot_for_installs_upgrading_to_version_11() {
        // default-scenes.test.js:1470-1479
        let scenes = vec![seeded_with_id("Roto Zoom Riot", "roto-id")];
        let result =
            get_default_scenes_to_remove(10, &scenes, Some(&["Roto Zoom Riot".to_string()]));

        assert_eq!(
            result,
            vec![SceneRemoval {
                id: json!("roto-id"),
                name: "Roto Zoom Riot".to_string(),
            }]
        );
    }

    #[test]
    fn keeps_user_scenes_that_share_a_retired_name_but_were_not_seeded() {
        // default-scenes.test.js:1481-1490
        let scenes = vec![seeded_with_id("Ferrofluid Pulse", "user-id")];
        let result =
            get_default_scenes_to_remove(7, &scenes, Some(&["Something Else".to_string()]));

        assert!(result.is_empty());
    }

    #[test]
    fn does_not_remove_anything_on_the_new_defaults_version() {
        // default-scenes.test.js:1492-1501
        let scenes = vec![seeded_with_id("Ferrofluid Pulse", "user-id")];
        let result = get_default_scenes_to_remove(
            DEFAULTS_VERSION,
            &scenes,
            Some(&["Ferrofluid Pulse".to_string()]),
        );

        assert!(result.is_empty());
    }

    // ————————————————— replacement migration —————————————————

    fn cube_floor_scene(name: &str, id: &str) -> Value {
        json!({
            "id": id,
            "name": name,
            "modules": {
                "module-id": {"$moduleName": REMOVED_DEFAULT_SCENE_MODULES[0]},
            },
        })
    }

    #[test]
    fn rebuilds_existing_defaults_that_still_use_a_removed_module() {
        // default-scenes.test.js:726-767
        let scenes = vec![
            cube_floor_scene("Smoke Tunnel", "smoke-tunnel-id"),
            cube_floor_scene("Mycelium Bloom", "mycelium-id"),
            cube_floor_scene("Dither District", "dither-id"),
        ];
        let plan = get_default_scenes_to_replace(6, &scenes, None, None);

        assert!(plan.removals.is_empty());
        assert!(plan.additions.is_empty());
        let names: Vec<&str> = plan
            .replacements
            .iter()
            .map(|replacement| replacement.name.as_str())
            .collect();
        for name in ["Smoke Tunnel", "Mycelium Bloom", "Dither District"] {
            assert!(names.contains(&name), "missing replacement {name}");
        }

        let smoke = plan
            .replacements
            .iter()
            .find(|replacement| replacement.name == "Smoke Tunnel")
            .unwrap();
        assert_eq!(smoke.id, json!("smoke-tunnel-id"));
        assert_eq!(smoke.definition.name, "Smoke Tunnel");
        assert!(
            !smoke
                .definition
                .module_names()
                .contains(&REMOVED_DEFAULT_SCENE_MODULES[0])
        );
    }

    #[test]
    fn preserves_user_edits_when_a_scene_no_longer_uses_the_removed_module() {
        // default-scenes.test.js:769-788
        let scenes = vec![json!({
            "id": "smoke-tunnel-id",
            "name": "Smoke Tunnel",
            "modules": {"module-id": {"$moduleName": "Point10 Light Tunnel"}},
        })];
        let plan = get_default_scenes_to_replace(6, &scenes, None, None);

        assert!(plan.replacements.is_empty());
        assert!(plan.removals.is_empty());
        assert!(plan.additions.is_empty());
    }

    #[test]
    fn removes_the_renamed_cube_floor_drive_and_seeds_waveform_drive() {
        // default-scenes.test.js:790-805
        let scenes = vec![cube_floor_scene("Cube Floor Drive", "cube-drive-id")];
        let plan = get_default_scenes_to_replace(6, &scenes, None, None);

        assert!(plan.replacements.is_empty());
        assert_eq!(
            plan.removals,
            vec![SceneRemoval {
                id: json!("cube-drive-id"),
                name: "Cube Floor Drive".to_string(),
            }]
        );
        assert_eq!(
            plan.additions
                .iter()
                .map(|definition| definition.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Waveform Drive"]
        );
    }

    #[test]
    fn ignores_user_scenes_that_do_not_share_a_default_name() {
        // default-scenes.test.js:807-818
        let scenes = vec![cube_floor_scene("My Cube Floor Experiment", "user-id")];
        let plan = get_default_scenes_to_replace(6, &scenes, None, None);

        assert!(plan.replacements.is_empty());
        assert!(plan.removals.is_empty());
        assert!(plan.additions.is_empty());
    }

    #[test]
    fn does_not_migrate_installs_already_on_the_new_defaults_version() {
        // default-scenes.test.js:820-831
        let scenes = vec![cube_floor_scene("Smoke Tunnel", "smoke-tunnel-id")];
        let plan = get_default_scenes_to_replace(DEFAULTS_VERSION, &scenes, None, None);

        assert!(plan.replacements.is_empty());
        assert!(plan.removals.is_empty());
        assert!(plan.additions.is_empty());
    }

    #[test]
    fn renames_updated_defaults_that_were_not_removed_for_a_module() {
        // default-scenes.test.js:1504-1534
        let scenes = vec![
            json!({"id": "vhs-id", "name": "VHS Afterglow", "modules": {}}),
            json!({"id": "night-id", "name": "It Comes At Night", "modules": {}}),
            json!({"id": "grid-id", "name": "Neon Grid", "modules": {}}),
            json!({"id": "sphere-id", "name": "Chromatic Sphere", "modules": {}}),
            json!({"id": "twirl-id", "name": "Twirl Tunnel", "modules": {}}),
        ];
        let seeded_names: Vec<String> = scenes
            .iter()
            .map(|scene| scene["name"].as_str().unwrap().to_string())
            .collect();

        let plan = get_default_scenes_to_replace(7, &scenes, None, Some(&seeded_names));

        assert_eq!(
            plan.removals
                .iter()
                .map(|removal| removal.id.clone())
                .collect::<Vec<_>>(),
            vec![
                json!("vhs-id"),
                json!("night-id"),
                json!("grid-id"),
                json!("sphere-id"),
                json!("twirl-id"),
            ]
        );
        assert_eq!(
            plan.additions
                .iter()
                .map(|definition| definition.name.as_str())
                .collect::<Vec<_>>(),
            vec![
                "Digital Black Hole",
                "It Twirls At Night",
                "Smoke Tunnel",
                "Chromatic Waveform",
                "Golden Spiral Tunnel",
            ]
        );
    }

    #[test]
    fn keeps_same_named_scenes_that_were_not_seeded_from_the_defaults() {
        // default-scenes.test.js:1536-1547
        let scenes = vec![json!({"id": "mine", "name": "VHS Afterglow", "modules": {}})];
        let plan = get_default_scenes_to_replace(7, &scenes, None, Some(&Vec::new()));

        assert!(plan.removals.is_empty());
        assert!(plan.additions.is_empty());
        assert!(plan.replacements.is_empty());
    }

    #[test]
    fn refreshes_the_trimmed_seeded_defaults_for_installs_on_version_11() {
        // default-scenes.test.js:1243-1310
        let names = [
            "Circuit Ritual",
            "Burning Mycelium Circle",
            "Soft Bands",
            "Quiet Bloom",
        ];
        let scenes: Vec<Value> = names.iter().map(|name| seeded(name)).collect();
        let seeded_names: Vec<String> = names.iter().map(|name| name.to_string()).collect();

        let plan = get_default_scenes_to_replace(11, &scenes, None, Some(&seeded_names));
        let replaced: Vec<&str> = plan
            .replacements
            .iter()
            .map(|replacement| replacement.name.as_str())
            .collect();

        for name in names {
            assert!(replaced.contains(&name), "missing refresh {name}");
        }

        let circuit = plan
            .replacements
            .iter()
            .find(|replacement| replacement.name == "Circuit Ritual")
            .unwrap();
        assert_eq!(circuit.id, json!("Circuit Ritual-id"));
        assert!(!circuit.definition.module_names().contains(&"RGB Halftone"));

        let burning = plan
            .replacements
            .iter()
            .find(|replacement| replacement.name == "Burning Mycelium Circle")
            .unwrap();
        assert_eq!(
            burning
                .definition
                .groups
                .iter()
                .map(|group| group.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Background", "Effects"]
        );

        let soft_bands = plan
            .replacements
            .iter()
            .find(|replacement| replacement.name == "Soft Bands")
            .unwrap();
        assert_eq!(soft_bands.id, json!("Soft Bands-id"));
        assert_eq!(
            soft_bands
                .definition
                .groups
                .iter()
                .map(|group| group.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Background"]
        );

        let quiet = plan
            .replacements
            .iter()
            .find(|replacement| replacement.name == "Quiet Bloom")
            .unwrap();
        assert_eq!(quiet.definition.modules_in_group("Effects"), vec!["Bloom"]);
    }

    #[test]
    fn lists_the_scenes_whose_definitions_changed() {
        // default-scenes.test.js:1312-1319
        for name in [
            "Circuit Ritual",
            "Strobe Riot",
            "Soft Bands",
            "Quiet Bloom",
            "It Twirls At Night",
        ] {
            assert!(REFRESHED_DEFAULT_SCENES.contains(&name), "{name}");
        }
        assert!(!REFRESHED_DEFAULT_SCENES.contains(&"It Comes At Night"));
    }

    #[test]
    fn does_not_refresh_installs_two_versions_behind() {
        // default-scenes.test.js:1419-1429
        let scenes = vec![seeded("Soft Bands"), seeded("Quiet Bloom")];
        let seeded_names = vec!["Soft Bands".to_string(), "Quiet Bloom".to_string()];

        let plan = get_default_scenes_to_replace(10, &scenes, None, Some(&seeded_names));

        assert!(plan.replacements.is_empty());
    }

    #[test]
    fn does_not_overwrite_user_scenes_that_were_not_seeded() {
        // default-scenes.test.js:1321-1331, 1431-1442
        let scenes = vec![seeded("Circuit Ritual"), seeded("Soft Bands")];
        let plan = get_default_scenes_to_replace(11, &scenes, None, Some(&Vec::new()));

        assert!(plan.replacements.is_empty());
    }

    #[test]
    fn freshness_check_is_scoped_to_the_previous_version() {
        // default-scenes.js:381-414
        let scenes = vec![seeded("Quiet Bloom")];
        let removed: &[&str] = &[];
        let plan = get_default_scenes_to_replace(
            11,
            &scenes,
            Some(removed),
            Some(&["Quiet Bloom".to_string()]),
        );

        assert_eq!(plan.replacements.len(), 1);
        assert_eq!(plan.replacements[0].definition.name, "Quiet Bloom");
        assert_eq!(plan.replacements[0].id, json!("Quiet Bloom-id"));
    }

    #[test]
    fn rebuilds_the_bands_using_defaults_for_installs_on_version_11() {
        // default-scenes.test.js:1349-1397
        let names = [
            "Rolling Fire Bloom",
            "Around Water Garden",
            "Mosaic Meridian",
            "Soft Bands",
            "Quiet Bloom",
        ];
        let scenes: Vec<Value> = names.iter().map(|name| seeded(name)).collect();
        let seeded_names: Vec<String> = names.iter().map(|name| name.to_string()).collect();

        let plan = get_default_scenes_to_replace(11, &scenes, None, Some(&seeded_names));
        let replaced: Vec<&str> = plan
            .replacements
            .iter()
            .map(|replacement| replacement.name.as_str())
            .collect();

        for name in names {
            assert!(replaced.contains(&name), "missing refresh {name}");
        }

        for replacement in &plan.replacements {
            assert_eq!(replacement.id, json!(format!("{}-id", replacement.name)));
            assert!(!replacement.definition.module_names().contains(&"Bands"));
            assert!(
                replacement
                    .definition
                    .input_links
                    .iter()
                    .all(|link| link.module.as_deref() != Some("Bands"))
            );
        }

        let rolling = plan
            .replacements
            .iter()
            .find(|replacement| replacement.name == "Rolling Fire Bloom")
            .unwrap();
        assert!(rolling.definition.modules_in_group("Foreground").is_empty());
    }
}
