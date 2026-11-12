//! Version-keyed rename/removal migrations for the default Scenes.
//!
//! The default Scene set ships with [`crate::default_scenes::DEFAULTS_VERSION`]
//! (`default-scenes.js:39`). Seeding
//! ([`crate::default_scenes::get_scenes_to_seed`]) only appends names that are
//! new *and* neither already seeded nor already present, so it cannot express
//! two edits that a version bump may make to Scenes an install already holds:
//!
//! - a default Scene is **renamed** (the library would keep the stale name and
//!   gain nothing, or — if the rename also shipped a new definition — end up
//!   with two Scenes for one look), and
//! - a default Scene is **retired** (the old name would linger forever).
//!
//! This module is the data-driven table and the pure logic for those edits:
//!
//! - [`DEFAULT_SCENE_MIGRATIONS`] — one [`DefaultSceneMigrationRule`] per
//!   defaults-version bump that renamed or retired seeded Scenes.
//! - [`get_default_scene_migrations`] — aggregates the rules in `from..=to`.
//! - [`apply_default_scene_migrations`] — applies the aggregate to a Scene list
//!   and the `seededDefaultScenes` list, reporting what happened.
//! - [`ensure_default_scene_migration`] — the call the store makes before
//!   seeding: applies the migration and returns the definitions that still
//!   need to be built for the new version.
//!
//! ## Adding a rule
//!
//! When `default-scenes.js` renames or removes a default Scene, add the pair /
//! name to the rule for the version that ships the change (create the rule if
//! it is the first edit of that bump) and keep the table in ascending `version`
//! order. A rule only applies to installs moving across its version, exactly
//! once. The `the_production_table_is_consistent` test guards the invariants:
//! the old name must be gone from the data, the new name must exist.
//!
//! ## Why the production table is empty
//!
//! The historical edits in the JS file are **not** version-keyed:
//! `RENAMED_DEFAULT_SCENES` (`default-scenes.js:60-100`) and
//! `REMOVED_DEFAULT_SCENES` (`default-scenes.js:49-58`) are applied to *any*
//! install below `DEFAULTS_VERSION` by `getDefaultScenesToReplace` /
//! `getDefaultScenesToRemove`, which remain the ported path for them
//! ([`crate::default_scenes::get_default_scenes_to_replace`],
//! [`crate::default_scenes::get_default_scenes_to_remove`]). Keying those rows
//! here would invent which bump each rename shipped in — and re-remove Scenes
//! the in-place refresh already handles — so the version-keyed table starts
//! empty. The first rename/removal that ships *with* a version bump is recorded
//! here. The mechanism is exercised by
//! [`get_default_scene_migrations_with`]/[`ensure_default_scene_migrations_with`],
//! which accept an explicit rule table.
//!
//! ## Semantics
//!
//! A "Scene" here is a serialised Scene document ([`serde_json::Value`]); the
//! engine keeps Scenes as JSON documents (`ScenesState::scenes` in
//! `modv-app`), so the helpers take `&mut Vec<Value>`.
//!
//! - Names are matched with [`crate::default_scenes::scene_key`] (trim +
//!   lower-case), like every other seeding/migration helper.
//! - Only names listed in `seeded_names` are migrated. A user-created Scene
//!   that happens to share a default's name is never touched (and a user Scene
//!   whose name is not in `seeded_names` blocks a rename collision, see below).
//! - A **rename** rewrites the Scene's `name` in place, keeps its `id`, and
//!   replaces the old entry in `seeded_names`. The JS
//!   `getDefaultScenesToReplace` implements a rename as remove + re-add, so the
//!   migrated Scene receives a **fresh id** there. Keeping the id here
//!   preserves `currentSceneId`, the Scene's timing override and playlist
//!   references across the bump; `seeded_names` still maps old → name so
//!   seeding cannot add a duplicate.
//! - A **removal** deletes the first seeded Scene carrying the retired name
//!   (the JS removes one Scene per rule, by id) and drops the name from
//!   `seeded_names`. Another same-named Scene — a seeded duplicate, a user
//!   Scene, or an import — is kept. A removed name that still exists in the
//!   current defaults would be re-seeded, so a rule must only retire names that
//!   left the data (guarded by the consistency test).
//! - A rename whose target name is already carried by another Scene is
//!   **skipped** (reported in [`MigrationReport::skipped`]), never merged or
//!   overwritten.
//! - Re-running the same migration is a no-op: the old name is gone, so the
//!   second pass reports it as skipped and leaves the library and
//!   `seeded_names` unchanged.
//!
//! The rules ported here correspond to no specific JS function: they are the
//! version-keyed complement of `default-scenes.js`'s unversioned tables.
//! Line numbers refer to the tree at commit `8a512b6e` (branch `rust-hybrid`).

