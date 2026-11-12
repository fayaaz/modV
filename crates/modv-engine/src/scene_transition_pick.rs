//! Crossfade style selection for Scene changes.
//!
//! Ported from `src/renderer/src/application/worker/scene-transition-pick.js`
//! (36 lines). The rule that matters (WS-8, "Transition selection"): while
//! performance mode is running **every** cell change is a straight crossfade,
//! regardless of the allowed-transitions list, so automatic transitions are
//! never sudden `None` switches or jarring shader wipes. The allowed list only
//! applies to manual Scene changes made while performance mode is stopped.
//!
//! The transition names themselves come from
//! `src/renderer/src/application/scene-transition-defs.js`: [`FADE_TRANSITION`]
//! and [`NONE_TRANSITION`] plus the six native shader transitions
//! ([`NATIVE_TRANSITION_NAMES`]). The ISF shader bodies are WS-4 scope and are
//! not duplicated here.
//!
//! Line numbers refer to the tree at commit `8a512b6e` (branch `rust-hybrid`).

/// The crossfade every Scene change falls back to (`scene-transition-defs.js:1`).
pub const FADE_TRANSITION: &str = "Fade";

/// The instant switch a user may allow for manual changes
/// (`scene-transition-defs.js:2`).
pub const NONE_TRANSITION: &str = "None";

/// The native shader transitions registered by the app
/// (`scene-transition-defs.js:19-125`).
pub const NATIVE_TRANSITION_NAMES: &[&str] = &[
    "Three Crossfade",
    "Three Dissolve",
    "Three Pixelate",
    "Three Glitch",
    "Three Wipe",
    "Three Circle",
];

/// `pickTransitionName` (`scene-transition-pick.js:18-36`).
///
/// - `performance_enabled` → always [`FADE_TRANSITION`] (no RNG call).
/// - otherwise the allowed list is filtered to names that actually exist;
///   an empty result also fades, and a valid list is picked at random.
///
/// `random` is only called when a real pick happens, matching the JS
/// `random()` call sites.
pub fn pick_transition_name<F>(
    performance_enabled: bool,
    allowed_transitions: &[String],
    available_transition_names: &[String],
    mut random: F,
) -> String
where
    F: FnMut() -> f64,
{
    // scene-transition-pick.js:24-26
    if performance_enabled {
        return FADE_TRANSITION.to_string();
    }

    // scene-transition-pick.js:28-33
    let allowed: Vec<&String> = allowed_transitions
        .iter()
        .filter(|name| available_transition_names.contains(name))
        .collect();

    if allowed.is_empty() {
        return FADE_TRANSITION.to_string();
    }

    // scene-transition-pick.js:35. Out-of-range rolls clamp to a valid entry;
    // a real `Math.random` roll is always in `[0, 1)`.
    let index = (random() * allowed.len() as f64).floor();
    let index = index.clamp(0.0, (allowed.len() - 1) as f64) as usize;

    allowed[index].clone()
}

#[cfg(test)]
mod tests {
    //! Port of `tests/unit/scene-transition-pick.test.js` (Vitest).
    use super::*;

    fn owned(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn always_fades_while_performance_runs_even_with_none_or_a_shader_allowed() {
        // scene-transition-pick.test.js:6-33
        assert_eq!(
            pick_transition_name(
                true,
                &owned(&["None"]),
                &owned(&[FADE_TRANSITION, "None"]),
                || 0.0,
            ),
            FADE_TRANSITION
        );
        assert_eq!(
            pick_transition_name(
                true,
                &owned(&["Three Wipe"]),
                &owned(&[FADE_TRANSITION, "None", "Three Wipe"]),
                || 0.0,
            ),
            FADE_TRANSITION
        );
        assert_eq!(
            pick_transition_name(true, &[], &owned(&[FADE_TRANSITION]), || 0.0),
            FADE_TRANSITION
        );
    }

    #[test]
    fn does_not_roll_the_dice_while_performance_runs() {
        // scene-transition-pick.test.js:35-46
        let mut calls = 0;
        pick_transition_name(
            true,
            &owned(&["Three Wipe"]),
            &owned(&[FADE_TRANSITION, "Three Wipe"]),
            || {
                calls += 1;
                0.0
            },
        );
        assert_eq!(calls, 0);
    }

    #[test]
    fn picks_from_the_allowed_list_when_performance_is_off() {
        // scene-transition-pick.test.js:48-57
        assert_eq!(
            pick_transition_name(
                false,
                &owned(&["Three Wipe"]),
                &owned(&[FADE_TRANSITION, "None", "Three Wipe"]),
                || 0.0,
            ),
            "Three Wipe"
        );
    }

    #[test]
    fn uses_the_random_roll_to_pick_when_performance_is_off() {
        // scene-transition-pick.test.js:59-72
        let names = owned(&["A", "B", "C", "D"]);
        let mut calls = 0;
        let picked = pick_transition_name(false, &names, &names, || {
            calls += 1;
            0.6
        });
        assert_eq!(picked, "C");
        assert_eq!(calls, 1);
    }

    #[test]
    fn drops_allowed_names_that_are_not_available() {
        // scene-transition-pick.test.js:74-83
        assert_eq!(
            pick_transition_name(
                false,
                &owned(&["E2E Missing Transition"]),
                &owned(&[FADE_TRANSITION, "None"]),
                || 0.0,
            ),
            FADE_TRANSITION
        );
    }

    #[test]
    fn fades_when_nothing_is_allowed_while_performance_is_off() {
        // scene-transition-pick.test.js:85-94
        assert_eq!(
            pick_transition_name(
                false,
                &[],
                &owned(&[FADE_TRANSITION, "None", "Three Wipe"]),
                || 0.5,
            ),
            FADE_TRANSITION
        );
    }

    #[test]
    fn transition_constants_match_the_defs_module() {
        // scene-transition-defs.js:1-2, 19-125
        assert_eq!(FADE_TRANSITION, "Fade");
        assert_eq!(NONE_TRANSITION, "None");
        assert_eq!(NATIVE_TRANSITION_NAMES.len(), 6);
        assert_eq!(NATIVE_TRANSITION_NAMES[0], "Three Crossfade");
        assert_eq!(NATIVE_TRANSITION_NAMES[5], "Three Circle");
    }
}
