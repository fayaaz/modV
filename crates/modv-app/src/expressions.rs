//! The store-side expression cache (WS-12).
//!
//! [`ExpressionCache`] owns the Boa-backed [`ExpressionEngine`] and one
//! compiled handle per **input** id, mirroring
//! `store/modules/expressions.js`'s `getByInputId` getter: the assignment is
//! looked up by the id of the input being written, and the first assignment in
//! state order wins.
//!
//! The JS store compiles an expression once when the assignment is created or
//! loaded and then calls the stored function on every prop/meta write
//! (`utils/apply-expression.js`). The engine does the same: [`Self::rebuild`]
//! runs on preset/Scene restore and on `expressions/*` actions, while
//! [`Self::apply`] is the per-write/per-frame path.
//!
//! # `inputValue`
//!
//! `applyExpression` builds `inputValue` with
//! `get(store.state, input.getLocation)`, i.e. the *current* value of the
//! target the input writes to (`modules.active["<id>"].props["<prop>"]`,
//! `modules.active["<id>"].meta.alpha`, ...). [`resolve_input_value`] walks the
//! same paths directly against [`EngineState`]. Paths the engine does not keep
//! (Meyda's raw feature bag) resolve to `null`, like a missing lodash path.
//!
//! # `osc`
//!
//! JS reads `store.state.osc.data`; the engine has no OSC receive state
//! (WS-9 leaves OSC in the UI), so callers pass `{}` and [`Self::apply`] does
//! the same. Expressions using `osc` therefore see the JS default empty
//! object, not live OSC data.
//!
//! # Performance
//!
//! Compilation is a Boa parse plus wrapper evaluation; it runs only when
//! assignments change (create/update/remove, preset or Scene load). Evaluation
//! is one JS call with four JSON ↔ JS conversions. Expressions run on the
//! engine's main (window/event-loop) thread, so a pathological expression is
//! capped by the evaluator's runtime limits; ordinary expressions are
//! nanoseconds-to-microseconds and the every-frame path evaluates each input at
//! most once per frame.

use std::collections::HashMap;

use serde_json::{Value, json};

use modv_engine::engine_state::EngineState;
use modv_engine::expressions::{ExpressionEngine, ExpressionHandle, ExpressionScope};
use modv_engine::preset::ExpressionAssignment;

use crate::input_links::BeatsState;

/// An expression that could not be compiled or that threw while evaluating.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpressionFailure {
    /// Id of the assignment that failed (the warning key).
    pub assignment_id: String,
    /// Human-readable error from the JS engine.
    pub message: String,
}

/// One compiled assignment, keyed by the input it writes to.
#[derive(Clone, Debug)]
struct CompiledExpression {
    /// Id of the assignment (`expressions/REMOVE_EXPRESSION` keys on it).
    assignment_id: String,
    /// The source, kept for diagnostics.
    source: String,
    /// Whether the assignment should also run once per frame.
    evaluate_every_frame: bool,
    /// The compiled function.
    handle: ExpressionHandle,
}

/// The compiled-expression cache owned by the [`crate::store_ops::Store`].
pub struct ExpressionCache {
    /// Created on first use: most engine sessions never load an expression,
    /// and a Boa context is not free.
    engine: Option<ExpressionEngine>,
    /// Input id → the first assignment (state order) writing to it.
    by_input: HashMap<String, CompiledExpression>,
}

impl Default for ExpressionCache {
    fn default() -> Self {
        Self::new()
    }
}

impl ExpressionCache {
    /// An empty cache; the JS context is created on the first compile.
    pub fn new() -> Self {
        Self {
            engine: None,
            by_input: HashMap::new(),
        }
    }

