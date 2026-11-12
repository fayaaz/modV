//! Default input links / tweens a Scene receives.
//!
//! Ported from `src/renderer/src/application/worker/scene-link-defaults.js` (73
//! lines) plus the pure range/link builders from `store/modules/scenes.js`
//! (`getPropRange`, `getBassLinkRange`, `getBpmLinkRange`, `createBassLink`,
//! `createBpmTweenLink`).
//!
//! The automatic BPM Tween deliberately runs slowly and eases in/out so props
//! move musically instead of snapping ([`DEFAULT_SCENE_BPM_TWEEN`]). Structural
//! geometry props are excluded per Module so a Scene falls back to a Group
//! alpha (or another Module) rather than warping its shape on every beat
//! ([`AUTO_BASS_EXCLUDED_PROPS`] / [`AUTO_BPM_EXCLUDED_PROPS`], WS-8
//! "Automatic Scene links").
//!
//! Line numbers refer to the tree at commit `8a512b6e` (branch `rust-hybrid`).

use serde_json::{Value, json};

use crate::scene_timing::to_finite_number;

/// The automatic BPM tween settings (`scene-link-defaults.js:7-10`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DefaultBpmTween {
    /// BPM subdivision the tween advances on (`bpmDivision: 32`).
    pub bpm_division: i64,
    /// anime.js easing name (`easing: "easeInOutSine"`).
    pub easing: &'static str,
}

/// `DEFAULT_SCENE_BPM_TWEEN` (`scene-link-defaults.js:7-10`).
pub const DEFAULT_SCENE_BPM_TWEEN: DefaultBpmTween = DefaultBpmTween {
    bpm_division: 32,
    easing: "easeInOutSine",
};

/// Props excluded from automatic **bass** links (`scene-link-defaults.js:12-16`).
pub const AUTO_BASS_EXCLUDED_PROPS: &[(&str, &[&str])] = &[
    ("Twirl", &["radius", "amount"]),
    ("Kaleidoscope", &["sides"]),
    ("Kaleidoscope Tile", &["sides", "size", "rotation", "angle"]),
];

/// Props excluded from automatic **BPM** links (`scene-link-defaults.js:18-22`).
pub const AUTO_BPM_EXCLUDED_PROPS: &[(&str, &[&str])] = &[
    ("Twirl", &["radius", "amount"]),
    ("Kaleidoscope", &["sides"]),
    ("Kaleidoscope Tile", &["sides", "size", "angle"]),
];

/// The two automatic link modes (`scene-link-defaults.js:24-27`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkMode {
    /// Meyda feature link (bass by default).
    Bass,
    /// BPM tween link.
    Bpm,
}

impl LinkMode {
    /// The JS mode token (`"bass"` / `"bpm"`).
    pub fn as_str(self) -> &'static str {
        match self {
            LinkMode::Bass => "bass",
            LinkMode::Bpm => "bpm",
        }
    }

    /// Parse a JS mode token; unknown tokens are `None`, matching
    /// `isAutoLinkExcluded`'s "unknown mode never excludes" behaviour.
    pub fn from_token(token: &str) -> Option<Self> {
        match token {
            "bass" => Some(LinkMode::Bass),
            "bpm" => Some(LinkMode::Bpm),
            _ => None,
        }
    }

    /// The exclusion table for this mode.
    pub fn excluded_props(self) -> &'static [(&'static str, &'static [&'static str])] {
        match self {
            LinkMode::Bass => AUTO_BASS_EXCLUDED_PROPS,
            LinkMode::Bpm => AUTO_BPM_EXCLUDED_PROPS,
        }
    }
}

/// A link's min/max range (`scenes.js:611-616` shape).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LinkRange {
    /// Lower bound written to the input link.
    pub min: f64,
    /// Upper bound written to the input link.
    pub max: f64,
}

impl Default for LinkRange {
    fn default() -> Self {
        LinkRange { min: 0.0, max: 1.0 }
    }
}

/// `getBpmDivision` (`scene-link-defaults.js:33-41`): a positive integer wins,
/// everything else falls back to [`DEFAULT_SCENE_BPM_TWEEN`].
pub fn get_bpm_division(bpm_division: Option<&Value>) -> i64 {
    match to_finite_number(bpm_division) {
        Some(value) if value.fract() == 0.0 && value >= 1.0 => value as i64,
        _ => DEFAULT_SCENE_BPM_TWEEN.bpm_division,
    }
}

