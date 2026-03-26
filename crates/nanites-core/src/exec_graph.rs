//! Execution graph — the monotonically-growing audit log of all spawned tasks.
//!
//! The [`Frontier`](crate::Frontier) tracks *pending* tasks only; it shrinks
//! as tasks complete. The [`ExecGraph`] is the complementary structure: it
//! records every task that was ever spawned, its parent–child lineage, its
//! params snapshot (if the task implements [`SerializableTask`]), and its
//! terminal state.
//!
//! The execution graph grows monotonically — entries are never removed.
//! Query it for audit, replay, or debugging purposes.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::Value as JsonValue;

use crate::frontier::NodeId;

// ─── TerminalState ────────────────────────────────────────────────────────────

/// The resolved outcome of a task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalState {
    /// Task produced a value. Carries the Rust type name of the output type.
    Completed {
        /// `std::any::type_name` of the output type.
        output_type: &'static str,
    },
    /// Task returned an error.
    Failed {
        /// `Display` string of the error.
        message: String,
    },
    /// Task was cancelled before producing a value.
    Cancelled,
}

// ─── ExecNode ─────────────────────────────────────────────────────────────────

/// A single record in the execution graph.
#[derive(Debug, Clone)]
pub struct ExecNode {
    /// Unique identifier (same space as [`NodeId`] in the frontier).
    pub id: NodeId,
    /// `DynTask::type_name()` at spawn time.
    pub task_type: String,
    /// Params snapshot at spawn time, if the task implements
    /// [`SerializableTask`](crate::registry::SerializableTask).
    /// `None` for tasks that don't implement the trait.
    pub params_snapshot: Option<JsonValue>,
    /// Parent node id, if this task was spawned by another task.
    pub parent: Option<NodeId>,
    /// All child nodes spawned by this task.
    pub children: Vec<NodeId>,
    /// Terminal state, set when the task finishes.
    /// `None` while the task is still running.
    pub terminal: Option<TerminalState>,
}

// ─── ExecGraph ────────────────────────────────────────────────────────────────

/// Monotonically-growing audit log of all spawned tasks.
///
/// All mutations are guarded by an internal [`Mutex`]. Clone is cheap —
/// all clones share the same underlying storage.
#[derive(Clone, Default)]
pub struct ExecGraph {
    inner: Arc<Mutex<ExecGraphInner>>,
}

#[derive(Default)]
struct ExecGraphInner {
    nodes: HashMap<NodeId, ExecNode>,
}

impl ExecGraph {
    /// Create an empty execution graph.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a newly spawned task.
    ///
    /// Called by the runtime immediately after a task is registered in the
    /// frontier, before execution begins.
    pub fn record_spawn(
        &self,
        id: NodeId,
        task_type: impl Into<String>,
        params_snapshot: Option<JsonValue>,
        parent: Option<NodeId>,
    ) {
        let mut inner = self.inner.lock().unwrap();
        let node = ExecNode {
            id,
            task_type: task_type.into(),
            params_snapshot,
            parent,
            children: Vec::new(),
            terminal: None,
        };
        inner.nodes.insert(id, node);

        // Wire up the parent → child relationship.
        if let Some(pid) = parent
            && let Some(p) = inner.nodes.get_mut(&pid)
        {
            p.children.push(id);
        }
    }