use std::collections::HashSet;

use serde_json::Value;

use crate::default_scenes::{SceneDocument, get_scenes_to_seed, scene_key};

/// One defaults-version bump's rename/removal edits.
#[derive(Clone, Debug, PartialEq)]
pub struct DefaultSceneMigrationRule {
    /// The [`crate::default_scenes::DEFAULTS_VERSION`] bump this rule ships
    /// with. Applied to installs moving from `< version` to `>= version`.
    pub version: i64,
    /// `(old name, new name)` pairs for Scenes renamed in this bump.
    pub renames: &'static [(&'static str, &'static str)],
    /// Names retired from the defaults in this bump.
    pub removed: &'static [&'static str],
}

/// The version-keyed rename/removal table.
///
/// Empty on purpose: see the module docs ("Why the production table is
/// empty"). Add one row per defaults-version bump that renames or retires a
/// seeded Scene, in ascending `version` order.
pub const DEFAULT_SCENE_MIGRATIONS: &[DefaultSceneMigrationRule] = &[];

/// The aggregated edits for a version span.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DefaultSceneMigration {
    /// `(old name, new name)` pairs, in application order.
    pub rename: Vec<(String, String)>,
    /// Retired names, in application order.
    pub remove: Vec<String>,
}

impl DefaultSceneMigration {
    /// Whether there is nothing to migrate.
    pub fn is_empty(&self) -> bool {
        self.rename.is_empty() && self.remove.is_empty()
    }
}

/// What [`apply_default_scene_migrations`] changed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MigrationReport {
    /// Scenes renamed, as `(old name, new name)`.
    pub renamed: Vec<(String, String)>,
    /// Seeded Scenes removed (each name once).
    pub removed: Vec<String>,
    /// Rules that did not apply, keyed by their old/retired name: no seeded
    /// Scene carried it, or the rename target was already taken.
    pub skipped: Vec<String>,
}

/// What [`ensure_default_scene_migration`] did and what the caller must seed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DefaultSceneMigrationPlan {
    /// The aggregated edits for the version span.
    pub migration: DefaultSceneMigration,
    /// Which of them applied.
    pub report: MigrationReport,
    /// Definitions introduced after `from_version` that are still missing and
    /// must be built and marked seeded by the caller — the same list
    /// [`crate::default_scenes::get_scenes_to_seed`] would return *after* the
    /// migration.
    pub seeds: Vec<&'static SceneDocument>,
}

/// Aggregate every [`DEFAULT_SCENE_MIGRATIONS`] rule in `from_version..=to_version`.
///
/// Empty when `to_version <= from_version`, so an install that is already on
/// (or ahead of) the target never re-applies history.
pub fn get_default_scene_migrations(from_version: i64, to_version: i64) -> DefaultSceneMigration {
    get_default_scene_migrations_with(DEFAULT_SCENE_MIGRATIONS, from_version, to_version)
}

/// [`get_default_scene_migrations`] with an explicit rule table (tests, or a
/// caller that composes rules from elsewhere).
pub fn get_default_scene_migrations_with(
    rules: &[DefaultSceneMigrationRule],
    from_version: i64,
    to_version: i64,
) -> DefaultSceneMigration {
    let mut result = DefaultSceneMigration::default();

    if to_version <= from_version {
        return result;
    }

    let mut seen_renames: HashSet<String> = HashSet::new();
    let mut seen_removals: HashSet<String> = HashSet::new();

    for rule in rules {
        if rule.version <= from_version || rule.version > to_version {
            continue;
        }

        for (old_name, new_name) in rule.renames {
            if seen_renames.insert(scene_key(old_name)) {
                result
                    .rename
                    .push(((*old_name).to_string(), (*new_name).to_string()));
            }
        }

        for name in rule.removed {
            if seen_removals.insert(scene_key(name)) {
                result.remove.push((*name).to_string());
            }
        }
    }

    result
}

/// Apply `migration` to `scenes` and `seeded_names`, reporting the outcome.
///
/// Renames run before removals (so a bump may rename into a name it retires
/// later, though the consistency test discourages that). Only names listed in
/// `seeded_names` are touched; see the module docs for the id/collision rules.
// The removal path uses `Vec::retain` and the store passes its Scene list, so
// the public API keeps `&mut Vec` rather than a slice.
#[allow(clippy::ptr_arg)]
pub fn apply_default_scene_migrations(
    scenes: &mut Vec<Value>,
    seeded_names: &mut Vec<String>,
    migration: &DefaultSceneMigration,
) -> MigrationReport {
    let mut report = MigrationReport::default();

    for (old_name, new_name) in &migration.rename {
        apply_rename(scenes, seeded_names, old_name, new_name, &mut report);
    }

    for name in &migration.remove {
        apply_removal(scenes, seeded_names, name, &mut report);
    }

    report
}

