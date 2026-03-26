//! [`Ctx`] — the runtime context passed to every task.
//!
//! Ctx carries only what the runtime needs to expose to tasks:
//! - A handle to the frontier for spawning child tasks
//! - A handle to the execution graph for recording lineage
//! - A cancellation token
//!
//! Nothing LLM-specific, nothing domain-specific belongs here.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use crate::cancellation::CancellationToken;
use crate::dyn_task::{AnyInput, AnyOutput, SharedDynTask, downcast_output, erase};
use crate::error::RuntimeError;
use crate::exec_graph::ExecGraph;
use crate::executor::TaskExecutor;
use crate::frontier::{FrontierHandle, NodeId};
use crate::handle::TaskHandle;
use crate::scaffold::Scaffold;
use crate::task::Task;

/// Runtime context passed to every task during execution.
///
/// Use [`Ctx::spawn`] to launch child tasks (typed API),
/// [`Ctx::spawn_dyn`] to launch them dynamically, or
/// [`Ctx::spawn_all`] for fan-out over a collection of inputs.
pub struct Ctx {
    frontier: FrontierHandle,
    exec_graph: ExecGraph,
    cancel: CancellationToken,
    /// The frontier node id of the currently executing task.
    pub(crate) node_id: Option<NodeId>,
    /// Scaffolds inherited from the runtime.
    scaffolds: Arc<Vec<Scaffold>>,
    /// Registered I/O task executors, keyed by task type name.
    executors: Arc<HashMap<&'static str, Arc<dyn TaskExecutor>>>,
}

impl fmt::Debug for Ctx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Ctx")
            .field("node_id", &self.node_id)
            .field("cancelled", &self.cancel.is_cancelled())
            .finish_non_exhaustive()
    }
}

impl Ctx {
    /// Create a root context (used by the runtime for top-level tasks).
    pub(crate) fn root(
        frontier: FrontierHandle,
        exec_graph: ExecGraph,
        cancel: CancellationToken,
        scaffolds: Arc<Vec<Scaffold>>,
        executors: Arc<HashMap<&'static str, Arc<dyn TaskExecutor>>>,
    ) -> Self {
        Ctx {
            frontier,
            exec_graph,
            cancel,
            node_id: None,
            scaffolds,
            executors,
        }
    }

    /// Create a child context for a spawned task.
    pub(crate) fn child(&self, node_id: NodeId) -> Self {
        Ctx {
            frontier: self.frontier.clone(),
            exec_graph: self.exec_graph.clone(),
            cancel: self.cancel.clone(),
            node_id: Some(node_id),
            scaffolds: Arc::clone(&self.scaffolds),
            executors: Arc::clone(&self.executors),
        }
    }

    /// Returns `true` if cancellation has been signalled.
    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// The cancellation token for this context.
    pub fn cancellation_token(&self) -> &CancellationToken {
        &self.cancel
    }

    // -------------------------------------------------------------------------
    // Typed spawn
    // -------------------------------------------------------------------------

