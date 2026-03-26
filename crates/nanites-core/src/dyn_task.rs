//! Type-erased task layer.
//!
//! The runtime and frontier operate on [`DynTask`] trait objects so they can
//! hold tasks of heterogeneous types without generics. Concrete [`Task`]
//! implementors are wrapped in [`ErasedTask`], which boxes the run function
//! and carries type-identity information for runtime checks.
//!
//! I/O tasks — those that need external resources and are executed by a
//! registered [`crate::executor::TaskExecutor`] — are wrapped in
//! [`ErasedIoTask`]. They implement `DynTask` but `run_erased` returns an
//! error directing the caller to register an executor.
//!
//! # Type safety model
//!
//! - **Static** (`ctx.spawn::<T>(task, input)`) — full compile-time checking.
//! - **Dynamic** (`ctx.spawn_dyn(erased, input_box)`) — type-checked at the
//!   `Box<dyn Any + Send>` boundary; mismatches produce [`RuntimeError::InputTypeMismatch`].

use std::any::{Any, TypeId};
use std::fmt;
use std::pin::Pin;
use std::sync::Arc;

use crate::ctx::Ctx;
use crate::error::RuntimeError;

/// Type-erased input value.
pub type AnyInput = Box<dyn Any + Send>;
/// Type-erased output value.
pub type AnyOutput = Box<dyn Any + Send>;

/// Type-erased result from a dynamic task execution.
pub type DynResult = Result<AnyOutput, Box<dyn std::error::Error + Send + Sync>>;

/// Type identity for a task's input and output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskTypes {
    /// [`TypeId`] of the task's `Input` associated type.
    pub input: TypeId,
    /// [`TypeId`] of the task's `Output` associated type.
    pub output: TypeId,
}

/// Trait object interface for tasks.
///
/// Implemented by [`ErasedTask`] (pure tasks) and [`ErasedIoTask`] (I/O tasks).
/// You should not need to implement this manually.
pub trait DynTask: Send + Sync + fmt::Debug {
    /// Human-readable name of the concrete task type (for logging / frontier display).
    fn type_name(&self) -> &'static str;

    /// TypeIds of Input and Output.
    fn task_types(&self) -> TaskTypes;

    /// Execute with type-erased input.
    ///
    /// For pure tasks (backed by [`ErasedTask`]), this delegates to
    /// [`crate::Task::run`].
    ///
    /// For I/O tasks (backed by [`ErasedIoTask`]), this returns
    /// [`RuntimeError::NoExecutor`] — the runtime dispatches to a registered
    /// [`crate::executor::TaskExecutor`] instead of calling this.
    ///
    /// Returns [`RuntimeError::InputTypeMismatch`] if `input` is not the
    /// expected concrete type.
    fn run_erased<'a>(
        &'a self,
        input: AnyInput,
        ctx: &'a Ctx,
    ) -> Pin<Box<dyn futures::Future<Output = Result<DynResult, RuntimeError>> + Send + 'a>>;

    /// Clone into a fresh `Box<dyn DynTask>`.
    fn clone_box(&self) -> Box<dyn DynTask>;

    /// Downcast to `&dyn Any` pointing at the **inner task struct** (not the wrapper).
    ///
    /// Used by [`crate::executor::TaskExecutor`] implementations to recover the
    /// concrete task configuration. For example, a `RigCompletionExecutor` calls
    /// `task_as_any().downcast_ref::<CompletionTask>()` to read the model name.
    fn task_as_any(&self) -> &dyn Any;

    /// Downcast to `&dyn Any` for inspection or scaffold use.
    ///
    /// Returns the wrapper (`ErasedTask<T>` or `ErasedIoTask<T>`). For access to
    /// the inner task struct, use [`DynTask::task_as_any`] instead.
    fn as_any(&self) -> &dyn Any;
}

impl Clone for Box<dyn DynTask> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

/// Concrete wrapper that erases a [`Task`] implementor into a [`DynTask`].
///
/// # Internals
///
/// Stores the task behind an `Arc` so cloning is cheap and `run_erased` can
/// borrow `self` across await points without lifetime issues.
pub struct ErasedTask<T> {
    inner: Arc<T>,
}

impl<T: crate::task::Task + fmt::Debug + Clone> ErasedTask<T> {
    /// Wrap a task.
    pub fn new(task: T) -> Self {
        Self {
            inner: Arc::new(task),
        }
    }
}

impl<T: crate::task::Task + fmt::Debug + Clone> fmt::Debug for ErasedTask<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ErasedTask({})", std::any::type_name::<T>())
    }
}