/// `getFeatureSmoothing` (`scene-link-defaults.js:47-55`): only values in
/// `(0, 1]` are usable.
pub fn get_feature_smoothing(smoothing: Option<&Value>) -> Option<f64> {
    match to_finite_number(smoothing) {
        Some(value) if value > 0.0 && value <= 1.0 => Some(value),
        _ => None,
    }
}

/// The excluded prop names for a module in a mode, if any.
pub fn excluded_props_for(module_name: &str, mode: LinkMode) -> Option<&'static [&'static str]> {
    mode.excluded_props()
        .iter()
        .find(|(name, _)| *name == module_name)
        .map(|(_, props)| *props)
}

/// `isAutoLinkExcluded` (`scene-link-defaults.js:63-73`): a missing module
/// name or an unknown mode never excludes anything.
pub fn is_auto_link_excluded(module_name: Option<&str>, prop_name: &str, mode: &str) -> bool {
    let (Some(module_name), Some(mode)) = (module_name, LinkMode::from_token(mode)) else {
        return false;
    };

    if module_name.is_empty() {
        return false;
    }

    excluded_props_for(module_name, mode)
        .map(|props| props.contains(&prop_name))
        .unwrap_or(false)
}

/// `getPropRange` (`scenes.js:611-616`): the prop's declared `min`/`max`, or
/// `0..1` when it does not declare a usable ascending range.
pub fn get_prop_range(module: &Value, prop_name: &str) -> LinkRange {
    let prop = module.get("$props").and_then(|props| props.get(prop_name));
    let min = to_finite_number(prop.and_then(|prop| prop.get("min")));
    let max = to_finite_number(prop.and_then(|prop| prop.get("max")));

    match (min, max) {
        (Some(min), Some(max)) if max > min => LinkRange { min, max },
        _ => LinkRange { min: 0.0, max: 1.0 },
    }
}

/// `clamp` (`scenes.js:600-602`).
fn clamp(value: f64, min: f64, max: f64) -> f64 {
    value.max(min).min(max)
}

/// The current numeric prop value the way JS `Number(module.props[name])`
/// reads it (`scenes.js:621,641`).
fn current_prop_value(module: &Value, prop_name: &str) -> Option<f64> {
    to_finite_number(module.get("props").and_then(|props| props.get(prop_name)))
}

/// `getBassLinkRange` (`scenes.js:618-637`).
///
/// A positive current value gets a `0.6×..1.4×` bracket (clamped to the prop
/// range); otherwise the link breathes over the lower third of the range.
pub fn get_bass_link_range(module: &Value, prop_name: &str) -> LinkRange {
    let LinkRange { min, max } = get_prop_range(module, prop_name);
    let span = max - min;
    let current = current_prop_value(module, prop_name);

    let (link_min, link_max) = match current {
        Some(current) if current > 0.0 => (
            clamp(current * 0.6, min, max),
            clamp(current * 1.4, min, max),
        ),
        _ => (min + span * 0.05, min + span * 0.35),
    };

    if link_max <= link_min {
        return LinkRange { min, max };
    }

    LinkRange {
        min: link_min,
        max: link_max,
    }
}

/// `getBpmLinkRange` (`scenes.js:639-658`).
///
/// A finite current value gets a `±20 % of span` bracket; otherwise the whole
/// prop range is used.
pub fn get_bpm_link_range(module: &Value, prop_name: &str) -> LinkRange {
    let LinkRange { min, max } = get_prop_range(module, prop_name);
    let span = max - min;
    let current = current_prop_value(module, prop_name);

    let (link_min, link_max) = match current {
        Some(current) => (
            clamp(current - span * 0.2, min, max),
            clamp(current + span * 0.2, min, max),
        ),
        None => (min, max),
    };

    if link_max <= link_min {
        return LinkRange { min, max };
    }

    LinkRange {
        min: link_min,
        max: link_max,
    }
}

/// A resolved default input link (`createBassLink` / `createBpmTweenLink`,
/// `scenes.js:757-819`).
#[derive(Clone, Debug, PartialEq)]
pub struct DefaultInputLink {
    /// `"getter"` for a meyda feature link, `"state"` for a tween link.
    pub r#type: &'static str,
    /// Vuex location the link reads (`meyda/getFeature` or the tween value).
    pub location: String,
    /// `"meyda"` or `"tween"`.
    pub source: &'static str,
    /// The meyda feature for a bass link (`bass`, `mid`, `treble`, ...).
    pub feature: Option<String>,
    /// The link range.
    pub range: LinkRange,
}