/// The migration + seeding plan for one version bump.
///
/// Run this *before* seeding: renames/removals are applied first (so a renamed
/// Scene is not seeded again under its new name and a retired Scene is not
/// resurrected), then [`crate::default_scenes::get_scenes_to_seed`] is computed
/// from the migrated `seeded_names`/`existing names`. The caller builds
/// [`DefaultSceneMigrationPlan::seeds`] and marks their names seeded; passing
/// `to_version = DEFAULTS_VERSION` makes an install on `DEFAULTS_VERSION - 1`
/// converge on the fresh-install set exactly once.
pub fn ensure_default_scene_migration(
    from_version: i64,
    to_version: i64,
    scenes: &mut Vec<Value>,
    seeded_names: &mut Vec<String>,
) -> DefaultSceneMigrationPlan {
    ensure_default_scene_migrations_with(
        DEFAULT_SCENE_MIGRATIONS,
        from_version,
        to_version,
        scenes,
        seeded_names,
    )
}

/// [`ensure_default_scene_migration`] with an explicit rule table.
pub fn ensure_default_scene_migrations_with(
    rules: &[DefaultSceneMigrationRule],
    from_version: i64,
    to_version: i64,
    scenes: &mut Vec<Value>,
    seeded_names: &mut Vec<String>,
) -> DefaultSceneMigrationPlan {
    let migration = get_default_scene_migrations_with(rules, from_version, to_version);
    let report = apply_default_scene_migrations(scenes, seeded_names, &migration);
    let existing_names: Vec<String> = scenes
        .iter()
        .filter_map(|scene| scene.get("name").and_then(Value::as_str))
        .map(str::to_string)
        .collect();
    let seeds = get_scenes_to_seed(from_version, seeded_names, &existing_names);

    DefaultSceneMigrationPlan {
        migration,
        report,
        seeds,
    }
}

/// A Scene document's `name`, if it is a string.
fn scene_name(scene: &Value) -> Option<&str> {
    scene.get("name").and_then(Value::as_str)
}

/// Rename the first seeded Scene called `old_name` to `new_name`.
fn apply_rename(
    scenes: &mut [Value],
    seeded_names: &mut Vec<String>,
    old_name: &str,
    new_name: &str,
    report: &mut MigrationReport,
) {
    let old_key = scene_key(old_name);
    let new_key = scene_key(new_name);

    // A case/spacing-only edit has nothing to rewrite; name matching is
    // case-insensitive everywhere, so treat it as a no-op rather than a rule
    // failure.
    if old_key == new_key {
        return;
    }

    let Some(index) = seeded_scene_index(scenes, seeded_names, &old_key) else {
        report.skipped.push(old_name.to_string());
        return;
    };

    // Never merge into or overwrite a Scene that already owns the new name
    // (a user Scene, a previously renamed Scene, or a duplicate import).
    if scenes.iter().enumerate().any(|(other, scene)| {
        other != index && scene_name(scene).is_some_and(|name| scene_key(name) == new_key)
    }) {
        report.skipped.push(old_name.to_string());
        return;
    }

    let actual_old = scene_name(&scenes[index]).unwrap_or(old_name).to_string();

    if let Some(object) = scenes[index].as_object_mut() {
        object.insert("name".to_string(), Value::String(new_name.to_string()));
    } else {
        // Not a JSON object: leave it alone rather than replacing the payload.
        report.skipped.push(old_name.to_string());
        return;
    }

    rename_seeded_entry(seeded_names, &old_key, new_name);
    report.renamed.push((actual_old, new_name.to_string()));
}

/// Delete the first seeded Scene called `name` and stop treating the name as
/// seeded.
///
/// The JS removal path removes **one** Scene per rule — the id
/// `getDefaultScenesToRemove` reported — and then deletes the name from its
/// `seededKeys` set (`scenes.js:3028-3047`). Removing every same-named Scene
/// would delete an unrelated seeded duplicate as well. Dropping *all*
/// `seeded_names` entries for the name (not just the first) keeps a second run
/// from retiring the surviving duplicate: the JS never runs a removal twice for
/// the same bump, but the engine may re-apply a migration on a failed upgrade.
fn apply_removal(
    scenes: &mut Vec<Value>,
    seeded_names: &mut Vec<String>,
    name: &str,
    report: &mut MigrationReport,
) {
    let key = scene_key(name);
    let seeded: HashSet<String> = seeded_names.iter().map(|name| scene_key(name)).collect();

    if !seeded.contains(&key) {
        report.skipped.push(name.to_string());
        return;
    }

    // Only seeded Scenes are retired; a same-named user Scene is kept, and
    // exactly one duplicate is removed.
    let Some(index) = scenes
        .iter()
        .position(|scene| scene_name(scene).is_some_and(|scene| scene_key(scene) == key))
    else {
        report.skipped.push(name.to_string());
        return;
    };

    let actual = scene_name(&scenes[index]).unwrap_or(name).to_string();
    scenes.remove(index);

    seeded_names.retain(|seeded| scene_key(seeded) != key);
    report.removed.push(actual);
}

