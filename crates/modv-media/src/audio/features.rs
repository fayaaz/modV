//! Feature smoothing helpers, ported from
//! `src/renderer/src/application/worker/audio-features.js` (48 LOC) together with
//! its single smoothing primitive
//! (`src/renderer/src/application/utils/lerp.js` — `lerp(start, end, amt) = (1 - amt) * start + amt * end`).
//!
//! Differences from the JS source, all intentional:
//!
//! * The JS module-level singletons (`features` / `smoothedFeatures`,
//!   audio-features.js:6-7) become an explicit [`SmoothedFeatures`] value so
//!   Rust callers can own their state instead of sharing hidden globals. The
//!   JS exports map 1:1 onto methods.
//! * The JS `smoothed[id] || 0` fallback (audio-features.js:31) — which treats
//!   a stored `0` and a missing entry identically — becomes
//!   `unwrap_or(0.0)`.
//! * A missing `features[feature]` made the JS `lerp` produce `NaN` (multiply by
//!   `undefined`). This port feeds `f64::NAN` as the lerp target, so the
//!   smoothed value becomes `NaN` exactly as in JS instead of panicking.
//! * `get_feature` returns `Option<f64>` (JS `undefined` for a missing key).
//! * Keys are `String`-keyed maps (JS objects with string keys).

use std::collections::HashMap;

/// JS `lerp` (utils/lerp.js:1-3): `(1 - amt) * start + amt * end`.
fn lerp(start: f64, end: f64, amt: f64) -> f64 {
    (1.0 - amt) * start + amt * end
}

/// Maximum smoothing value accepted by the Audio/Video panel slider; the step is
/// one `SMOOTHING_STEP` below this so the slider never hits the unsmoothed edge
/// (audio-features.js:3; consumed by
/// `src/renderer/src/components/InputLinkComponents/AudioFeatures.vue:37-38`).
pub const MAX_SMOOTHING: f64 = 1.0;

/// Slider step for the smoothing control (audio-features.js:4).
pub const SMOOTHING_STEP: f64 = 0.001;

/// Per-consumer smoothed-feature registry. Mirrors the JS module singleton
/// (audio-features.js:6-48); construct one per consumer group instead of
/// sharing global state.
#[derive(Debug, Default)]
pub struct SmoothedFeatures {
    features: HashMap<String, f64>,
    smoothed: HashMap<String, f64>,
}

impl SmoothedFeatures {
    /// Creates an empty registry (JS module initialisation,
    /// audio-features.js:6-7).
    pub fn new() -> Self {
        Self::default()
    }

    /// All features currently set (audio-features.js:9-11 — `getFeatures`).
    pub fn get_features(&self) -> &HashMap<String, f64> {
        &self.features
    }

    /// One feature by key, or `None` when unset (audio-features.js:13-15 —
    /// `getFeature`).
    pub fn get_feature(&self, key: &str) -> Option<f64> {
        self.features.get(key).copied()
    }

    /// Replaces the feature map wholesale (audio-features.js:17-19 —
    /// `setFeatures`).
    pub fn set_features(&mut self, features: HashMap<String, f64>) {
        self.features = features;
    }

    /// Registers a smoothing id with an initial smoothed value of 0
    /// (audio-features.js:21-23 — `addSmoothingId`).
    pub fn add_smoothing_id(&mut self, id: &str) {
        self.smoothed.insert(id.to_string(), 0.0);
    }

    /// Drops a smoothing id; unknown ids are a no-op like the JS `delete`
    /// (audio-features.js:25-27 — `removeSmoothingId`).
    pub fn remove_smoothing_id(&mut self, id: &str) {
        self.smoothed.remove(id);
    }

    /// Lerps the smoothed value for `id` towards `features[feature]` by
    /// `smoothing_value` and returns it (audio-features.js:29-37 —
    /// `getSmoothedFeature`).
    ///
    /// The current smoothed value starts at 0 for ids that were never touched
    /// (`smoothed[id] || 0`, audio-features.js:31). A missing feature yields a
    /// `NaN` result, matching the JS behaviour of lerping towards `undefined`.
    pub fn get_smoothed_feature(&mut self, feature: &str, id: &str, smoothing_value: f64) -> f64 {
        let target = self.features.get(feature).copied().unwrap_or(f64::NAN); // audio-features.js:32
        let start = self.smoothed.get(id).copied().unwrap_or(0.0); // audio-features.js:31
        let smoothed = lerp(start, target, smoothing_value); // audio-features.js:30-34

        self.smoothed.insert(id.to_string(), smoothed); // audio-features.js:30

        smoothed
    }
}

#[cfg(test)]
mod tests {
    //! Supplementary unit tests for the features port. The Vitest suites do not
    //! cover `audio-features.js`; these tests pin the ported semantics
    //! (lerp step, `|| 0` fallback, map replacement).
    use super::*;

    #[test]
    fn smoothing_constants_match_the_js_exports() {
        assert_eq!(MAX_SMOOTHING, 1.0);
        assert_eq!(SMOOTHING_STEP, 0.001);
    }

    #[test]
    fn get_smoothed_feature_lerps_from_zero_towards_the_feature() {
        let mut store = SmoothedFeatures::new();
        store.set_features(HashMap::from([("bass".to_string(), 1.0)]));
        store.add_smoothing_id("consumer");

        // (1 - 0.5) * 0 + 0.5 * 1 = 0.5
        let first = store.get_smoothed_feature("bass", "consumer", 0.5);
        assert!((first - 0.5).abs() < 1e-9);

        // (1 - 0.5) * 0.5 + 0.5 * 1 = 0.75
        let second = store.get_smoothed_feature("bass", "consumer", 0.5);
        assert!((second - 0.75).abs() < 1e-9);
    }

    #[test]
    fn get_smoothed_feature_without_registered_id_still_works() {
        let mut store = SmoothedFeatures::new();
        store.set_features(HashMap::from([("bass".to_string(), 1.0)]));

        // JS `smoothed[id] || 0` → starts from 0.
        let value = store.get_smoothed_feature("bass", "anonymous", 0.25);
        assert!((value - 0.25).abs() < 1e-9);
    }

    #[test]
    fn remove_smoothing_id_drops_the_stored_value() {
        let mut store = SmoothedFeatures::new();
        store.set_features(HashMap::from([("bass".to_string(), 1.0)]));
        store.add_smoothing_id("consumer");
        store.get_smoothed_feature("bass", "consumer", 0.5);
        store.remove_smoothing_id("consumer");

        // After removal the next read restarts from 0.
        let value = store.get_smoothed_feature("bass", "consumer", 0.5);
        assert!((value - 0.5).abs() < 1e-9);
    }

    #[test]
    fn missing_feature_propagates_nan_like_the_js_lerp() {
        let mut store = SmoothedFeatures::new();

        let value = store.get_smoothed_feature("absent", "consumer", 0.5);

        assert!(value.is_nan());
    }

    #[test]
    fn set_features_replaces_the_map() {
        let mut store = SmoothedFeatures::new();
        store.set_features(HashMap::from([("bass".to_string(), 1.0)]));
        store.set_features(HashMap::from([("treble".to_string(), 0.5)]));

        assert_eq!(store.get_feature("bass"), None);
        assert_eq!(store.get_feature("treble"), Some(0.5));
    }
}