/// Builds the default input link for a Scene control
/// (`scenes.js:757-819`).
///
/// - [`LinkMode::Bass`]: a `getter` link on `meyda/getFeature` for `feature`
///   (default `"bass"`), over `range` or the module-less `0.55..1` default.
/// - [`LinkMode::Bpm`]: a `state` link on `tweens.tweens['<input_id>'].value[0]`
///   sourced from the tween, over `range` or the module-less `0.4..1` default.
pub fn default_link_for_input(
    mode: LinkMode,
    input_id: &str,
    feature: Option<&str>,
    range: Option<LinkRange>,
) -> DefaultInputLink {
    match mode {
        LinkMode::Bass => DefaultInputLink {
            r#type: "getter",
            location: "meyda/getFeature".to_string(),
            source: "meyda",
            feature: Some(feature.unwrap_or("bass").to_string()),
            range: range.unwrap_or(LinkRange {
                min: 0.55,
                max: 1.0,
            }),
        },
        LinkMode::Bpm => DefaultInputLink {
            r#type: "state",
            location: format!("tweens.tweens['{input_id}'].value[0]"),
            source: "tween",
            feature: None,
            range: range.unwrap_or(LinkRange { min: 0.4, max: 1.0 }),
        },
    }
}

/// The default BPM tween descriptor (`createBpmTweenLink`'s
/// `dataTypes/createType` args, `scenes.js:806-817`).
///
/// The tween loops, alternates direction and advances on
/// [`DEFAULT_SCENE_BPM_TWEEN::bpm_division`] rather than wall-clock time.
pub fn default_link_tween(input_id: &str, bpm_division: Option<&Value>) -> Value {
    json!({
        "id": input_id,
        "data": [[0], [1]],
        "duration": 1000,
        "direction": "alternate",
        "easing": DEFAULT_SCENE_BPM_TWEEN.easing,
        "loop": true,
        "useBpm": true,
        "bpmDivision": get_bpm_division(bpm_division),
        "durationAsTotalTime": false,
        "steps": 0,
    })
}

#[cfg(test)]
mod tests {
    //! Port of `tests/unit/scene-link-defaults.test.js` (Vitest) plus the pure
    //! range builders from `scenes.js`.
    use super::*;

    // ————————————————— defaults —————————————————

    #[test]
    fn runs_the_automatic_bpm_tween_slowly_with_a_sine_ease() {
        // scene-link-defaults.test.js:12-16
        assert_eq!(DEFAULT_SCENE_BPM_TWEEN.bpm_division, 32);
        assert_eq!(DEFAULT_SCENE_BPM_TWEEN.easing, "easeInOutSine");
    }

    #[test]
    fn excludes_twirl_geometry_props_for_bass_and_bpm() {
        // scene-link-defaults.test.js:18-24
        assert!(is_auto_link_excluded(Some("Twirl"), "radius", "bass"));
        assert!(is_auto_link_excluded(Some("Twirl"), "amount", "bass"));
        assert!(is_auto_link_excluded(Some("Twirl"), "radius", "bpm"));
        assert!(is_auto_link_excluded(Some("Twirl"), "amount", "bpm"));
        assert!(!is_auto_link_excluded(Some("Twirl"), "center", "bass"));
    }

    #[test]
    fn excludes_kaleidoscope_sides_but_not_unrelated_props() {
        // scene-link-defaults.test.js:26-30
        assert!(is_auto_link_excluded(Some("Kaleidoscope"), "sides", "bass"));
        assert!(is_auto_link_excluded(Some("Kaleidoscope"), "sides", "bpm"));
        assert!(!is_auto_link_excluded(
            Some("Kaleidoscope"),
            "rotation",
            "bpm"
        ));
    }

    #[test]
    fn only_allows_kaleidoscope_tile_rotation_for_bpm() {
        // scene-link-defaults.test.js:32-43
        for prop in ["sides", "size", "rotation", "angle"] {
            assert!(
                is_auto_link_excluded(Some("Kaleidoscope Tile"), prop, "bass"),
                "{prop} should be excluded for bass"
            );
        }

        assert!(is_auto_link_excluded(
            Some("Kaleidoscope Tile"),
            "sides",
            "bpm"
        ));
        assert!(is_auto_link_excluded(
            Some("Kaleidoscope Tile"),
            "size",
            "bpm"
        ));
        assert!(is_auto_link_excluded(
            Some("Kaleidoscope Tile"),
            "angle",
            "bpm"
        ));
        assert!(!is_auto_link_excluded(
            Some("Kaleidoscope Tile"),
            "rotation",
            "bpm"
        ));
    }