/// Index of the first Scene whose name matches `key` and that was seeded.
fn seeded_scene_index(scenes: &[Value], seeded_names: &[String], key: &str) -> Option<usize> {
    let seeded = seeded_names.iter().any(|name| scene_key(name) == key);

    if !seeded {
        return None;
    }

    scenes
        .iter()
        .position(|scene| scene_name(scene).is_some_and(|name| scene_key(name) == key))
}

/// Replace the `seeded_names` entry for `old_key` with `new_name`.
///
/// Duplicate old entries collapse to one; when the new name is already in
/// `seeded_names` (a Scene the user deleted after it was seeded) the old entry
/// is dropped instead of creating a second entry for the same name.
fn rename_seeded_entry(seeded_names: &mut Vec<String>, old_key: &str, new_name: &str) {
    let new_key = scene_key(new_name);
    let has_new = seeded_names.iter().any(|name| scene_key(name) == new_key);
    let mut replaced = false;

    seeded_names.retain_mut(|name| {
        if scene_key(name) != old_key {
            return true;
        }

        if has_new || replaced {
            return false;
        }

        *name = new_name.to_string();
        replaced = true;
        true
    });
}

#[cfg(test)]
mod tests {
    //! The mechanism is exercised with an explicit fixture table because the
    //! production table is empty (see the module docs).
    use super::*;
    use crate::default_scenes::{
        DEFAULTS_VERSION, LEGACY_DEFAULTS_VERSION, get_default_definition,
        get_default_scene_names_up_to,
    };
    use serde_json::json;

    /// A bump at the current defaults version renames a Scene that exists in
    /// the data (under its historical name) and retires a name that left the
    /// data; the next bump chains a second rename.
    const FIXTURE_RULES: &[DefaultSceneMigrationRule] = &[
        DefaultSceneMigrationRule {
            version: DEFAULTS_VERSION,
            renames: &[("Old Mirror Chamber", "Mirror Chamber")],
            removed: &["Old Retired Scene"],
        },
        DefaultSceneMigrationRule {
            version: DEFAULTS_VERSION + 1,
            renames: &[("Mirror Chamber", "Reflected Chamber")],
            removed: &[],
        },
    ];

    fn scene(name: &str, id: &str) -> Value {
        json!({
            "id": id,
            "name": name,
            "minSeconds": 8,
            "maxSeconds": 20,
            "groups": [{"name": "Background", "modules": []}],
        })
    }

    fn names(scenes: &[Value]) -> Vec<String> {
        scenes
            .iter()
            .map(|scene| scene["name"].as_str().unwrap_or_default().to_string())
            .collect()
    }

    fn migration(renames: &[(&str, &str)], removed: &[&str]) -> DefaultSceneMigration {
        DefaultSceneMigration {
            rename: renames
                .iter()
                .map(|(old, new)| ((*old).to_string(), (*new).to_string()))
                .collect(),
            remove: removed.iter().map(|name| (*name).to_string()).collect(),
        }
    }

    // ————————————————— version aggregation —————————————————

    #[test]
    fn aggregates_every_bump_in_the_range() {
        let aggregated = get_default_scene_migrations_with(
            FIXTURE_RULES,
            DEFAULTS_VERSION - 1,
            DEFAULTS_VERSION + 1,
        );

        assert_eq!(
            aggregated.rename,
            vec![
                (
                    "Old Mirror Chamber".to_string(),
                    "Mirror Chamber".to_string()
                ),
                (
                    "Mirror Chamber".to_string(),
                    "Reflected Chamber".to_string()
                ),
            ]
        );
        assert_eq!(aggregated.remove, vec!["Old Retired Scene".to_string()]);
    }

    #[test]
    fn excludes_bumps_at_or_before_the_from_version() {
        let aggregated = get_default_scene_migrations_with(
            FIXTURE_RULES,
            DEFAULTS_VERSION,
            DEFAULTS_VERSION + 1,
        );

        assert_eq!(
            aggregated.rename,
            vec![(
                "Mirror Chamber".to_string(),
                "Reflected Chamber".to_string()
            )]
        );
        assert!(aggregated.remove.is_empty());
    }

