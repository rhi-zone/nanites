//! [`TaskHandle<O>`] — a future representing a spawned task's eventual output.
//!
//! Awaiting a handle blocks the current task until the child task completes,
//! creating an implicit dependency edge in the task graph. Handles are the
//! primary mechanism for expressing dataflow: pass a handle as input to another
//! spawn call and the runtime captures the happens-before relationship.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::sync::oneshot;

use crate::error::RuntimeError;

/// Boxed task error type used internally by handles.
pub(crate) type TaskError = Box<dyn std::error::Error + Send + Sync>;

/// A future that resolves to the output of a spawned child task.
///
/// Obtained from [`crate::Ctx::spawn`] or [`crate::Ctx::spawn_dyn`].
/// Awaiting this future suspends the parent task until the child completes.
pub struct TaskHandle<O> {
    receiver: oneshot::Receiver<Result<O, TaskError>>,
}

impl<O: Send + 'static> TaskHandle<O> {
    /// Create a handle + the sender half used by the runtime to deliver results.
    #[allow(clippy::type_complexity)]
    pub(crate) fn new() -> (Self, oneshot::Sender<Result<O, TaskError>>) {
        let (tx, rx) = oneshot::channel();
        (TaskHandle { receiver: rx }, tx)
    }
}

impl<O: Send + 'static> Future for TaskHandle<O> {
    type Output = Result<O, RuntimeError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match Pin::new(&mut self.receiver).poll(cx) {
            Poll::Ready(Ok(result)) => {
                Poll::Ready(result.map_err(|e| RuntimeError::TaskPanicked(e.to_string())))
            }
            Poll::Ready(Err(_)) => {
                // Sender dropped — the runtime was shut down or the task was cancelled.
                Poll::Ready(Err(RuntimeError::Cancelled))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}