    /// Mark a node as completed.
    ///
    /// `output_type` should be `std::any::type_name::<T::Output>()`.
    pub fn set_completed(&self, id: NodeId, output_type: &'static str) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(node) = inner.nodes.get_mut(&id) {
            node.terminal = Some(TerminalState::Completed { output_type });
        }
    }

    /// Mark a node as failed.
    pub fn set_failed(&self, id: NodeId, message: impl Into<String>) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(node) = inner.nodes.get_mut(&id) {
            node.terminal = Some(TerminalState::Failed {
                message: message.into(),
            });
        }
    }

    /// Mark a node as cancelled.
    pub fn set_cancelled(&self, id: NodeId) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(node) = inner.nodes.get_mut(&id) {
            node.terminal = Some(TerminalState::Cancelled);
        }
    }

    /// Get a snapshot of a single node.
    pub fn get(&self, id: NodeId) -> Option<ExecNode> {
        self.inner.lock().unwrap().nodes.get(&id).cloned()
    }

    /// Get snapshots of all nodes.
    pub fn snapshot(&self) -> Vec<ExecNode> {
        self.inner.lock().unwrap().nodes.values().cloned().collect()
    }

    /// Get the children of a node.
    pub fn get_children(&self, id: NodeId) -> Vec<NodeId> {
        self.inner
            .lock()
            .unwrap()
            .nodes
            .get(&id)
            .map(|n| n.children.clone())
            .unwrap_or_default()
    }

    /// All nodes with the given terminal state (convenience query).
    pub fn completed_nodes(&self) -> Vec<ExecNode> {
        self.inner
            .lock()
            .unwrap()
            .nodes
            .values()
            .filter(|n| matches!(&n.terminal, Some(TerminalState::Completed { .. })))
            .cloned()
            .collect()
    }

    /// All nodes that failed.
    pub fn failed_nodes(&self) -> Vec<ExecNode> {
        self.inner
            .lock()
            .unwrap()
            .nodes
            .values()
            .filter(|n| matches!(&n.terminal, Some(TerminalState::Failed { .. })))
            .cloned()
            .collect()
    }

    /// All nodes that are still running (no terminal state recorded yet).
    pub fn running_nodes(&self) -> Vec<ExecNode> {
        self.inner
            .lock()
            .unwrap()
            .nodes
            .values()
            .filter(|n| n.terminal.is_none())
            .cloned()
            .collect()
    }

    /// Total number of nodes ever recorded.
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().nodes.len()
    }

    /// Returns `true` if no tasks have been recorded.
    pub fn is_empty(&self) -> bool {
        self.inner.lock().unwrap().nodes.is_empty()
    }
}

impl std::fmt::Debug for ExecGraph {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecGraph")
            .field("len", &self.len())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_and_complete() {
        let graph = ExecGraph::new();
        graph.record_spawn(0, "MyTask", None, None);
        graph.record_spawn(1, "ChildTask", None, Some(0));

        graph.set_completed(0, "String");
        graph.set_failed(1, "something went wrong");

        let root = graph.get(0).unwrap();
        assert_eq!(root.children, vec![1]);
        assert!(matches!(
            root.terminal,
            Some(TerminalState::Completed {
                output_type: "String"
            })
        ));

        let child = graph.get(1).unwrap();
        assert_eq!(child.parent, Some(0));
        assert!(matches!(child.terminal, Some(TerminalState::Failed { .. })));
    }

    #[test]
    fn children_query() {
        let graph = ExecGraph::new();
        graph.record_spawn(0, "Root", None, None);
        graph.record_spawn(1, "A", None, Some(0));
        graph.record_spawn(2, "B", None, Some(0));
        graph.record_spawn(3, "C", None, Some(1));

        let children = graph.get_children(0);
        assert_eq!(children, vec![1, 2]);

        let grandchildren = graph.get_children(1);
        assert_eq!(grandchildren, vec![3]);
    }

    #[test]
    fn running_nodes_query() {
        let graph = ExecGraph::new();
        graph.record_spawn(0, "A", None, None);
        graph.record_spawn(1, "B", None, None);
        graph.set_completed(0, "()");

        let running = graph.running_nodes();
        assert_eq!(running.len(), 1);
        assert_eq!(running[0].id, 1);
    }

    #[test]
    fn params_snapshot_stored() {
        let graph = ExecGraph::new();
        let params = serde_json::json!({"model": "gpt-4", "temperature": 0.7});
        graph.record_spawn(0, "LlmTask", Some(params.clone()), None);

        let node = graph.get(0).unwrap();
        assert_eq!(node.params_snapshot, Some(params));
    }
}