    #[test]
    fn returns_an_empty_migration_for_same_or_older_versions() {
        for (from, to) in [
            (DEFAULTS_VERSION, DEFAULTS_VERSION),
            (DEFAULTS_VERSION + 1, DEFAULTS_VERSION),
            (0, 0),
            (12, -1),
        ] {
            assert!(
                get_default_scene_migrations_with(FIXTURE_RULES, from, to).is_empty(),
                "{from}..{to}"
            );
        }

        assert!(get_default_scene_migrations(DEFAULTS_VERSION, DEFAULTS_VERSION).is_empty());
    }

    #[test]
    fn deduplicates_a_name_repeated_across_rules() {
        const REPEATED: &[DefaultSceneMigrationRule] = &[
            DefaultSceneMigrationRule {
                version: 8,
                renames: &[("Old", "New")],
                removed: &["Gone"],
            },
            DefaultSceneMigrationRule {
                version: 9,
                renames: &[(" old ", "New")],
                removed: &["gone"],
            },
        ];

        let aggregated = get_default_scene_migrations_with(REPEATED, 7, 9);

        assert_eq!(
            aggregated.rename,
            vec![("Old".to_string(), "New".to_string())]
        );
        assert_eq!(aggregated.remove, vec!["Gone".to_string()]);
    }

    #[test]
    fn the_production_table_is_consistent() {
        let mut last_version = LEGACY_DEFAULTS_VERSION;

        for rule in DEFAULT_SCENE_MIGRATIONS {
            assert!(
                rule.version >= last_version,
                "rules must be in ascending version order"
            );
            last_version = rule.version;

            for (old_name, new_name) in rule.renames {
                assert_ne!(scene_key(old_name), scene_key(new_name));
                assert!(
                    get_default_definition(old_name).is_none(),
                    "{old_name} was renamed but is still in the defaults"
                );
                assert!(
                    get_default_definition(new_name).is_some(),
                    "{new_name} is missing from the defaults"
                );
            }

            for name in rule.removed {
                assert!(
                    get_default_definition(name).is_none(),
                    "{name} was retired but is still in the defaults"
                );
            }
        }
    }

    // ————————————————— renames —————————————————

    #[test]
    fn renames_only_seeded_scenes() {
        let mut scenes = vec![
            scene("Old Mirror Chamber", "seeded-id"),
            scene("Smoke Room", "other-id"),
        ];
        let mut seeded = vec!["Old Mirror Chamber".to_string(), "Smoke Room".to_string()];

        let report = apply_default_scene_migrations(
            &mut scenes,
            &mut seeded,
            &migration(&[("Old Mirror Chamber", "Mirror Chamber")], &[]),
        );

        assert_eq!(
            names(&scenes),
            vec!["Mirror Chamber".to_string(), "Smoke Room".to_string()]
        );
        assert_eq!(
            report.renamed,
            vec![(
                "Old Mirror Chamber".to_string(),
                "Mirror Chamber".to_string()
            )]
        );
        assert!(report.removed.is_empty());
        assert!(report.skipped.is_empty());
    }

    #[test]
    fn leaves_a_user_scene_with_the_old_name_untouched() {
        let mut scenes = vec![scene("Old Mirror Chamber", "user-id")];
        let mut seeded = Vec::new();

        let report = apply_default_scene_migrations(
            &mut scenes,
            &mut seeded,
            &migration(&[("Old Mirror Chamber", "Mirror Chamber")], &[]),
        );

        assert_eq!(names(&scenes), vec!["Old Mirror Chamber".to_string()]);
        assert!(report.renamed.is_empty());
        assert_eq!(report.skipped, vec!["Old Mirror Chamber".to_string()]);
        assert!(seeded.is_empty());
    }

    #[test]
    fn a_rename_keeps_the_scene_id_and_payload() {
        let mut scenes = vec![scene("Old Mirror Chamber", "mirror-id")];
        let mut seeded = vec!["Old Mirror Chamber".to_string()];

        apply_default_scene_migrations(
            &mut scenes,
            &mut seeded,
            &migration(&[("Old Mirror Chamber", "Mirror Chamber")], &[]),
        );

        let renamed = &scenes[0];

        assert_eq!(renamed["id"], json!("mirror-id"));
        assert_eq!(renamed["name"], json!("Mirror Chamber"));
        assert_eq!(renamed["minSeconds"], json!(8));
        assert_eq!(
            renamed["groups"],
            scene("Mirror Chamber", "mirror-id")["groups"]
        );
    }