impl<T> DynTask for ErasedTask<T>
where
    T: crate::task::Task + fmt::Debug + Clone,
    T::Error: std::error::Error + Send + Sync + 'static,
{
    fn type_name(&self) -> &'static str {
        std::any::type_name::<T>()
    }

    fn task_types(&self) -> TaskTypes {
        TaskTypes {
            input: TypeId::of::<T::Input>(),
            output: TypeId::of::<T::Output>(),
        }
    }

    fn run_erased<'a>(
        &'a self,
        input: AnyInput,
        ctx: &'a Ctx,
    ) -> Pin<Box<dyn futures::Future<Output = Result<DynResult, RuntimeError>> + Send + 'a>> {
        let task = Arc::clone(&self.inner);

        // Downcast the erased input to the concrete type.
        let typed_input = match input.downcast::<T::Input>() {
            Ok(v) => *v,
            Err(rejected) => {
                let got = (*rejected).type_id();
                return Box::pin(async move {
                    Err(RuntimeError::InputTypeMismatch {
                        expected: TypeId::of::<T::Input>(),
                        got,
                    })
                });
            }
        };

        Box::pin(async move {
            let result = task.run(typed_input, ctx).await;
            Ok(result
                .map(|out| -> AnyOutput { Box::new(out) })
                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) }))
        })
    }

    fn clone_box(&self) -> Box<dyn DynTask> {
        Box::new(ErasedTask {
            inner: Arc::clone(&self.inner),
        })
    }

    fn task_as_any(&self) -> &dyn Any {
        self.inner.as_ref()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Shared, cheaply cloneable [`DynTask`].
pub type SharedDynTask = Arc<dyn DynTask>;

/// Wrap a concrete task as a [`SharedDynTask`].
pub fn erase<T>(task: T) -> SharedDynTask
where
    T: crate::task::Task + fmt::Debug + Clone,
    T::Error: std::error::Error + Send + Sync + 'static,
{
    Arc::new(ErasedTask::new(task))
}

// ─── IoTask ───────────────────────────────────────────────────────────────────

/// Marker trait for tasks that require an external executor.
///
/// I/O tasks carry only configuration (model name, endpoint URL, etc.).
/// They do **not** implement [`crate::Task::run`]. Instead, a
/// [`crate::executor::TaskExecutor`] registered with the runtime handles
/// execution.
///
/// Use [`erase_io`] to wrap an `IoTask` into a [`SharedDynTask`].
///
/// # Example
///
/// ```no_run
/// use nanites_core::dyn_task::{IoTask, erase_io};
///
/// #[derive(Debug, Clone)]
/// struct FetchUrl { url: String }
///
/// impl IoTask for FetchUrl {
///     type Input = ();
///     type Output = String;
/// }
/// ```
pub trait IoTask: Send + Sync + fmt::Debug + Clone + 'static {
    /// The input value this task consumes.
    type Input: Send + 'static;
    /// The value produced on success.
    type Output: Send + 'static;
}

/// Concrete wrapper that erases an [`IoTask`] into a [`DynTask`].
///
/// Analogous to [`ErasedTask`] for pure tasks. `run_erased` returns
/// [`RuntimeError::NoExecutor`]; the runtime dispatches to the registered
/// [`crate::executor::TaskExecutor`] instead.
pub struct ErasedIoTask<T> {
    inner: Arc<T>,
}

impl<T: IoTask> ErasedIoTask<T> {
    /// Wrap an I/O task.
    pub fn new(task: T) -> Self {
        Self {
            inner: Arc::new(task),
        }
    }
}

impl<T: IoTask + fmt::Debug> fmt::Debug for ErasedIoTask<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ErasedIoTask({})", std::any::type_name::<T>())
    }
}

impl<T: IoTask> DynTask for ErasedIoTask<T> {
    fn type_name(&self) -> &'static str {
        std::any::type_name::<T>()
    }

    fn task_types(&self) -> TaskTypes {
        TaskTypes {
            input: std::any::TypeId::of::<T::Input>(),
            output: std::any::TypeId::of::<T::Output>(),
        }
    }

    fn run_erased<'a>(
        &'a self,
        _input: AnyInput,
        _ctx: &'a Ctx,
    ) -> Pin<Box<dyn futures::Future<Output = Result<DynResult, RuntimeError>> + Send + 'a>> {
        let name = std::any::type_name::<T>();
        Box::pin(async move { Err(RuntimeError::NoExecutor(name)) })
    }

    fn clone_box(&self) -> Box<dyn DynTask> {
        Box::new(ErasedIoTask {
            inner: Arc::clone(&self.inner),
        })
    }

    fn task_as_any(&self) -> &dyn Any {
        self.inner.as_ref()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Wrap an [`IoTask`] as a [`SharedDynTask`].
///
/// Use this instead of [`erase`] for tasks that require an external executor.
pub fn erase_io<T: IoTask>(task: T) -> SharedDynTask {
    Arc::new(ErasedIoTask::new(task))
}

/// Downcast an [`AnyOutput`] to a concrete type.
///
/// Returns `Err(RuntimeError::OutputTypeMismatch)` if the types don't match.
pub fn downcast_output<O: 'static>(output: AnyOutput) -> Result<O, RuntimeError> {
    output
        .downcast::<O>()
        .map(|b| *b)
        .map_err(|rejected| RuntimeError::OutputTypeMismatch {
            expected: TypeId::of::<O>(),
            got: (*rejected).type_id(),
        })
}