    #[test]
    fn leaves_unrelated_modules_and_unknown_modes_unaffected() {
        // scene-link-defaults.test.js:45-50
        assert!(!is_auto_link_excluded(Some("Bloom"), "intensity", "bass"));
        assert!(!is_auto_link_excluded(Some("Bloom"), "blurAmount", "bpm"));
        assert!(!is_auto_link_excluded(Some("Twirl"), "radius", "explode"));
        assert!(!is_auto_link_excluded(None, "radius", "bpm"));
        assert!(!is_auto_link_excluded(Some(""), "radius", "bpm"));
    }

    #[test]
    fn keeps_the_exported_maps_keyed_by_module_name() {
        // scene-link-defaults.test.js:52-59
        assert!(
            excluded_props_for("Twirl", LinkMode::Bass)
                .unwrap()
                .contains(&"radius")
        );
        assert!(
            excluded_props_for("Twirl", LinkMode::Bpm)
                .unwrap()
                .contains(&"amount")
        );
        assert!(
            excluded_props_for("Kaleidoscope Tile", LinkMode::Bass)
                .unwrap()
                .contains(&"rotation")
        );
        assert!(
            !excluded_props_for("Kaleidoscope Tile", LinkMode::Bpm)
                .unwrap()
                .contains(&"rotation")
        );
        assert!(excluded_props_for("Bloom", LinkMode::Bass).is_none());
    }

    // ————————————————— link options —————————————————

    #[test]
    fn keeps_an_explicit_integer_bpm_tween_division() {
        // scene-link-defaults.test.js:63-67
        assert_eq!(get_bpm_division(Some(&json!(128))), 128);
        assert_eq!(get_bpm_division(Some(&json!(8))), 8);
        assert_eq!(get_bpm_division(Some(&json!(1))), 1);
        // `Number("128")` is an integer in JS.
        assert_eq!(get_bpm_division(Some(&json!("128"))), 128);
    }

    #[test]
    fn falls_back_to_the_default_division_for_unusable_input() {
        // scene-link-defaults.test.js:69-76
        assert_eq!(get_bpm_division(None), 32);
        assert_eq!(get_bpm_division(Some(&json!("NaN"))), 32);
        assert_eq!(get_bpm_division(Some(&json!(0))), 32);
        assert_eq!(get_bpm_division(Some(&json!(-4))), 32);
        assert_eq!(get_bpm_division(Some(&json!(3.5))), 32);
        assert_eq!(get_bpm_division(Some(&json!("abc"))), 32);
    }

    #[test]
    fn keeps_in_range_feature_smoothing_values() {
        // scene-link-defaults.test.js:78-85
        assert_eq!(
            get_feature_smoothing(Some(&json!(0.3486748374187093))),
            Some(0.3486748374187093)
        );
        assert_eq!(get_feature_smoothing(Some(&json!(1))), Some(1.0));
        assert_eq!(get_feature_smoothing(Some(&json!(0.001))), Some(0.001));
    }

    #[test]
    fn rejects_unusable_smoothing_values() {
        // scene-link-defaults.test.js:87-94
        assert_eq!(get_feature_smoothing(None), None);
        assert_eq!(get_feature_smoothing(Some(&json!("NaN"))), None);
        assert_eq!(get_feature_smoothing(Some(&json!(0))), None);
        assert_eq!(get_feature_smoothing(Some(&json!(-0.5))), None);
        assert_eq!(get_feature_smoothing(Some(&json!(2))), None);
        assert_eq!(get_feature_smoothing(Some(&json!("abc"))), None);
    }

    // ————————————————— ranges —————————————————

    #[test]
    fn prop_range_uses_declared_bounds_and_falls_back_to_zero_one() {
        // scenes.js:611-616
        let module = json!({"$props": {"gain": {"min": 0.5, "max": 2}}});
        assert_eq!(
            get_prop_range(&module, "gain"),
            LinkRange { min: 0.5, max: 2.0 }
        );

        let module = json!({"$props": {"gain": {"min": 2, "max": 1}}});
        assert_eq!(
            get_prop_range(&module, "gain"),
            LinkRange { min: 0.0, max: 1.0 }
        );
        assert_eq!(
            get_prop_range(&json!({}), "missing"),
            LinkRange { min: 0.0, max: 1.0 }
        );
    }

    #[test]
    fn bass_link_range_brackets_a_positive_current_value() {
        // scenes.js:618-637
        let module = json!({
            "$props": {"intensity": {"min": 0, "max": 10}},
            "props": {"intensity": 5},
        });
        assert_eq!(
            get_bass_link_range(&module, "intensity"),
            LinkRange { min: 3.0, max: 7.0 }
        );
    }