    #[test]
    fn a_rename_updates_seeded_names_and_is_idempotent() {
        let mut scenes = vec![scene("Old Mirror Chamber", "mirror-id")];
        let mut seeded = vec!["Old Mirror Chamber".to_string(), "Smoke Room".to_string()];
        let migration = migration(&[("Old Mirror Chamber", "Mirror Chamber")], &[]);

        let first = apply_default_scene_migrations(&mut scenes, &mut seeded, &migration);

        assert_eq!(
            seeded,
            vec!["Mirror Chamber".to_string(), "Smoke Room".to_string()]
        );

        let before_scenes = scenes.clone();
        let before_seeded = seeded.clone();
        let second = apply_default_scene_migrations(&mut scenes, &mut seeded, &migration);

        assert!(first.renamed.len() == 1);
        assert!(second.renamed.is_empty());
        assert_eq!(second.skipped, vec!["Old Mirror Chamber".to_string()]);
        assert_eq!(scenes, before_scenes);
        assert_eq!(seeded, before_seeded);
    }

    #[test]
    fn collapses_duplicate_seeded_names_when_the_new_name_is_already_seeded() {
        let mut scenes = vec![scene("Old Mirror Chamber", "mirror-id")];
        let mut seeded = vec![
            "Old Mirror Chamber".to_string(),
            "  mirror chamber ".to_string(),
        ];

        apply_default_scene_migrations(
            &mut scenes,
            &mut seeded,
            &migration(&[("Old Mirror Chamber", "Mirror Chamber")], &[]),
        );

        assert_eq!(seeded, vec!["  mirror chamber ".to_string()]);
    }

    #[test]
    fn skips_a_rename_whose_target_name_is_taken() {
        let mut scenes = vec![
            scene("Old Mirror Chamber", "seeded-id"),
            scene("Mirror Chamber", "user-id"),
        ];
        let mut seeded = vec!["Old Mirror Chamber".to_string()];

        let report = apply_default_scene_migrations(
            &mut scenes,
            &mut seeded,
            &migration(&[("Old Mirror Chamber", "Mirror Chamber")], &[]),
        );

        assert_eq!(
            names(&scenes),
            vec![
                "Old Mirror Chamber".to_string(),
                "Mirror Chamber".to_string()
            ]
        );
        assert_eq!(scenes[1]["id"], json!("user-id"));
        assert!(report.renamed.is_empty());
        assert_eq!(report.skipped, vec!["Old Mirror Chamber".to_string()]);
        assert_eq!(seeded, vec!["Old Mirror Chamber".to_string()]);
    }

    #[test]
    fn skips_a_case_only_rename_without_reporting_it() {
        let mut scenes = vec![scene("Mirror Chamber", "mirror-id")];
        let mut seeded = vec!["Mirror Chamber".to_string()];

        let report = apply_default_scene_migrations(
            &mut scenes,
            &mut seeded,
            &migration(&[("mirror chamber", "Mirror Chamber")], &[]),
        );

        assert!(report.renamed.is_empty());
        assert!(report.skipped.is_empty());
        assert_eq!(names(&scenes), vec!["Mirror Chamber".to_string()]);
    }

    #[test]
    fn chains_renames_across_bumps_in_order() {
        let aggregated = get_default_scene_migrations_with(
            FIXTURE_RULES,
            DEFAULTS_VERSION - 1,
            DEFAULTS_VERSION + 1,
        );
        let mut scenes = vec![scene("Old Mirror Chamber", "mirror-id")];
        let mut seeded = vec!["Old Mirror Chamber".to_string()];

        let report = apply_default_scene_migrations(&mut scenes, &mut seeded, &aggregated);

        assert_eq!(names(&scenes), vec!["Reflected Chamber".to_string()]);
        assert_eq!(seeded, vec!["Reflected Chamber".to_string()]);
        assert_eq!(
            report.renamed,
            vec![
                (
                    "Old Mirror Chamber".to_string(),
                    "Mirror Chamber".to_string()
                ),
                (
                    "Mirror Chamber".to_string(),
                    "Reflected Chamber".to_string()
                ),
            ]
        );
    }

    // ————————————————— removals —————————————————

    #[test]
    fn removes_seeded_scenes_and_their_seeded_names() {
        let mut scenes = vec![
            scene("Old Retired Scene", "retired-id"),
            scene("Smoke Room", "smoke-id"),
        ];
        let mut seeded = vec!["Old Retired Scene".to_string(), "Smoke Room".to_string()];

        let report = apply_default_scene_migrations(
            &mut scenes,
            &mut seeded,
            &migration(&[], &["Old Retired Scene"]),
        );

        assert_eq!(names(&scenes), vec!["Smoke Room".to_string()]);
        assert_eq!(seeded, vec!["Smoke Room".to_string()]);
        assert_eq!(report.removed, vec!["Old Retired Scene".to_string()]);
        assert!(report.renamed.is_empty());
        assert!(report.skipped.is_empty());
    }

