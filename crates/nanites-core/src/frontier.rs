//! The frontier — the live, manipulable tree of pending tasks.
//!
//! Every spawned task is registered as a [`TaskNode`] in the frontier before
//! execution begins. Parent–child relationships are recorded automatically by
//! [`crate::Ctx::spawn`]. The frontier can be inspected, pruned, or extended
//! from outside the runtime (e.g. for pause/resume/inspect tooling).

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use crate::dyn_task::SharedDynTask;

/// Unique identifier for a node in the frontier tree.
pub type NodeId = u64;

/// A single node in the frontier tree.
#[derive(Clone)]
pub struct TaskNode {
    /// Unique id for this node.
    pub id: NodeId,
    /// The task that will be / is being executed.
    pub task: SharedDynTask,
    /// Parent node, if any (root nodes have `None`).
    pub parent: Option<NodeId>,
    /// Child nodes spawned by this task so far.
    pub children: Vec<NodeId>,
    /// Current execution status.
    pub status: NodeStatus,
}

impl fmt::Debug for TaskNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TaskNode")
            .field("id", &self.id)
            .field("task", &self.task.type_name())
            .field("parent", &self.parent)
            .field("children", &self.children)
            .field("status", &self.status)
            .finish()
    }
}

/// Lifecycle state of a frontier node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeStatus {
    /// Registered but not yet executing.
    Pending,
    /// Currently executing.
    Running,
    /// Completed successfully.
    Completed,
    /// Completed with an error.
    Failed(String),
    /// Cancelled before or during execution.
    Cancelled,
}

/// The live task tree.
///
/// All mutations are guarded by an internal [`Mutex`] so the frontier can be
/// shared safely between the tokio executor threads and any external inspector.
#[derive(Clone, Default)]
pub struct Frontier {
    inner: Arc<Mutex<FrontierInner>>,
}

#[derive(Default)]
struct FrontierInner {
    nodes: HashMap<NodeId, TaskNode>,
    next_id: NodeId,
}

impl Frontier {
    /// Create an empty frontier.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a new task node and return its id.
    ///
    /// The caller must supply the parent id (if any) — it is recorded on both
    /// the new node and the parent's `children` list.
    pub fn register(&self, task: SharedDynTask, parent: Option<NodeId>) -> NodeId {
        let mut inner = self.inner.lock().unwrap();
        let id = inner.next_id;
        inner.next_id += 1;

        let node = TaskNode {
            id,
            task,
            parent,
            children: Vec::new(),
            status: NodeStatus::Pending,
        };
        inner.nodes.insert(id, node);

        // Wire up the parent→child relationship.
        if let Some(pid) = parent
            && let Some(p) = inner.nodes.get_mut(&pid)
        {
            p.children.push(id);
        }

        id
    }

    /// Mark a node as running.
    pub fn set_running(&self, id: NodeId) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(node) = inner.nodes.get_mut(&id) {
            node.status = NodeStatus::Running;
        }
    }

    /// Mark a node as completed.
    pub fn set_completed(&self, id: NodeId) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(node) = inner.nodes.get_mut(&id) {
            node.status = NodeStatus::Completed;
        }
    }

    /// Mark a node as failed.
    pub fn set_failed(&self, id: NodeId, msg: String) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(node) = inner.nodes.get_mut(&id) {
            node.status = NodeStatus::Failed(msg);
        }
    }

    /// Mark a node as cancelled.
    pub fn set_cancelled(&self, id: NodeId) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(node) = inner.nodes.get_mut(&id) {
            node.status = NodeStatus::Cancelled;
        }
    }

    /// Get a snapshot of a node.
    pub fn get(&self, id: NodeId) -> Option<TaskNode> {
        self.inner.lock().unwrap().nodes.get(&id).cloned()
    }

    /// Get a snapshot of all nodes.
    pub fn snapshot(&self) -> Vec<TaskNode> {
        self.inner.lock().unwrap().nodes.values().cloned().collect()
    }

    /// Remove a node (e.g. after completion, for memory hygiene).
    pub fn remove(&self, id: NodeId) {
        self.inner.lock().unwrap().nodes.remove(&id);
    }

    /// Number of nodes currently tracked.
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().nodes.len()
    }

    /// Returns `true` if the frontier has no nodes.
    pub fn is_empty(&self) -> bool {
        self.inner.lock().unwrap().nodes.is_empty()
    }

    /// Replace the task on a pending node (used by scaffolds to transform tasks
    /// before execution).
    ///
    /// Has no effect if the node is not in [`NodeStatus::Pending`].
    pub fn replace_task(&self, id: NodeId, task: SharedDynTask) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(node) = inner.nodes.get_mut(&id)
            && node.status == NodeStatus::Pending
        {
            node.task = task;
        }
    }
}

/// A cheap, shareable handle to the frontier — what [`Ctx`](crate::Ctx) holds.
///
/// Wrapping in a newtype keeps the public API of `Ctx` clean while sharing
/// the same `Arc<Mutex<…>>` interior.
#[derive(Clone, Default)]
pub struct FrontierHandle(pub(crate) Frontier);

impl FrontierHandle {
    pub(crate) fn new(frontier: Frontier) -> Self {
        FrontierHandle(frontier)
    }

    pub(crate) fn inner(&self) -> &Frontier {
        &self.0
    }
}