    /// Recompile the cache from `state.expressions`.
    ///
    /// Invalid sources are skipped and returned so the caller can warn once;
    /// the previous compiled map is replaced wholesale, which is what a
    /// preset/Scene restore or a `expressions/remove` needs.
    pub fn rebuild(&mut self, state: &EngineState) -> Vec<ExpressionFailure> {
        // A fresh realm per rebuild: a long-lived Boa context lets one
        // expression's prototype pollution affect later expressions (and
        // persists across preset/Scene loads). Recompilation is cheap relative
        // to loading a document. Old handles die with the realm, so drop them
        // before the reset (releasing them afterwards would delete the newly
        // compiled functions: handle keys restart at 0).
        let previous = std::mem::take(&mut self.by_input);
        if let Some(engine) = self.engine.as_mut() {
            for compiled in previous.values() {
                engine.release(&compiled.handle);
            }
            engine.reset();
        }
        self.by_input = HashMap::new();
        let mut failures = Vec::new();

        for assignment in state.expressions.values() {
            // Every assignment is compiled (so an invalid duplicate is still
            // caught and reported), but `getByInputId` returns the first match
            // in object order, so only the first compiled one is applied.
            match self.compile(assignment) {
                Ok(compiled) => {
                    if self.by_input.contains_key(&assignment.input_id) {
                        self.release(&compiled.handle);
                    } else {
                        self.by_input.insert(assignment.input_id.clone(), compiled);
                    }
                }
                Err(failure) => failures.push(failure),
            }
        }

        failures
    }

    fn release(&mut self, handle: &ExpressionHandle) {
        if let Some(engine) = self.engine.as_mut() {
            engine.release(handle);
        }
    }

    fn compile(
        &mut self,
        assignment: &ExpressionAssignment,
    ) -> Result<CompiledExpression, ExpressionFailure> {
        // `new Function("... return ;")` is valid JS and yields `undefined`;
        // the wrapper needs an expression, so an empty source maps to
        // `undefined` to keep the same result.
        let source = if assignment.expression.trim().is_empty() {
            "undefined"
        } else {
            assignment.expression.as_str()
        };

        match self
            .engine
            .get_or_insert_with(ExpressionEngine::new)
            .compile(source)
        {
            Ok(handle) => Ok(CompiledExpression {
                assignment_id: assignment.id.clone(),
                source: assignment.expression.clone(),
                evaluate_every_frame: assignment.evaluate_every_frame,
                handle,
            }),
            Err(error) => Err(ExpressionFailure {
                assignment_id: assignment.id.clone(),
                message: format!("{error} (expression: {:?})", assignment.expression),
            }),
        }
    }

    /// `true` when `input_id` has a compiled expression.
    pub fn has(&self, input_id: &str) -> bool {
        self.by_input.contains_key(input_id)
    }

    /// Number of compiled expressions (tests/diagnostics).
    pub fn len(&self) -> usize {
        self.by_input.len()
    }

    /// Evaluate the expression assigned to `input_id`, if any.
    ///
    /// `Ok(None)` means the input has no expression and the caller must keep
    /// the incoming value. `Err` means the expression threw; the caller keeps
    /// the previous value and warns once.
    pub fn apply(
        &mut self,
        input_id: &str,
        value: &Value,
        input_value: &Value,
        osc: &Value,
        time_ms: f64,
    ) -> Result<Option<Value>, ExpressionFailure> {
        let Some(compiled) = self.by_input.get(input_id) else {
            return Ok(None);
        };
        let Some(engine) = self.engine.as_mut() else {
            return Ok(None);
        };

        let result = engine.evaluate(
            &compiled.handle,
            &ExpressionScope {
                value,
                time_ms,
                input_value,
                osc,
            },
        );

        match result {
            Ok(value) => Ok(Some(value)),
            Err(error) => Err(ExpressionFailure {
                assignment_id: compiled.assignment_id.clone(),
                message: format!(
                    "{error} (expression: {:?}, input: {input_id})",
                    compiled.source
                ),
            }),
        }
    }

    /// Inputs whose assignment is marked `evaluateEveryFrame`.
    ///
    /// At most one entry per input (the first assignment wins), so the
    /// per-frame loop cannot evaluate the same target twice.
    pub fn every_frame_inputs(&self) -> Vec<String> {
        self.by_input
            .iter()
            .filter(|(_, compiled)| compiled.evaluate_every_frame)
            .map(|(input_id, _)| input_id.clone())
            .collect()
    }

    /// The assignment id compiled for `input_id`, if any.
    pub fn assignment_id_for(&self, input_id: &str) -> Option<&str> {
        self.by_input
            .get(input_id)
            .map(|compiled| compiled.assignment_id.as_str())
    }
}