    #[test]
    fn keeps_a_user_scene_that_shares_a_retired_name() {
        let mut scenes = vec![scene("Old Retired Scene", "user-id")];
        let mut seeded = Vec::new();

        let report = apply_default_scene_migrations(
            &mut scenes,
            &mut seeded,
            &migration(&[], &["Old Retired Scene"]),
        );

        assert_eq!(names(&scenes), vec!["Old Retired Scene".to_string()]);
        assert!(report.removed.is_empty());
        assert_eq!(report.skipped, vec!["Old Retired Scene".to_string()]);
    }

    #[test]
    fn removes_one_of_two_duplicate_seeded_scenes_and_keeps_the_other() {
        // The JS removal path removes **one** Scene (by id) per rule
        // (`scenes.js:3028-3047`) and splices one `seededDefaultScenes` entry.
        // A duplicate seeded Scene must not be collateral damage.
        let mut scenes = vec![
            scene("Old Retired Scene", "first-id"),
            scene("Old Retired Scene", "second-id"),
            scene("Smoke Room", "smoke-id"),
        ];
        let mut seeded = vec![
            "Old Retired Scene".to_string(),
            "Old Retired Scene".to_string(),
            "Smoke Room".to_string(),
        ];

        let report = apply_default_scene_migrations(
            &mut scenes,
            &mut seeded,
            &migration(&[], &["Old Retired Scene"]),
        );

        // Exactly one duplicate is gone; the survivor keeps its payload.
        assert_eq!(
            scenes
                .iter()
                .map(|scene| scene["id"].as_str().unwrap_or_default())
                .collect::<Vec<_>>(),
            vec!["second-id", "smoke-id"]
        );
        assert_eq!(report.removed, vec!["Old Retired Scene".to_string()]);
        // The retired name is no longer seeded, so the survivor is not removed
        // by a re-run (see `a_removal_is_idempotent_with_duplicate_seeded_scenes`).
        assert_eq!(seeded, vec!["Smoke Room".to_string()]);
    }

    #[test]
    fn a_removal_is_idempotent_with_duplicate_seeded_scenes() {
        let mut scenes = vec![
            scene("Old Retired Scene", "first-id"),
            scene("Old Retired Scene", "second-id"),
        ];
        let mut seeded = vec![
            "Old Retired Scene".to_string(),
            "Old Retired Scene".to_string(),
        ];
        let migration = migration(&[], &["Old Retired Scene"]);

        let first = apply_default_scene_migrations(&mut scenes, &mut seeded, &migration);
        assert_eq!(first.removed, vec!["Old Retired Scene".to_string()]);
        assert_eq!(scenes.len(), 1);

        // Re-running the same migration must not eat the surviving duplicate.
        let second = apply_default_scene_migrations(&mut scenes, &mut seeded, &migration);
        assert!(second.removed.is_empty());
        assert_eq!(second.skipped, vec!["Old Retired Scene".to_string()]);
        assert_eq!(scenes.len(), 1);
        assert_eq!(scenes[0]["id"], json!("second-id"));
        assert!(seeded.is_empty());
    }

    #[test]
    fn skips_removals_with_no_matching_scene() {
        let mut scenes = vec![scene("Smoke Room", "smoke-id")];
        let mut seeded = vec!["Smoke Room".to_string()];

        let report = apply_default_scene_migrations(
            &mut scenes,
            &mut seeded,
            &migration(&[], &["Does Not Exist"]),
        );

        assert_eq!(scenes.len(), 1);
        assert_eq!(seeded, vec!["Smoke Room".to_string()]);
        assert!(report.removed.is_empty());
        assert_eq!(report.skipped, vec!["Does Not Exist".to_string()]);
    }

    #[test]
    fn an_empty_migration_changes_nothing() {
        let mut scenes = vec![scene("Smoke Room", "smoke-id")];
        let mut seeded = vec!["Smoke Room".to_string()];
        let before = scenes.clone();

        let report = apply_default_scene_migrations(
            &mut scenes,
            &mut seeded,
            &DefaultSceneMigration::default(),
        );

        assert!(report == MigrationReport::default());
        assert_eq!(scenes, before);
        assert_eq!(seeded, vec!["Smoke Room".to_string()]);
    }

    // ————————————————— seeding integration —————————————————