    /// Spawn a child task, returning a handle that resolves to its output.
    ///
    /// The parent–child relationship is recorded in the frontier and execution
    /// graph automatically. Awaiting the returned handle creates an implicit
    /// dependency edge.
    pub fn spawn<T>(&self, task: T, input: T::Input) -> TaskHandle<T::Output>
    where
        T: Task + fmt::Debug + Clone,
        T::Error: std::error::Error + Send + Sync + 'static,
    {
        let erased: SharedDynTask = erase(task);
        let boxed_input: AnyInput = Box::new(input);

        let (handle, tx) = TaskHandle::<T::Output>::new();

        self.spawn_erased_internal(
            erased,
            boxed_input,
            std::any::type_name::<T::Output>(),
            move |result: Result<AnyOutput, _>| {
                let typed = result.and_then(downcast_output::<T::Output>);
                // If the receiver has been dropped the result is discarded — that's fine.
                let _ = tx.send(
                    typed.map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) }),
                );
            },
        );

        handle
    }

    /// Fan-out: spawn the same task once per input, returning one handle per
    /// output.
    ///
    /// All child tasks are registered in the frontier and execution graph before
    /// any of them begin executing. Equivalent to calling `ctx.spawn(task.clone(),
    /// input)` in a loop, but more ergonomic for fan-out patterns.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use nanites_core::{Task, Ctx, Runtime};
    /// # #[derive(Debug, Clone)] struct Double;
    /// # impl Task for Double {
    /// #     type Input = i64; type Output = i64; type Error = std::convert::Infallible;
    /// #     async fn run(&self, input: i64, _: &Ctx) -> Result<i64, Self::Error> { Ok(input * 2) }
    /// # }
    /// # async fn example(ctx: &Ctx) {
    /// let handles = ctx.spawn_all(Double, vec![1i64, 2, 3, 4]);
    /// let results: Vec<_> = futures::future::join_all(handles).await;
    /// # }
    /// ```
    pub fn spawn_all<T>(&self, task: T, inputs: Vec<T::Input>) -> Vec<TaskHandle<T::Output>>
    where
        T: Task + fmt::Debug + Clone,
        T::Error: std::error::Error + Send + Sync + 'static,
    {
        inputs
            .into_iter()
            .map(|input| self.spawn(task.clone(), input))
            .collect()
    }

    // -------------------------------------------------------------------------
    // Dynamic spawn
    // -------------------------------------------------------------------------

    /// Spawn a child task dynamically (type-erased).
    ///
    /// `input` must be a `Box<T::Input>` for the concrete task — the runtime
    /// checks the TypeId at call time and returns an error handle on mismatch.
    ///
    /// Returns a handle that resolves to a type-erased `AnyOutput`.
    pub fn spawn_dyn(&self, task: SharedDynTask, input: AnyInput) -> TaskHandle<AnyOutput> {
        let (handle, tx) = TaskHandle::<AnyOutput>::new();

        self.spawn_erased_internal(task, input, "dyn", move |result| {
            let _ = tx.send(
                result.map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) }),
            );
        });

        handle
    }

    // -------------------------------------------------------------------------
    // Internal
    // -------------------------------------------------------------------------

    fn spawn_erased_internal<F>(
        &self,
        mut task: SharedDynTask,
        input: AnyInput,
        output_type_name: &'static str,
        on_done: F,
    ) where
        F: FnOnce(Result<AnyOutput, RuntimeError>) + Send + 'static,
    {
        // Apply scaffolds.
        for scaffold in self.scaffolds.iter() {
            task = scaffold.apply(task);
        }

        let frontier = self.frontier.inner().clone();
        let exec_graph = self.exec_graph.clone();
        let parent_id = self.node_id;
        let node_id = frontier.register(task.clone(), parent_id);

        // Record in the execution graph before execution begins.
        exec_graph.record_spawn(node_id, task.type_name(), None, parent_id);

        let cancel = self.cancel.clone();
        let child_ctx = self.child(node_id);

        // Look up a registered executor for this task type.
        let executor: Option<Arc<dyn crate::executor::TaskExecutor>> =
            self.executors.get(task.type_name()).cloned();

        tokio::spawn(async move {
            if cancel.is_cancelled() {
                frontier.set_cancelled(node_id);
                frontier.remove_terminal(node_id);
                exec_graph.set_cancelled(node_id);
                on_done(Err(RuntimeError::Cancelled));
                return;
            }

            frontier.set_running(node_id);

            // Dispatch: registered executor takes priority over run_erased.
            let outcome: Result<AnyOutput, RuntimeError> = if let Some(exec) = executor {
                exec.execute_erased(task.task_as_any(), input, &child_ctx)
                    .await
                    .map_err(|e| RuntimeError::TaskPanicked(e.to_string()))
            } else {
                let result = task.run_erased(input, &child_ctx).await;
                match result {
                    Ok(inner) => inner.map_err(|e| RuntimeError::TaskPanicked(e.to_string())),
                    Err(e) => Err(e),
                }
            };

            match outcome {
                Ok(output) => {
                    frontier.set_completed(node_id);
                    frontier.remove_terminal(node_id);
                    exec_graph.set_completed(node_id, output_type_name);
                    on_done(Ok(output));
                }
                Err(e) => {
                    let msg = e.to_string();
                    frontier.set_failed(node_id, msg.clone());
                    frontier.remove_terminal(node_id);
                    exec_graph.set_failed(node_id, msg);
                    on_done(Err(e));
                }
            }
        });
    }
}