/// A path segment in a lodash-style lookup (`a.b[0]["c"]`).
#[derive(Clone, Debug, PartialEq, Eq)]
enum PathSegment {
    Key(String),
    Index(usize),
}

/// Parse a lodash path into segments.
///
/// Supports the shapes the store writes: dot keys, bracket-quoted keys
/// (`["module-id"]`, `['alpha']`) and numeric indexes. Invalid paths yield
/// `None`.
fn parse_path(path: &str) -> Option<Vec<PathSegment>> {
    let mut segments = Vec::new();
    let chars: Vec<char> = path.chars().collect();
    let mut index = 0;

    while index < chars.len() {
        match chars[index] {
            '.' => {
                index += 1;
            }
            '[' => {
                index += 1;
                if index >= chars.len() {
                    return None;
                }
                if chars[index] == '"' || chars[index] == '\'' {
                    let quote = chars[index];
                    index += 1;
                    let start = index;
                    while index < chars.len() && chars[index] != quote {
                        // A backslash-escaped character is taken verbatim; the
                        // generated paths never contain escapes, but a user
                        // expression path is not the source of these strings.
                        if chars[index] == '\\' && index + 1 < chars.len() {
                            index += 1;
                        }
                        index += 1;
                    }
                    if index >= chars.len() {
                        return None;
                    }
                    segments.push(PathSegment::Key(chars[start..index].iter().collect()));
                    index += 1;
                } else {
                    let start = index;
                    while index < chars.len() && chars[index] != ']' {
                        index += 1;
                    }
                    let text: String = chars[start..index].iter().collect();
                    segments.push(PathSegment::Index(text.parse().ok()?));
                }
                if index >= chars.len() || chars[index] != ']' {
                    return None;
                }
                index += 1;
            }
            _ => {
                let start = index;
                while index < chars.len() && chars[index] != '.' && chars[index] != '[' {
                    index += 1;
                }
                if start == index {
                    return None;
                }
                segments.push(PathSegment::Key(chars[start..index].iter().collect()));
            }
        }
    }

    if segments.is_empty() {
        None
    } else {
        Some(segments)
    }
}

/// Walk `segments` into `value`.
fn walk(value: &Value, segments: &[PathSegment]) -> Option<Value> {
    let mut current = value;
    for segment in segments {
        current = match segment {
            PathSegment::Key(key) => current.get(key)?,
            PathSegment::Index(index) => current.get(*index)?,
        };
    }
    Some(current.clone())
}

