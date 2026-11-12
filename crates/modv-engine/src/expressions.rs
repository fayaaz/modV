//! JavaScript expression evaluation (WS-12).
//!
//! The worker store compiles a user's expression with
//! `new Function("scope", "const { value, time, inputValue, osc } = scope; return <expression>")`
//! (`store/modules/expressions.js:25-45`) and calls it on every prop/meta write
//! (`utils/apply-expression.js:4-23`). Presets store only the source text and
//! the `evaluateEveryFrame` flag, so an engine that ignores expressions silently
//! drops a user-visible part of a preset. This module is the Rust replacement
//! for that JS evaluator, backed by [Boa](https://github.com/boa-dev/boa), a
//! pure-Rust ECMAScript engine.
//!
//! # API
//!
//! ```ignore
//! let mut engine = ExpressionEngine::new();
//! let handle = engine.compile("value * 2 + Math.sin(time)")?;
//! let value = engine.evaluate(&handle, &ExpressionScope {
//!     value: &json!(0.5),
//!     time_ms: 1234.0,
//!     input_value: &json!(1.0),
//!     osc: &json!({}),
//! })?;
//! # Ok::<(), ExpressionError>(())
//! ```
//!
//! `compile` parses the expression once; every `evaluate` reuses the compiled
//! function object, so the per-write/per-frame cost is a call plus the JSON ↔
//! JS conversion, not a parse/compile. Handles are kept alive as properties of
//! a hidden object on the JS global (Boa's GC only traces from JS roots, not
//! from Rust structs), so a long-lived [`ExpressionEngine`] never sees a
//! compiled function collected.
//!
//! # Semantics
//!
//! * The expression body has `value`, `time`, `inputValue` and `osc` in scope,
//!   exactly like the JS `new Function` body. `time` is `Date.now()` at
//!   evaluation time, supplied by the caller as `time_ms`.
//! * JSON numbers/booleans/strings/arrays/objects round-trip through
//!   [`serde_json::Value`], so the store's prop coercion sees the same shape JS
//!   would commit.
//! * JS `undefined` cannot be represented in JSON; it is returned as
//!   [`serde_json::Value::Null`] (documented deviation — the JS path would
//!   commit `undefined`, which the prop coercion then turns into `NaN`/no-op).
//!   Non-finite numbers likewise cannot be represented and become `null`.
//! * Runtime limits are set on the Boa context
//!   ([`ExpressionEngine::new`]): a runaway `while (true)` expression errors
//!   instead of hanging the engine's main thread.
//!
//! # Deviation: `osc`
//!
//! The JS scope reads `store.state.osc.data`. The Rust engine has no OSC
//! receive state yet (WS-9 leaves the renderer owning OSC), so callers pass an
//! empty object; `osc` is always `{}` until that state lands. Expressions that
//! only read `value`/`time`/`inputValue` are unaffected.

use boa_engine::{
    Context, JsObject, JsValue, Source, js_string, property::Attribute, vm::RuntimeLimits,
};
use serde_json::Value;
use thiserror::Error;

/// Maximum number of loop iterations one expression may perform.
///
/// Boa throws when this is exceeded, so a saved `while (true) …` cannot wedge
/// the engine thread. Normal expressions never approach it.
const EXPRESSION_LOOP_LIMIT: u64 = 100_000;

/// Maximum JS recursion depth inside one expression.
const EXPRESSION_RECURSION_LIMIT: usize = 64;

/// An error from compiling or evaluating an expression.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ExpressionError {
    /// The source did not compile (syntax error / bad wrapper).
    #[error("could not compile expression: {0}")]
    Compile(String),
    /// The expression compiled but threw (or returned a value that cannot be
    /// converted to JSON) when evaluated.
    #[error("expression threw: {0}")]
    Runtime(String),
}

/// A compiled expression, reusable for many evaluations.
///
/// The handle keeps the source text for diagnostics and a key identifying the
/// function object inside the [`ExpressionEngine`] that compiled it. A handle
/// is only valid for the engine that produced it; evaluating it on another
/// engine yields an error rather than the wrong function.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpressionHandle {
    key: String,
    source: String,
}

impl ExpressionHandle {
    /// The expression source this handle was compiled from.
    pub fn source(&self) -> &str {
        &self.source
    }
}