    #[test]
    fn bass_link_range_uses_the_lower_third_without_a_current_value() {
        // scenes.js:629-635
        let module = json!({"$props": {"intensity": {"min": 0, "max": 10}}});
        assert_eq!(
            get_bass_link_range(&module, "intensity"),
            LinkRange { min: 0.5, max: 3.5 }
        );
    }

    #[test]
    fn bpm_link_range_brackets_a_known_value_by_a_fifth_of_the_span() {
        // scenes.js:639-658
        let module = json!({
            "$props": {"blur": {"min": 0, "max": 10}},
            "props": {"blur": 5},
        });
        assert_eq!(
            get_bpm_link_range(&module, "blur"),
            LinkRange { min: 3.0, max: 7.0 }
        );

        let module = json!({"$props": {"blur": {"min": 0, "max": 10}}});
        assert_eq!(
            get_bpm_link_range(&module, "blur"),
            LinkRange {
                min: 0.0,
                max: 10.0
            }
        );
    }

    #[test]
    fn link_ranges_collapse_to_the_prop_range_when_the_bracket_is_empty() {
        // scenes.js:634-636, 655-657
        let module = json!({
            "$props": {"gain": {"min": 0, "max": 1}},
            "props": {"gain": 100},
        });
        assert_eq!(
            get_bpm_link_range(&module, "gain"),
            LinkRange { min: 0.0, max: 1.0 }
        );
    }

    // ————————————————— default links —————————————————

    #[test]
    fn bass_link_is_a_meyda_getter_with_the_default_range() {
        // scenes.js:757-780
        let link = default_link_for_input(LinkMode::Bass, "in-1", None, None);
        assert_eq!(link.r#type, "getter");
        assert_eq!(link.location, "meyda/getFeature");
        assert_eq!(link.source, "meyda");
        assert_eq!(link.feature.as_deref(), Some("bass"));
        assert_eq!(
            link.range,
            LinkRange {
                min: 0.55,
                max: 1.0
            }
        );
    }

    #[test]
    fn bass_link_keeps_a_requested_feature_and_range() {
        // scenes.js:757-780 (`feature = "bass"`, `range` override)
        let link = default_link_for_input(
            LinkMode::Bass,
            "in-1",
            Some("treble"),
            Some(LinkRange { min: 0.0, max: 2.0 }),
        );
        assert_eq!(link.feature.as_deref(), Some("treble"));
        assert_eq!(link.range, LinkRange { min: 0.0, max: 2.0 });
    }

    #[test]
    fn bpm_link_is_a_tween_state_link_on_the_input() {
        // scenes.js:790-819
        let link = default_link_for_input(LinkMode::Bpm, "in-42", Some("bass"), None);
        assert_eq!(link.r#type, "state");
        assert_eq!(link.location, "tweens.tweens['in-42'].value[0]");
        assert_eq!(link.source, "tween");
        assert_eq!(link.feature, None);
        assert_eq!(link.range, LinkRange { min: 0.4, max: 1.0 });
    }

    #[test]
    fn link_mode_tokens_round_trip() {
        assert_eq!(LinkMode::Bass.as_str(), "bass");
        assert_eq!(LinkMode::Bpm.as_str(), "bpm");
        assert_eq!(LinkMode::from_token("bass"), Some(LinkMode::Bass));
        assert_eq!(LinkMode::from_token("bpm"), Some(LinkMode::Bpm));
        assert_eq!(LinkMode::from_token("explode"), None);
    }

    #[test]
    fn default_link_tween_matches_create_bpm_tween_link_args() {
        // scenes.js:806-817
        let tween = default_link_tween("in-7", None);
        assert_eq!(tween["id"], json!("in-7"));
        assert_eq!(tween["data"], json!([[0], [1]]));
        assert_eq!(tween["duration"], json!(1000));
        assert_eq!(tween["direction"], json!("alternate"));
        assert_eq!(tween["easing"], json!("easeInOutSine"));
        assert_eq!(tween["loop"], json!(true));
        assert_eq!(tween["useBpm"], json!(true));
        assert_eq!(tween["bpmDivision"], json!(32));
        assert_eq!(tween["durationAsTotalTime"], json!(false));
        assert_eq!(tween["steps"], json!(0));

        let tween = default_link_tween("in-7", Some(&json!(16)));
        assert_eq!(tween["bpmDivision"], json!(16));
    }
}