/// Resolve `input.getLocation` against the engine state.
///
/// This is the Rust equivalent of `get(store.state, input.getLocation)` in
/// `apply-expression.js`. `null` is returned for any path the engine does not
/// carry (missing module/group/input, the Meyda feature bag, ...), matching
/// lodash's `undefined` for a missing path after JSON conversion.
pub fn resolve_input_value(state: &EngineState, beats: &BeatsState, path: &str) -> Value {
    let Some(segments) = parse_path(path) else {
        return Value::Null;
    };

    if let [
        PathSegment::Key(root),
        PathSegment::Key(active),
        PathSegment::Key(module_id),
        PathSegment::Key(field),
        PathSegment::Key(name),
        rest @ ..,
    ] = segments.as_slice()
        && root == "modules"
        && active == "active"
    {
        let Some(module) = state.modules.get(module_id) else {
            return Value::Null;
        };
        let source = match field.as_str() {
            "props" => &module.props,
            "meta" => &module.meta,
            "data" => &module.data,
            "$props" => &module.dollar_props,
            _ => return Value::Null,
        };
        return source
            .get(name.as_str())
            .and_then(|value| walk(value, rest))
            .unwrap_or(Value::Null);
    }

    // `groups.groups[<index>].<key>` and `groups.<index>.<key>`.
    let group_parts: &[PathSegment] = match segments.as_slice() {
        [PathSegment::Key(root), PathSegment::Key(groups), rest @ ..]
            if root == "groups" && groups == "groups" =>
        {
            rest
        }
        [PathSegment::Key(root), rest @ ..] if root == "groups" => rest,
        _ => &[],
    };
    if let Some((PathSegment::Index(index), rest)) = group_parts.split_first() {
        let Some(group) = state.groups.get(*index) else {
            return Value::Null;
        };
        return serde_json::to_value(group)
            .ok()
            .and_then(|group| walk(&group, rest))
            .unwrap_or(Value::Null);
    }

    // `inputs.inputs[<id>]...`.
    if let [
        PathSegment::Key(root),
        PathSegment::Key(inputs),
        PathSegment::Key(input_id),
        rest @ ..,
    ] = segments.as_slice()
        && root == "inputs"
        && inputs == "inputs"
    {
        let Some(input) = state.inputs.get(input_id) else {
            return Value::Null;
        };
        return serde_json::to_value(input)
            .ok()
            .and_then(|input| walk(&input, rest))
            .unwrap_or(Value::Null);
    }

    match segments.as_slice() {
        [PathSegment::Key(root), PathSegment::Key(key)] if root == "beats" => match key.as_str() {
            "bpm" => json!(beats.bpm),
            "kick" => json!(beats.kick),
            _ => Value::Null,
        },
        // No engine OSC state; the JS scope reads `osc.data` and would find an
        // empty object by default.
        [PathSegment::Key(root)] if root == "osc" => json!({ "data": {} }),
        [PathSegment::Key(root), PathSegment::Key(data), rest @ ..]
            if root == "osc" && data == "data" =>
        {
            walk(&json!({}), rest).unwrap_or(Value::Null)
        }
        _ => Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use modv_engine::engine_state::ModuleInstance;
    use modv_engine::inputs::Input;
    use serde_json::Map;

    fn state_with_module(id: &str, props: Value, meta: Value) -> EngineState {
        let mut state = EngineState::new();
        state.modules.insert(
            id.to_string(),
            ModuleInstance {
                id: id.to_string(),
                module_name: "Bands".to_string(),
                meta,
                props,
                data: json!({}),
                dollar_props: json!({}),
                status: Vec::new(),
                extra: Map::new(),
            },
        );
        state
    }

    #[test]
    fn parses_dot_and_bracket_paths() {
        assert_eq!(
            parse_path("modules.active[\"m-1\"].props[\"speed\"]").unwrap(),
            vec![
                PathSegment::Key("modules".into()),
                PathSegment::Key("active".into()),
                PathSegment::Key("m-1".into()),
                PathSegment::Key("props".into()),
                PathSegment::Key("speed".into()),
            ]
        );
        assert_eq!(
            parse_path("groups.groups[2].alpha").unwrap(),
            vec![
                PathSegment::Key("groups".into()),
                PathSegment::Key("groups".into()),
                PathSegment::Index(2),
                PathSegment::Key("alpha".into()),
            ]
        );
        assert_eq!(
            parse_path("a['b'].c").unwrap(),
            vec![
                PathSegment::Key("a".into()),
                PathSegment::Key("b".into()),
                PathSegment::Key("c".into()),
            ]
        );
        assert!(parse_path("").is_none());
        assert!(parse_path("a[").is_none());
        assert!(parse_path("a[b]").is_none());
    }

    #[test]
    fn resolves_module_props_meta_and_dollar_props() {
        let state = state_with_module(
            "m-1",
            json!({"speed": 3.5, "nested": {"x": 7}}),
            json!({"alpha": 0.5}),
        );
        assert_eq!(
            resolve_input_value(
                &state,
                &BeatsState::default(),
                "modules.active[\"m-1\"].props[\"speed\"]"
            ),
            json!(3.5)
        );
        assert_eq!(
            resolve_input_value(
                &state,
                &BeatsState::default(),
                "modules.active[\"m-1\"].props.nested.x"
            ),
            json!(7)
        );
        assert_eq!(
            resolve_input_value(
                &state,
                &BeatsState::default(),
                "modules.active[\"m-1\"].meta.alpha"
            ),
            json!(0.5)
        );
        assert_eq!(
            resolve_input_value(
                &state,
                &BeatsState::default(),
                "modules.active[\"missing\"].props.speed"
            ),
            Value::Null
        );
    }

    #[test]
    fn resolves_inputs_beats_and_osc() {
        let mut state = EngineState::new();
        state.inputs.insert(
            "in-1".to_string(),
            Input {
                id: "in-1".to_string(),
                type_: "action".to_string(),
                data: json!({"moduleId": "m-1", "prop": "speed"}),
                get_location: None,
                location: None,
            },
        );
        let beats = BeatsState {
            bpm: 128.0,
            kick: true,
        };
        assert_eq!(
            resolve_input_value(&state, &beats, "inputs.inputs[\"in-1\"].data.prop"),
            json!("speed")
        );
        assert_eq!(
            resolve_input_value(&state, &beats, "beats.bpm"),
            json!(128.0)
        );
        assert_eq!(
            resolve_input_value(&state, &beats, "beats.kick"),
            json!(true)
        );
        assert_eq!(resolve_input_value(&state, &beats, "osc.data"), json!({}));
        assert_eq!(
            resolve_input_value(&state, &beats, "meyda.energy"),
            Value::Null
        );
    }

    fn assignment(id: &str, input_id: &str, expression: &str) -> ExpressionAssignment {
        ExpressionAssignment {
            id: id.to_string(),
            input_id: input_id.to_string(),
            expression: expression.to_string(),
            evaluate_every_frame: false,
            extra: Map::new(),
        }
    }

    #[test]
    fn rebuild_compiles_and_applies_expression_per_input() {
        let mut state = EngineState::new();
        state
            .expressions
            .insert("ex-1".into(), assignment("ex-1", "in-1", "value * 2"));
        let mut cache = ExpressionCache::new();
        assert!(cache.rebuild(&state).is_empty());
        assert!(cache.has("in-1"));

        let result = cache
            .apply("in-1", &json!(3), &json!(3), &json!({}), 0.0)
            .unwrap();
        assert_eq!(result, Some(json!(6)));

        // An input without an assignment passes through as `None`.
        assert_eq!(
            cache
                .apply("in-2", &json!(3), &json!(3), &json!({}), 0.0)
                .unwrap(),
            None
        );
    }

    #[test]
    fn invalid_expression_is_skipped_and_reported() {
        let mut state = EngineState::new();
        state
            .expressions
            .insert("bad".into(), assignment("bad", "in-1", "value +"));
        let mut cache = ExpressionCache::new();
        let failures = cache.rebuild(&state);
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].assignment_id, "bad");
        assert!(!cache.has("in-1"));
    }

    #[test]
    fn first_assignment_for_an_input_wins() {
        let mut state = EngineState::new();
        state
            .expressions
            .insert("a".into(), assignment("a", "in-1", "value + 1"));
        state
            .expressions
            .insert("b".into(), assignment("b", "in-1", "value + 100"));
        let mut cache = ExpressionCache::new();
        cache.rebuild(&state);
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.assignment_id_for("in-1"), Some("a"));
        assert_eq!(
            cache
                .apply("in-1", &json!(1), &json!(1), &json!({}), 0.0)
                .unwrap(),
            Some(json!(2))
        );
    }

    #[test]
    fn every_frame_inputs_lists_enabled_assignments_once() {
        let mut state = EngineState::new();
        let mut first = assignment("a", "in-1", "value + 1");
        first.evaluate_every_frame = true;
        state.expressions.insert("a".into(), first);
        state
            .expressions
            .insert("b".into(), assignment("b", "in-2", "value + 1"));
        let mut cache = ExpressionCache::new();
        cache.rebuild(&state);
        assert_eq!(cache.every_frame_inputs(), vec!["in-1".to_string()]);
    }

    #[test]
    fn runtime_failure_is_reported() {
        let mut state = EngineState::new();
        state
            .expressions
            .insert("boom".into(), assignment("boom", "in-1", "inputValue.x.y"));
        let mut cache = ExpressionCache::new();
        assert!(cache.rebuild(&state).is_empty());
        let error = cache
            .apply("in-1", &json!(1), &Value::Null, &json!({}), 0.0)
            .expect_err("throws");
        assert_eq!(error.assignment_id, "boom");
    }

    #[test]
    fn empty_source_behaves_like_undefined() {
        let mut state = EngineState::new();
        state
            .expressions
            .insert("empty".into(), assignment("empty", "in-1", "   "));
        let mut cache = ExpressionCache::new();
        assert!(cache.rebuild(&state).is_empty());
        assert_eq!(
            cache
                .apply("in-1", &json!(1), &json!(1), &json!({}), 0.0)
                .unwrap(),
            Some(Value::Null)
        );
    }
}
