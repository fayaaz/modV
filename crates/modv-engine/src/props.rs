//! The module prop coercion pipeline, ported from
//! `store/modules/modules.js` `updateProp` (`modules.js:481-562`).
//!
//! JS pipeline per `updateProp`:
//!
//! 1. `dataTypes[type].create(dataOut)` — coerce into the type's canonical
//!    shape (`dataTypes.js`). For `float`/`int`/`bool`/`text`/`enum`/`event`
//!    the `dataTypes` entry has only a `get` — **no `create`** — so the
//!    value passes through unchanged. `texture`/`tween` `create` are async
//!    and domain-specific; they are **not ported in M2a**.
//! 2. `applyExpression` (WS-12) — not ported here.
//! 3. Clamp to `[min, max]` when the prop is `strict`.
//! 4. `abs` when the prop sets `abs`.
//! 5. `round` for `int`.
//!
//! Steps 3-5 are skipped when the value is an Array (`!Array.isArray(dataOut)`,
//! `modules.js:512`); objects do **not** skip them (JS checks arrays only).

use serde::{Deserialize, Serialize};

/// Schema for a single module prop, as registered by module definitions.
///
/// `type_` is the `dataTypes` key (`"float"`, `"int"`, `"bool"`, `"text"`,
/// `"enum"`, `"vec2"`, ...). Only `min`/`max`/`strict`/`abs`/`type_` take
/// part in coercion; `default` is kept for the init path (get-prop-default).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PropSchema {
    /// The `dataTypes` entry name, e.g. `"float"`. Serialised as `type`.
    #[serde(rename = "type")]
    pub type_: String,
    /// Inclusive lower bound used when `strict`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    /// Inclusive upper bound used when `strict`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    /// Clamp to `[min, max]` (requires both bounds).
    #[serde(default)]
    pub strict: bool,
    /// Take the absolute value.
    #[serde(default)]
    pub abs: bool,
    /// Registered default value for the prop (not applied during coercion).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_json::Value>,
}

/// The registry slice used for coercion: the prop schemas of one registered
/// module, keyed by prop name.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct RegisteredModule {
    /// Prop schemas keyed by prop name.
    pub props: indexmap::IndexMap<String, PropSchema>,
}

/// Round like JS `Math.round`: half rounds *towards +∞* (`floor(x + 0.5)`),
/// unlike `f64::round` (half away from zero). This only differs for exact
/// `.5` inputs on negatives (`Math.round(-2.5) === -2`, `f64::round(-2.5) === -3.0`).
fn js_round(value: f64) -> f64 {
    (value + 0.5).floor()
}

/// Build a JSON number, keeping the incoming value if the result is not finite.
fn number_or_input(number: f64, fallback: &serde_json::Value) -> serde_json::Value {
    serde_json::Number::from_f64(number)
        .map(serde_json::Value::Number)
        .unwrap_or_else(|| fallback.clone())
}

