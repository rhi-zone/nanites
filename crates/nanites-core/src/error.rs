//! Error types for the nanites runtime.

use std::any::TypeId;

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
        }
    }
}

impl std::error::Error for RuntimeError {}