/// The variables an expression sees, matching the JS `applyExpression` scope.
#[derive(Clone, Copy, Debug)]
pub struct ExpressionScope<'a> {
    /// The value being written (`value` in JS).
    pub value: &'a Value,
    /// Wall-clock time in milliseconds (`Date.now()` in JS).
    pub time_ms: f64,
    /// The current value of the input's `getLocation` (`inputValue` in JS).
    pub input_value: &'a Value,
    /// The OSC receive data (`osc` in JS); empty until the engine owns OSC.
    pub osc: &'a Value,
}

/// A reusable Boa context plus the compiled function objects.
pub struct ExpressionEngine {
    context: Context,
    /// Hidden global object holding every compiled function keyed by handle id.
    ///
    /// Boa's GC traces from JS roots only, so a `JsObject` kept in a Rust
    /// struct is not by itself a GC root. Registering the map on the global
    /// object keeps every function reachable for the engine's lifetime.
    functions: JsObject,
    next_key: u64,
}

impl Default for ExpressionEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl ExpressionEngine {
    /// A fresh evaluator with `Math`, `JSON`, `String`, `Array`, … available.
    pub fn new() -> Self {
        let mut context = Context::default();
        let mut limits = RuntimeLimits::default();
        limits.set_loop_iteration_limit(EXPRESSION_LOOP_LIMIT);
        limits.set_recursion_limit(EXPRESSION_RECURSION_LIMIT);
        context.set_runtime_limits(limits);

        let functions = JsObject::with_object_proto(context.intrinsics());
        // The set can only fail if the global object is not extensible, which
        // it is for a default context. Ignoring the result keeps `new`
        // infallible; `compile` reports a missing map as a compile error.
        let _ = context.register_global_property(
            js_string!("__modvExpressionFunctions"),
            functions.clone(),
            Attribute::WRITABLE | Attribute::CONFIGURABLE | Attribute::NON_ENUMERABLE,
        );

        Self {
            context,
            functions,
            next_key: 0,
        }
    }

    /// Compile `source` into a reusable handle.
    ///
    /// The wrapper mirrors the JS `new Function` body: the four scope names
    /// are parameters and the source is the returned expression. A syntax error
    /// is reported as [`ExpressionError::Compile`].
    pub fn compile(&mut self, source: &str) -> Result<ExpressionHandle, ExpressionError> {
        // The newlines matter: a trailing `// comment` in the user's source
        // would otherwise swallow the closing parenthesis.
        let wrapper =
            format!("(function(value, time, inputValue, osc) {{\n  return (\n{source}\n  );\n}})");

        let value = self
            .context
            .eval(Source::from_bytes(wrapper.as_str()))
            .map_err(|error| ExpressionError::Compile(error.to_string()))?;

        if !value.is_callable() {
            return Err(ExpressionError::Compile(
                "expression did not evaluate to a function".to_string(),
            ));
        }

        let key = format!("e{}", self.next_key);
        self.next_key = self.next_key.wrapping_add(1);

        self.functions
            .set(js_string!(key.as_str()), value, false, &mut self.context)
            .map_err(|error| ExpressionError::Compile(error.to_string()))?;

        Ok(ExpressionHandle {
            key,
            source: source.to_string(),
        })
    }

    /// Drop a compiled function from the engine.
    ///
    /// Handles live for the engine's lifetime otherwise; the store releases a
    /// handle when an expression is edited or removed so repeated edits do not
    /// accumulate function objects in the JS heap.
    pub fn release(&mut self, handle: &ExpressionHandle) {
        let _ = self
            .functions
            .delete_property_or_throw(js_string!(handle.key.as_str()), &mut self.context);
    }

    /// Evaluate a compiled expression with `scope`.    ///
    /// `undefined` and non-finite results become [`Value::Null`]; see the
    /// module docs.
    pub fn evaluate(
        &mut self,
        handle: &ExpressionHandle,
        scope: &ExpressionScope<'_>,
    ) -> Result<Value, ExpressionError> {
        let function = self
            .functions
            .get(js_string!(handle.key.as_str()), &mut self.context)
            .map_err(|error| ExpressionError::Runtime(error.to_string()))?
            .as_callable()
            .ok_or_else(|| {
                ExpressionError::Runtime(format!(
                    "compiled expression '{}' is missing from this engine",
                    handle.source
                ))
            })?;

        let value = JsValue::from_json(scope.value, &mut self.context)
            .map_err(|error| ExpressionError::Runtime(error.to_string()))?;
        let input_value = JsValue::from_json(scope.input_value, &mut self.context)
            .map_err(|error| ExpressionError::Runtime(error.to_string()))?;
        let osc = JsValue::from_json(scope.osc, &mut self.context)
            .map_err(|error| ExpressionError::Runtime(error.to_string()))?;
        let args = [value, JsValue::new(scope.time_ms), input_value, osc];

        let result = function
            .call(&JsValue::undefined(), &args, &mut self.context)
            .map_err(|error| ExpressionError::Runtime(error.to_string()))?;

        let json = result
            .to_json(&mut self.context)
            .map_err(|error| ExpressionError::Runtime(error.to_string()))?;

        Ok(json.map(normalize_json).unwrap_or(Value::Null))
    }
}