/// Coerce `value` against a prop schema.
///
/// Exact port of the JS coercion block at `modules.js:512-526`:
///
/// 1. Arrays skip steps 3-5 entirely (`!Array.isArray(dataOut)`); every other
///    value (objects included) goes through clamp/abs/round.
/// 2. Clamp to `[min, max]` **only** when `strict` and both bounds are
///    present. Numbers are coerced via `f64`; non-numbers pass through
///    untouched (JS would coerce numeric strings with `Math.max`; that is a
///    deviation from the JS `Math.*` behaviour, documented in the M2a report).
/// 3. `abs` for numbers only.
/// 4. `Math.round` for `int` (JS rounding, see [`js_round`]).
///
/// Data-types step (JS step 1): `float`/`int`/`bool`/`text`/`enum`/`event`
/// have no `create` in `dataTypes.js`, so the scalar value passes through
/// unchanged and there is nothing to port. `texture`/`tween` `create` are
/// async and domain-specific (image/video/canvas handles, tween frames) —
/// **gap: not ported in M2a**; their props pass through untouched here.
///
/// NaN handling: if the incoming value is a non-finite number the value is
/// left completely untouched (JS passes it through `Math.min/max/abs/round`
/// without filtering; clamp/abs/round are only applied to numbers).
/// `serde_json` cannot construct a non-finite `Number` in a `Value`, but a
/// non-finite `f64` may still appear via parsed `1e999` literals; those are
/// also left untouched.
pub fn coerce_prop_value(schema: &PropSchema, value: &serde_json::Value) -> serde_json::Value {
    if value.is_array() {
        return value.clone();
    }

    // Non-finite numbers are left untouched (see NaN handling note).
    let as_number = value.as_f64().filter(|n| n.is_finite());

    let mut out = value.clone();

    if schema.strict {
        if let (Some(min), Some(max)) = (schema.min, schema.max) {
            if let Some(n) = as_number {
                let clamped = n.max(min).min(max);
                out = number_or_input(clamped, value);
            }
        }
    }

    if schema.abs {
        if let Some(n) = out.as_f64().filter(|n| n.is_finite()) {
            out = number_or_input(n.abs(), value);
        }
    }

    if schema.type_ == "int" {
        if let Some(n) = out.as_f64().filter(|n| n.is_finite()) {
            out = number_or_input(js_round(n), value);
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schema(type_: &str) -> PropSchema {
        PropSchema {
            type_: type_.to_string(),
            min: None,
            max: None,
            strict: false,
            abs: false,
            default: None,
        }
    }

    #[test]
    fn strict_clamp_inside_range_is_identity() {
        let mut s = schema("float");
        s.strict = true;
        s.min = Some(0.0);
        s.max = Some(10.0);
        assert_eq!(coerce_prop_value(&s, &json!(5.5)), json!(5.5));
    }

    #[test]
    fn strict_clamp_outside_range_clamps() {
        let mut s = schema("float");
        s.strict = true;
        s.min = Some(0.0);
        s.max = Some(10.0);
        assert_eq!(coerce_prop_value(&s, &json!(42.0)), json!(10.0));
        assert_eq!(coerce_prop_value(&s, &json!(-3.0)), json!(0.0));
    }

    #[test]
    fn min_only_without_strict_does_not_clamp() {
        let mut s = schema("float");
        s.min = Some(0.0);
        s.max = Some(10.0);
        assert_eq!(coerce_prop_value(&s, &json!(-100.0)), json!(-100.0));

        s.strict = true;
        assert_eq!(coerce_prop_value(&s, &json!(-100.0)), json!(0.0));
    }

    #[test]
    fn strict_with_missing_bound_does_not_clamp() {
        let mut s = schema("float");
        s.strict = true;
        s.min = Some(0.0);
        // max missing: no clamp at all
        assert_eq!(coerce_prop_value(&s, &json!(-100.0)), json!(-100.0));

        let mut s = schema("float");
        s.strict = true;
        s.max = Some(10.0);
        assert_eq!(coerce_prop_value(&s, &json!(100.0)), json!(100.0));
    }

    #[test]
    fn abs_takes_absolute_value() {
        let mut s = schema("float");
        s.abs = true;
        assert_eq!(coerce_prop_value(&s, &json!(-2.5)), json!(2.5));
        assert_eq!(coerce_prop_value(&s, &json!(3.0)), json!(3.0));
    }

    #[test]
    fn abs_applies_after_clamp() {
        // clamp to [0, 10] then abs — a negative value clamps to 0, abs stays 0
        let mut s = schema("float");
        s.strict = true;
        s.min = Some(0.0);
        s.max = Some(10.0);
        s.abs = true;
        assert_eq!(coerce_prop_value(&s, &json!(-5.0)), json!(0.0));
    }

    #[test]
    fn int_rounds_like_js_math_round() {
        let s = schema("int");
        assert_eq!(coerce_prop_value(&s, &json!(3.4)), json!(3.0));
        assert_eq!(coerce_prop_value(&s, &json!(3.5)), json!(4.0));
        // JS Math.round rounds half toward +∞
        assert_eq!(coerce_prop_value(&s, &json!(-2.5)), json!(-2.0));
        assert_eq!(coerce_prop_value(&s, &json!(-2.7)), json!(-3.0));
    }

    #[test]
    fn arrays_skip_clamp_abs_and_round() {
        let mut s = schema("int");
        s.strict = true;
        s.min = Some(0.0);
        s.max = Some(10.0);
        s.abs = true;
        let array = json!([-100, 3.7, 999]);
        assert_eq!(coerce_prop_value(&s, &array), array);
    }

    #[test]
    fn objects_do_not_skip_clamp() {
        // JS only checks Array.isArray — objects with no number pass through,
        // objects with numeric keys are not "numbers" either way, so they pass
        // through untouched; this pins the array-only skip rule.
        let mut s = schema("int");
        s.strict = true;
        s.min = Some(0.0);
        s.max = Some(10.0);
        let object = json!({"x": -100});
        assert_eq!(coerce_prop_value(&s, &object), object);
    }

    #[test]
    fn non_numbers_pass_through_untouched() {
        let mut s = schema("int");
        s.strict = true;
        s.min = Some(0.0);
        s.max = Some(10.0);
        s.abs = true;
        assert_eq!(coerce_prop_value(&s, &json!("5")), json!("5"));
        assert_eq!(coerce_prop_value(&s, &json!(true)), json!(true));
        assert_eq!(coerce_prop_value(&s, &json!(null)), json!(null));
    }

    #[test]
    fn float_bool_text_enum_event_pass_through() {
        for type_ in ["float", "bool", "text", "enum", "event"] {
            let s = schema(type_);
            assert_eq!(coerce_prop_value(&s, &json!("keep")), json!("keep"));
            assert_eq!(coerce_prop_value(&s, &json!(12.25)), json!(12.25));
        }
    }

    #[test]
    fn texture_and_tween_pass_through_gap() {
        // texture/tween `create` are async and unported in M2a; values pass
        // through unchanged.
        let texture = json!({"type": "image", "options": {"path": "/tmp/a.png"}});
        assert_eq!(coerce_prop_value(&schema("texture"), &texture), texture);
        let tween = json!({"id": "tw-1", "frames": [0, 1]});
        assert_eq!(coerce_prop_value(&schema("tween"), &tween), tween);
    }

    #[test]
    fn combined_clamp_abs_round() {
        let mut s = schema("int");
        s.strict = true;
        s.min = Some(-10.0);
        s.max = Some(10.0);
        s.abs = true;
        // -4.4 -> clamp ok -> abs 4.4 -> round 4
        assert_eq!(coerce_prop_value(&s, &json!(-4.4)), json!(4.0));
        // 12 -> clamp 10 -> abs 10 -> round 10
        assert_eq!(coerce_prop_value(&s, &json!(12.0)), json!(10.0));
    }
}
