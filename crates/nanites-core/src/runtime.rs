//! [`Runtime`] — the entry point for task execution.
//!
//! The runtime:
//! 1. Owns the [`Frontier`], the [`ExecGraph`], and a list of global [`Scaffold`]s.
//! 2. Applies scaffolds to each task before execution.
//! 3. Drives tasks on the current tokio executor.
//!
//! # Root execution
//!
//! [`Runtime::run`] is the typed entry point for running a single top-level task.
//! [`Runtime::run_dyn`] is the dynamic variant.
//!
//! # Scaffolds
//!
//! Attach global scaffolds with [`Runtime::with_scaffold`]. They are applied to
//! every spawned task in order (first scaffold applied first).
//!
//! # Cancellation
//!
//! Every runtime has a root [`CancellationToken`]. Cancel it with
//! [`Runtime::cancel`] to signal all in-flight tasks.

use std::fmt;
use std::sync::Arc;

use crate::cancellation::CancellationToken;
use crate::ctx::Ctx;
use crate::dyn_task::{AnyInput, AnyOutput, SharedDynTask};
use crate::error::RuntimeError;
use crate::exec_graph::ExecGraph;
use crate::frontier::{Frontier, FrontierHandle};
use crate::handle::TaskHandle;
use crate::scaffold::Scaffold;
use crate::task::Task;

/// The nanites runtime.
///
/// Cheap to clone — all clones share the same frontier, execution graph,
/// cancel token, and scaffold list.
#[derive(Clone)]
pub struct Runtime {
    frontier: FrontierHandle,
    exec_graph: ExecGraph,
    cancel: CancellationToken,
    scaffolds: Arc<Vec<Scaffold>>,
}

impl fmt::Debug for Runtime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Runtime")
            .field("frontier_len", &self.frontier.inner().len())
            .field("exec_graph_len", &self.exec_graph.len())
            .field("scaffolds", &self.scaffolds.len())
            .finish_non_exhaustive()
    }
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}

impl Runtime {
    /// Create a runtime with no scaffolds and a fresh cancellation token.
    pub fn new() -> Self {
        Runtime {
            frontier: FrontierHandle::new(Frontier::new()),
            exec_graph: ExecGraph::new(),
            cancel: CancellationToken::new(),
            scaffolds: Arc::new(Vec::new()),
        }
    }

    /// Add a global scaffold (applied to every spawned task, in order).
    ///
    /// Returns `self` for method chaining.
    #[must_use]
    pub fn with_scaffold(mut self, scaffold: Scaffold) -> Self {
        Arc::make_mut(&mut self.scaffolds).push(scaffold);
        self
    }

    /// Signal cancellation for all tasks in this runtime.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    /// The live task tree (pending tasks only).
    pub fn frontier(&self) -> &Frontier {
        self.frontier.inner()
    }

    /// The monotonically-growing execution graph (all spawned tasks, including
    /// completed, failed, and cancelled).
    pub fn exec_graph(&self) -> &ExecGraph {
        &self.exec_graph
    }

    // -------------------------------------------------------------------------
    // Typed API
    // -------------------------------------------------------------------------

    /// Run a task with typed input, returning a handle to its output.
    ///
    /// This does NOT block. The task runs on the tokio executor. Await the
    /// handle to get the result.
    pub fn spawn<T>(&self, task: T, input: T::Input) -> TaskHandle<T::Output>
    where
        T: Task + fmt::Debug + Clone,
        T::Error: std::error::Error + Send + Sync + 'static,
    {
        let ctx = self.root_ctx();
        ctx.spawn(task, input)
    }

    /// Convenience method: spawn and await in one call.
    ///
    /// Equivalent to `runtime.spawn(task, input).await`.
    pub async fn run<T>(&self, task: T, input: T::Input) -> Result<T::Output, RuntimeError>
    where
        T: Task + fmt::Debug + Clone,
        T::Error: std::error::Error + Send + Sync + 'static,
    {
        self.spawn(task, input).await
    }

    // -------------------------------------------------------------------------
    // Dynamic API
    // -------------------------------------------------------------------------

    /// Spawn a type-erased task.
    pub fn spawn_dyn(&self, task: SharedDynTask, input: AnyInput) -> TaskHandle<AnyOutput> {
        let ctx = self.root_ctx();
        ctx.spawn_dyn(task, input)
    }

    /// Run a type-erased task.
    pub async fn run_dyn(
        &self,
        task: SharedDynTask,
        input: AnyInput,
    ) -> Result<AnyOutput, RuntimeError> {
        self.spawn_dyn(task, input).await
    }

    // -------------------------------------------------------------------------
    // Internal
    // -------------------------------------------------------------------------

    fn root_ctx(&self) -> Ctx {
        Ctx::root(
            self.frontier.clone(),
            self.exec_graph.clone(),
            self.cancel.clone(),
            Arc::clone(&self.scaffolds),
        )
    }
}
