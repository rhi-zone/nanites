//! Error types for the nanites runtime.

use std::any::TypeId;

/// Convenience alias for a heap-allocated, `Send + Sync` error.
///
/// Use `BoxError` as your task's `Error` associated type when you want to
/// return heterogeneous errors from `Task::run` without defining a bespoke
/// error enum. You can also use `anyhow::Error` for the same purpose if you
/// have `anyhow` in your dependency tree.
///
/// # Example
///
/// ```
/// use nanites_core::{Task, Ctx, BoxError};
///
/// #[derive(Debug, Clone)]
/// struct FetchUrl { url: String }
///
/// impl Task for FetchUrl {
///     type Input = ();
///     type Output = String;
///     type Error = BoxError;
///
///     async fn run(&self, _: (), _ctx: &Ctx) -> Result<String, BoxError> {
///         // any std::error::Error can be `?`-propagated here
///         Ok(format!("content of {}", self.url))
///     }
/// }
/// ```
pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Errors that can occur at runtime (distinct from task-level errors).
#[derive(Debug)]
pub enum RuntimeError {
    /// The input value had the wrong concrete type.
    InputTypeMismatch {
        /// TypeId the task expected.
        expected: TypeId,
        /// TypeId that was actually provided.
        got: TypeId,
    },
    /// A task panic or join error.
    TaskPanicked(String),
    /// Execution was cancelled.
    Cancelled,
    /// The task produced output of an unexpected type (internal invariant violation).
    OutputTypeMismatch {
        /// TypeId the caller expected.
        expected: TypeId,
        /// TypeId produced by the task.
        got: TypeId,
    },
    /// An I/O task was spawned but no executor is registered for its type.
    ///
    /// Register an executor with [`crate::Runtime::register_executor`].
    NoExecutor(&'static str),
}

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RuntimeError::InputTypeMismatch { expected, got } => {
                write!(f, "input type mismatch: expected {expected:?}, got {got:?}")
            }
            RuntimeError::TaskPanicked(msg) => write!(f, "task panicked: {msg}"),
            RuntimeError::Cancelled => write!(f, "execution cancelled"),
            RuntimeError::OutputTypeMismatch { expected, got } => {
                write!(
                    f,
                    "output type mismatch: expected {expected:?}, got {got:?}"
                )
            }
            RuntimeError::NoExecutor(type_name) => {
                write!(
                    f,
                    "no executor registered for I/O task {type_name:?}; \
                     call Runtime::register_executor before spawning this task"
                )
            }
        }
    }
}

impl std::error::Error for RuntimeError {}
