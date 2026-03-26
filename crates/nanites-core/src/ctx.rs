//! [`Ctx`] — the runtime context passed to every task.
//!
//! Ctx carries only what the runtime needs to expose to tasks:
//! - A handle to the frontier for spawning child tasks
//! - A cancellation token
//!
//! Nothing LLM-specific, nothing domain-specific belongs here.

use std::fmt;
use std::sync::Arc;

use crate::cancellation::CancellationToken;
use crate::dyn_task::{AnyInput, AnyOutput, SharedDynTask, downcast_output, erase};
use crate::error::RuntimeError;
use crate::frontier::{FrontierHandle, NodeId};
use crate::handle::TaskHandle;
use crate::scaffold::Scaffold;
use crate::task::Task;

/// Runtime context passed to every task during execution.
///
/// Use [`Ctx::spawn`] to launch child tasks (typed API) or
/// [`Ctx::spawn_dyn`] to launch them dynamically.
pub struct Ctx {
    frontier: FrontierHandle,
    cancel: CancellationToken,
    /// The frontier node id of the currently executing task.
    pub(crate) node_id: Option<NodeId>,
    /// Scaffolds inherited from the runtime.
    scaffolds: Arc<Vec<Scaffold>>,
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
        cancel: CancellationToken,
        scaffolds: Arc<Vec<Scaffold>>,
    ) -> Self {
        Ctx {
            frontier,
            cancel,
            node_id: None,
            scaffolds,
        }
    }

    /// Create a child context for a spawned task.
    pub(crate) fn child(&self, node_id: NodeId) -> Self {
        Ctx {
            frontier: self.frontier.clone(),
            cancel: self.cancel.clone(),
            node_id: Some(node_id),
            scaffolds: Arc::clone(&self.scaffolds),
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
    /// The parent–child relationship is recorded in the frontier automatically.
    /// Awaiting the returned handle creates an implicit dependency edge.
    pub fn spawn<T>(&self, task: T, input: T::Input) -> TaskHandle<T::Output>
    where
        T: Task + fmt::Debug + Clone,
        T::Error: std::error::Error + Send + Sync + 'static,
    {
        let erased: SharedDynTask = erase(task);
        let boxed_input: AnyInput = Box::new(input);

        let (handle, tx) = TaskHandle::<T::Output>::new();

        self.spawn_erased_internal(erased, boxed_input, move |result: Result<AnyOutput, _>| {
            let typed = result.and_then(downcast_output::<T::Output>);
            // If the receiver has been dropped the result is discarded — that's fine.
            let _ = tx.send(
                typed.map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) }),
            );
        });

        handle
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

        self.spawn_erased_internal(task, input, move |result| {
            let _ = tx.send(
                result.map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) }),
            );
        });

        handle
    }

    // -------------------------------------------------------------------------
    // Internal
    // -------------------------------------------------------------------------

    fn spawn_erased_internal<F>(&self, mut task: SharedDynTask, input: AnyInput, on_done: F)
    where
        F: FnOnce(Result<AnyOutput, RuntimeError>) + Send + 'static,
    {
        // Apply scaffolds.
        for scaffold in self.scaffolds.iter() {
            task = scaffold.apply(task);
        }

        let frontier = self.frontier.inner().clone();
        let parent_id = self.node_id;
        let node_id = frontier.register(task.clone(), parent_id);

        let cancel = self.cancel.clone();
        let child_ctx = self.child(node_id);

        tokio::spawn(async move {
            if cancel.is_cancelled() {
                frontier.set_cancelled(node_id);
                on_done(Err(RuntimeError::Cancelled));
                return;
            }

            frontier.set_running(node_id);

            let result = task.run_erased(input, &child_ctx).await;

            match result {
                Ok(inner) => match inner {
                    Ok(output) => {
                        frontier.set_completed(node_id);
                        on_done(Ok(output));
                    }
                    Err(e) => {
                        frontier.set_failed(node_id, e.to_string());
                        on_done(Err(RuntimeError::TaskPanicked(e.to_string())));
                    }
                },
                Err(e) => {
                    frontier.set_failed(node_id, e.to_string());
                    on_done(Err(e));
                }
            }
        });
    }
}
