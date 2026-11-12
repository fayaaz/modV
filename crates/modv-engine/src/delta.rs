//! The delta stream the engine emits to keep the UI mirror store valid.
//!
//! The engine emits the same `{ type, payload }` records the worker's
//! `commitQueue` produces (`worker/index.worker.js:136-144`), replayed by the
//! renderer into its mirror store. This module is intentionally minimal for
//! M2a: a queue of mutations and their drain. The bus dispatch, per-frame
//! filtering (`modules/UPDATE_ACTIVE_MODULE` only for `props`/`meta`,
//! `beats/SET_KICK` de-duplication) and replay live in `modv-engine` proper
//! (WS-1, later milestones).

use serde::{Deserialize, Serialize};

/// A single `{ type, payload }` mutation, as consumed by the mirror store.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mutation {
    /// Mutation type, e.g. `modules/UPDATE_ACTIVE_MODULE_PROP`.
    #[serde(rename = "type")]
    pub r#type: String,
    /// Mutation payload (opaque JSON).
    pub payload: serde_json::Value,
}

/// A FIFO queue of pending [`Mutation`]s.
///
/// The mirror replay is pull-based: each frame the engine drains the queue
/// and sends the mutations in order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DeltaStream {
    queue: Vec<Mutation>,
}

impl DeltaStream {
    /// An empty stream.
    pub fn new() -> Self {
        Self { queue: Vec::new() }
    }

    /// Queue a mutation at the back of the stream.
    pub fn push(&mut self, r#type: impl Into<String>, payload: serde_json::Value) {
        self.queue.push(Mutation {
            r#type: r#type.into(),
            payload,
        });
    }

    /// Remove and return all queued mutations in issue order.
    pub fn drain(&mut self) -> Vec<Mutation> {
        std::mem::take(&mut self.queue)
    }

    /// Number of pending mutations.
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    /// `true` when no mutations are pending.
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

impl IntoIterator for DeltaStream {
    type Item = Mutation;
    type IntoIter = std::vec::IntoIter<Mutation>;

    fn into_iter(self) -> Self::IntoIter {
        self.queue.into_iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn new_stream_is_empty() {
        let mut stream = DeltaStream::new();
        assert_eq!(stream.len(), 0);
        assert!(stream.is_empty());
        assert!(stream.drain().is_empty());
    }

    #[test]
    fn push_and_drain_in_order() {
        let mut stream = DeltaStream::new();
        stream.push("groups/ADD_GROUP", json!({"group": {"id": "g1"}}));
        stream.push("modules/UPDATE_ACTIVE_MODULE_PROP", json!({"id": "m1"}));
        stream.push("beats/SET_BPM", json!({"bpm": 128}));

        assert_eq!(stream.len(), 3);
        assert!(!stream.is_empty());

        let drained = stream.drain();
        assert_eq!(drained.len(), 3);
        assert_eq!(drained[0].r#type, "groups/ADD_GROUP");
        assert_eq!(drained[1].r#type, "modules/UPDATE_ACTIVE_MODULE_PROP");
        assert_eq!(drained[2].r#type, "beats/SET_BPM");
        assert_eq!(drained[2].payload, json!({"bpm": 128}));

        // Drain empties the stream.
        assert!(stream.is_empty());
    }

    #[test]
    fn mutation_serializes_like_worker_commit_queue() {
        let mut stream = DeltaStream::new();
        stream.push(
            "modules/UPDATE_ACTIVE_MODULE_META",
            json!({"metaKey": "alpha", "data": 0.5}),
        );
        let drained = stream.drain();
        assert_eq!(
            serde_json::to_value(&drained[0]).unwrap(),
            json!({"type": "modules/UPDATE_ACTIVE_MODULE_META", "payload": {"metaKey": "alpha", "data": 0.5}})
        );
    }

    #[test]
    fn mutation_deserializes_from_wire() {
        let raw = json!({"type": "groups/SWAP", "payload": null});
        let mutation: Mutation = serde_json::from_value(raw).unwrap();
        assert_eq!(mutation.r#type, "groups/SWAP");
        assert_eq!(mutation.payload, serde_json::Value::Null);
    }

    #[test]
    fn into_iter_preserves_order() {
        let mut stream = DeltaStream::new();
        stream.push("a/ONE", json!(1));
        stream.push("b/TWO", json!(2));
        let types: Vec<String> = stream.into_iter().map(|m| m.r#type).collect();
        assert_eq!(types, vec!["a/ONE", "b/TWO"]);
    }
}
