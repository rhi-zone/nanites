//! [`TaskExecutor`] — the I/O execution interface for tasks that need external resources.
//!
//! Pure computation tasks implement [`Task::run`] and get execution for free via
//! the blanket implementation in the runtime. I/O tasks — those that need a
//! database connection, an HTTP client, an LLM model handle — are **pure data
//! structs** that carry only their configuration (model name, endpoint URL, etc.)
//! and delegate all execution to a registered [`TaskExecutor`].
//!
//! # Registration
//!
//! ```no_run
//! use nanites_core::{Runtime, executor::TaskExecutor};
//!
//! // Register an executor at runtime startup:
//! // runtime.register_executor(MyHttpExecutor::new(client));
//! ```
//!
//! # How the runtime dispatches
//!
//! When a task is about to run, the runtime checks its executor map for the
//! task's `type_name()`. If a registered executor is found, the executor handles
//! execution; otherwise the runtime falls back to `DynTask::run_erased` (the
//! pure-task path via `Task::run`).

use std::any::Any;
use std::pin::Pin;

use crate::ctx::Ctx;
use crate::dyn_task::{AnyInput, AnyOutput};
use crate::error::BoxError;

/// Execution interface for tasks that require external resources.
///
/// Implement this for each category of I/O task (e.g. HTTP completions,
/// database queries). The executor holds the actual resources (connection pools,
/// model handles, clients) while the task structs remain pure data.
///
/// Register executors with [`crate::Runtime::register_executor`].
pub trait TaskExecutor: Send + Sync {
    /// The `std::any::type_name` of the task type this executor handles.
    ///
    /// Must match `DynTask::type_name()` for the task structs this executor
    /// can execute. The runtime uses this as the lookup key.
    fn task_type_name(&self) -> &'static str;

    /// Execute a task given its type-erased form and type-erased input.
    ///
    /// `task_data` is the `&dyn Any` from `DynTask::task_as_any()` — downcast
    /// it to the concrete task struct to read configuration.
    ///
    /// `input` is the task's input value, boxed. Downcast with
    /// `input.downcast::<T::Input>()`.
    ///
    /// Returns the task's output value, boxed, on success.
    fn execute_erased<'a>(
        &'a self,
        task_data: &'a dyn Any,
        input: AnyInput,
        ctx: &'a Ctx,
    ) -> Pin<Box<dyn futures::Future<Output = Result<AnyOutput, BoxError>> + Send + 'a>>;
}
