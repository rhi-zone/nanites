//! [`Runtime`] — the entry point for task execution.
//!
//! The runtime:
//! 1. Owns the [`Frontier`], the [`ExecGraph`], a list of global [`Scaffold`]s,
//!    and a map of [`TaskExecutor`]s for I/O tasks.
//! 2. Applies scaffolds to each task before execution.
//! 3. Dispatches I/O tasks to registered executors; pure tasks run via
//!    `DynTask::run_erased` (which calls `Task::run`).
//! 4. Drives tasks on the current tokio executor.
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
//! # Executors for I/O tasks
//!
//! Register executors with [`Runtime::register_executor`]. The runtime looks up
//! the task's `type_name()` and dispatches to the matching executor. Pure tasks
//! (those implementing `Task::run`) do not need a registered executor.
//!
//! # Cancellation
//!
//! Every runtime has a root [`CancellationToken`]. Cancel it with
//! [`Runtime::cancel`] to signal all in-flight tasks.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use crate::cache::TaskCache;
use crate::cancellation::CancellationToken;
use crate::ctx::Ctx;
use crate::dyn_task::{AnyInput, AnyOutput, SharedDynTask};
use crate::error::RuntimeError;
use crate::exec_graph::ExecGraph;
use crate::executor::TaskExecutor;
use crate::frontier::{Frontier, FrontierHandle};
use crate::handle::TaskHandle;
use crate::scaffold::Scaffold;
use crate::task::Task;

/// Shared executor map — keyed by `std::any::type_name` of the task type.
type ExecutorMap = Arc<HashMap<&'static str, Arc<dyn TaskExecutor>>>;

/// The nanites runtime.
///
/// Cheap to clone — all clones share the same frontier, execution graph,
/// cancel token, scaffold list, executor map, and optional cache.
#[derive(Clone)]
pub struct Runtime {
    frontier: FrontierHandle,
    exec_graph: ExecGraph,
    cancel: CancellationToken,
    scaffolds: Arc<Vec<Scaffold>>,
    executors: ExecutorMap,
    /// Optional result cache. `None` means no caching (the default).
    cache: Option<Arc<dyn TaskCache>>,
}

impl fmt::Debug for Runtime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Runtime")
            .field("frontier_len", &self.frontier.inner().len())
            .field("exec_graph_len", &self.exec_graph.len())
            .field("scaffolds", &self.scaffolds.len())
            .field("executors", &self.executors.len())
            .finish_non_exhaustive()
    }
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}

impl Runtime {
    /// Create a runtime with no scaffolds, no executors, no cache, and a fresh
    /// cancellation token.
    pub fn new() -> Self {
        Runtime {
            frontier: FrontierHandle::new(Frontier::new()),
            exec_graph: ExecGraph::new(),
            cancel: CancellationToken::new(),
            scaffolds: Arc::new(Vec::new()),
            executors: Arc::new(HashMap::new()),
            cache: None,
        }
    }

    /// Attach a result cache to this runtime.
    ///
    /// Tasks wrapped with [`crate::dyn_task::erase_serializable`] will check
    /// the cache before executing and store successful outputs after executing.
    ///
    /// Returns `self` for method chaining.
    #[must_use]
    pub fn with_cache(mut self, cache: Arc<dyn TaskCache>) -> Self {
        self.cache = Some(cache);
        self
    }

    /// Add a global scaffold (applied to every spawned task, in order).
    ///
    /// Returns `self` for method chaining.
    #[must_use]
    pub fn with_scaffold(mut self, scaffold: Scaffold) -> Self {
        Arc::make_mut(&mut self.scaffolds).push(scaffold);
        self
    }

    /// Register an executor for an I/O task type.
    ///
    /// The executor's [`TaskExecutor::task_type_name`] is used as the lookup
    /// key. When a task with that type name is spawned, the runtime dispatches
    /// to this executor instead of calling `DynTask::run_erased`.
    ///
    /// Returns `self` for method chaining.
    #[must_use]
    pub fn with_executor(mut self, executor: impl TaskExecutor + 'static) -> Self {
        let key = executor.task_type_name();
        Arc::make_mut(&mut self.executors).insert(key, Arc::new(executor));
        self
    }

    /// Register an executor in place (mutable borrow variant).
    ///
    /// Prefer [`Runtime::with_executor`] when building the runtime; use this
    /// for late registration after the runtime has already been cloned.
    pub fn register_executor(&mut self, executor: impl TaskExecutor + 'static) {
        let key = executor.task_type_name();
        Arc::make_mut(&mut self.executors).insert(key, Arc::new(executor));
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
            Arc::clone(&self.executors),
            self.cache.clone(),
        )
    }
}