/// Normalise a JS result to the JSON `JSON.stringify` would write.
///
/// JavaScript has one number type; Boa keeps `3` and `3.0` distinguishable in
/// its JSON conversion (an integral `Float64` becomes a `serde_json` float).
/// `JSON.stringify(3.0)` is `"3"`, and the worker's preset documents write
/// whole numbers without a fractional part, so integral values are turned into
/// JSON integers here. This keeps engine-written props byte-comparable with the
/// JS store's output. Floats stay floats; non-finite values already became
/// `null` in `to_json`.
fn normalize_json(value: Value) -> Value {
    match value {
        Value::Number(number) => match number.as_f64() {
            Some(number) if number.fract() == 0.0 && number.abs() <= 9_007_199_254_740_991.0 => {
                Value::from(number as i64)
            }
            _ => Value::Number(number),
        },
        Value::Array(items) => Value::Array(items.into_iter().map(normalize_json).collect()),
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, value)| (key, normalize_json(value)))
                .collect(),
        ),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn evaluate(source: &str, value: Value, time_ms: f64, input: Value, osc: Value) -> Value {
        let mut engine = ExpressionEngine::new();
        let handle = engine.compile(source).expect("expression compiles");
        engine
            .evaluate(
                &handle,
                &ExpressionScope {
                    value: &value,
                    time_ms,
                    input_value: &input,
                    osc: &osc,
                },
            )
            .expect("expression evaluates")
    }

    fn eval(source: &str, value: Value) -> Value {
        evaluate(source, value, 0.0, json!(0), json!({}))
    }

    #[test]
    fn arithmetic_and_math_work() {
        assert_eq!(eval("value * 2 + 1", json!(3)), json!(7));
        assert_eq!(eval("Math.round(value)", json!(2.6)), json!(3));
        assert_eq!(eval("Math.sin(0)", json!(0)), json!(0));
    }

    #[test]
    fn ternary_and_booleans_work() {
        assert_eq!(eval("value > 0.5 ? true : false", json!(0.75)), json!(true));
        assert_eq!(
            eval("value > 0.5 ? true : false", json!(0.25)),
            json!(false)
        );
        assert_eq!(eval("!value", json!(false)), json!(true));
    }

    #[test]
    fn strings_numbers_arrays_and_objects_round_trip() {
        assert_eq!(eval("'a' + value", json!("b")), json!("ab"));
        assert_eq!(
            eval("[value, value + 1, 'x']", json!(2)),
            json!([2, 3, "x"])
        );
        assert_eq!(
            eval("({ min: value, max: value * 10 })", json!(0.2)),
            json!({"min": 0.2, "max": 2})
        );
    }

    #[test]
    fn input_value_and_osc_are_in_scope() {
        assert_eq!(
            evaluate("inputValue + value", json!(2), 0.0, json!(40), json!({})),
            json!(42)
        );
        assert_eq!(
            evaluate(
                "osc.alpha === undefined ? value : osc.alpha",
                json!(1),
                0.0,
                json!(0),
                json!({"alpha": 7})
            ),
            json!(7)
        );
        assert_eq!(
            evaluate("osc", json!(1), 0.0, json!(0), json!({})),
            json!({})
        );
    }

    #[test]
    fn time_is_in_scope() {
        assert_eq!(
            evaluate("time * 2", json!(0), 21.5, json!(0), json!({})),
            json!(43)
        );
        assert_eq!(
            evaluate(
                "Math.floor(time / 1000)",
                json!(0),
                2500.0,
                json!(0),
                json!({})
            ),
            json!(2)
        );
    }

    #[test]
    fn missing_properties_are_undefined_and_become_null() {
        assert_eq!(eval("inputValue.missing", json!(0)), Value::Null);
        assert_eq!(eval("value", json!(null)), Value::Null);
        // `undefined` has no JSON representation; the engine maps it to null.
        assert_eq!(eval("undefined", json!(0)), Value::Null);
    }

    #[test]
    fn syntax_error_is_reported_as_compile_error() {
        let mut engine = ExpressionEngine::new();
        let error = engine.compile("value +").expect_err("invalid syntax");
        assert!(matches!(error, ExpressionError::Compile(_)));
    }

    #[test]
    fn runtime_throw_is_reported() {
        let mut engine = ExpressionEngine::new();
        let handle = engine
            .compile("inputValue.missing.deeper")
            .expect("compiles");
        let error = engine
            .evaluate(
                &handle,
                &ExpressionScope {
                    value: &json!(0),
                    time_ms: 0.0,
                    input_value: &Value::Null,
                    osc: &json!({}),
                },
            )
            .expect_err("throws at runtime");
        assert!(matches!(error, ExpressionError::Runtime(_)));
    }

    #[test]
    fn handles_are_reusable_and_independent() {
        let mut engine = ExpressionEngine::new();
        let double = engine.compile("value * 2").expect("compiles");
        let plus_one = engine.compile("value + 1").expect("compiles");
        let mut check = |handle: &ExpressionHandle, value: Value| {
            engine
                .evaluate(
                    handle,
                    &ExpressionScope {
                        value: &value,
                        time_ms: 0.0,
                        input_value: &Value::Null,
                        osc: &Value::Null,
                    },
                )
                .unwrap()
        };
        assert_eq!(check(&double, json!(2)), json!(4));
        assert_eq!(check(&plus_one, json!(2)), json!(3));
        assert_eq!(check(&double, json!(5)), json!(10));
    }

    #[test]
    fn node_reference_semantics_match() {
        // Expected values produced by Node 24 running the worker's wrapper:
        //   new Function("scope", "const {value,time,inputValue,osc}=scope; return <expr>")
        // and `JSON.stringify(result)` (undefined printed as typeof).
        let cases: &[(&str, Value, Value)] = &[
            ("value * 2 + 1", json!(3), json!(7)),
            ("value > 0.5 ? true : false", json!(0.75), json!(true)),
            ("!value", json!(false), json!(true)),
            ("'a' + value", json!("b"), json!("ab")),
            ("Math.round(value)", json!(2.6), json!(3)),
            ("[value, value + 1, 'x']", json!(2), json!([2, 3, "x"])),
            (
                "({ min: value, max: value * 10 })",
                json!(0.2),
                json!({"min": 0.2, "max": 2}),
            ),
            // Node reports `undefined`; the engine's JSON boundary maps it to
            // null (see the module docs).
            ("undefined", json!(0), Value::Null),
            ("inputValue.missing", json!(0), Value::Null),
        ];

        for (source, value, expected) in cases {
            assert_eq!(&eval(source, value.clone()), expected, "{source}");
        }
    }

    #[test]
    fn released_handle_cannot_be_evaluated() {
        let mut engine = ExpressionEngine::new();
        let handle = engine.compile("value * 2").expect("compiles");
        engine.release(&handle);
        let error = engine
            .evaluate(
                &handle,
                &ExpressionScope {
                    value: &json!(1),
                    time_ms: 0.0,
                    input_value: &Value::Null,
                    osc: &Value::Null,
                },
            )
            .expect_err("released handle is gone");
        assert!(matches!(error, ExpressionError::Runtime(_)));
    }

    #[test]
    fn trailing_line_comment_does_not_break_the_wrapper() {
        assert_eq!(eval("value + 1 // maths", json!(1)), json!(2));
    }

    #[test]
    fn runaway_loop_hits_the_runtime_limit() {
        let mut engine = ExpressionEngine::new();
        let handle = engine
            .compile("(() => { while (true) {} })()")
            .expect("compiles");
        let error = engine
            .evaluate(
                &handle,
                &ExpressionScope {
                    value: &json!(0),
                    time_ms: 0.0,
                    input_value: &Value::Null,
                    osc: &json!({}),
                },
            )
            .expect_err("loop limit throws");
        assert!(matches!(error, ExpressionError::Runtime(_)));
    }
}