    #[test]
    fn upgrading_from_the_previous_version_matches_a_fresh_install() {
        let previous_version = DEFAULTS_VERSION - 1;
        let mut seeded: Vec<String> = get_default_scene_names_up_to(previous_version)
            .into_iter()
            .map(|name| {
                if name == "Mirror Chamber" {
                    "Old Mirror Chamber".to_string()
                } else {
                    name
                }
            })
            .collect();
        seeded.push("Old Retired Scene".to_string());

        let mut scenes: Vec<Value> = seeded
            .iter()
            .enumerate()
            .map(|(index, name)| scene(name, &format!("id-{index}")))
            .collect();
        let renamed_index = scenes
            .iter()
            .position(|scene| scene["name"] == json!("Old Mirror Chamber"))
            .expect("historical Mirror Chamber");

        let plan = ensure_default_scene_migrations_with(
            FIXTURE_RULES,
            previous_version,
            DEFAULTS_VERSION,
            &mut scenes,
            &mut seeded,
        );

        assert_eq!(
            plan.report.renamed,
            vec![(
                "Old Mirror Chamber".to_string(),
                "Mirror Chamber".to_string()
            )]
        );
        assert_eq!(plan.report.removed, vec!["Old Retired Scene".to_string()]);
        assert_eq!(
            scenes[renamed_index]["id"],
            json!(format!("id-{renamed_index}"))
        );

        // The caller builds the seeds and marks them seeded; afterwards the
        // name set must equal a fresh install, with no duplicates.
        let mut upgraded = names(&scenes);
        upgraded.extend(plan.seeds.iter().map(|definition| definition.name.clone()));
        let mut upgraded_keys: Vec<String> = upgraded.iter().map(|name| scene_key(name)).collect();
        upgraded_keys.sort();

        let mut fresh: Vec<String> = get_default_scene_names_up_to(DEFAULTS_VERSION)
            .iter()
            .map(|name| scene_key(name))
            .collect();
        fresh.sort();

        assert_eq!(upgraded_keys, fresh);
        assert_eq!(
            upgraded_keys.iter().collect::<HashSet<_>>().len(),
            upgraded_keys.len()
        );
        assert_eq!(
            upgraded_keys.len(),
            crate::default_scenes::default_scenes().len()
        );
        assert!(
            !plan
                .seeds
                .iter()
                .any(|definition| { definition.introduced_version <= previous_version })
        );
    }

    #[test]
    fn re_running_the_upgrade_migration_seeds_and_changes_nothing() {
        // Already upgraded: the renamed Scene carries the new name and the
        // retired one is gone, so a second pass over the same span is inert.
        let mut scenes = vec![
            scene("Mirror Chamber", "mirror-id"),
            scene("Smoke Room", "smoke-id"),
        ];
        let mut seeded = vec!["Mirror Chamber".to_string(), "Smoke Room".to_string()];
        let before = scenes.clone();

        let plan = ensure_default_scene_migrations_with(
            FIXTURE_RULES,
            DEFAULTS_VERSION - 1,
            DEFAULTS_VERSION,
            &mut scenes,
            &mut seeded,
        );

        assert_eq!(
            plan.migration,
            migration(
                &[("Old Mirror Chamber", "Mirror Chamber")],
                &["Old Retired Scene"]
            )
        );
        assert!(plan.report.renamed.is_empty());
        assert!(plan.report.removed.is_empty());
        assert_eq!(plan.report.skipped.len(), 2);
        assert_eq!(scenes, before);
        assert_eq!(
            seeded,
            vec!["Mirror Chamber".to_string(), "Smoke Room".to_string()]
        );
        // Seeding is idempotent too: the Scene introduced in this bump was
        // never built here, so it is still offered (and the caller marks it
        // seeded), but no seeded name is duplicated.
        assert!(plan.seeds.iter().all(|definition| {
            !seeded
                .iter()
                .any(|name| scene_key(name) == scene_key(&definition.name))
        }));
    }

    #[test]
    fn the_production_path_is_a_no_op_until_a_rule_ships() {
        let mut scenes = vec![scene("Smoke Room", "smoke-id")];
        let mut seeded = vec!["Smoke Room".to_string()];

        let plan = ensure_default_scene_migration(0, DEFAULTS_VERSION, &mut scenes, &mut seeded);

        assert!(plan.migration.is_empty());
        assert!(plan.report.renamed.is_empty());
        assert!(plan.report.removed.is_empty());
        assert!(plan.report.skipped.is_empty());
        assert_eq!(scenes.len(), 1);
        assert_eq!(seeded, vec!["Smoke Room".to_string()]);
    }
}
