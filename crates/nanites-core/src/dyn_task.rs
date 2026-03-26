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

    /// Return serialization info for cache keying, if available.
    ///
    /// Returns `Some((stable_type_name, params_json))` for tasks that implement
    /// [`crate::registry::SerializableTask`], `None` for all others.
    ///
    /// The default implementation returns `None`. [`ErasedTask`] overrides this
    /// when the concrete task type implements `SerializableTask`.
    fn serializable_info(&self) -> Option<(&'static str, serde_json::Value)> {
        None
    }

    /// Hash the type-erased input for cache keying.
    ///
    /// Returns `Some(hash)` when the task was wrapped with [`erase_serializable`]
    /// and the input type implements `serde::Serialize`. Returns `None` otherwise.
    ///
    /// The input is not consumed; only a shared reference is taken.
    fn hash_input_erased(&self, _input: &AnyInput) -> Option<u64> {
        None
    }

    /// Serialize the type-erased input to a JSON value for checkpointing.
    ///
    /// Returns `Some(json)` when the task was wrapped with [`erase_serializable`]
    /// and the input type implements `serde::Serialize`. Returns `None` otherwise.
    ///
    /// The input is not consumed; only a shared reference is taken.
    fn serialize_input_erased(&self, _input: &AnyInput) -> Option<serde_json::Value> {
        None
    }

    /// Serialize the type-erased output to a JSON value for checkpointing.
    ///
    /// Returns `Some(json)` when the task was wrapped with [`erase_serializable`]
    /// and the output type implements `serde::Serialize`. Returns `None` otherwise.
    ///
    /// The output is not consumed; only a shared reference is taken.
    fn serialize_output_erased(&self, _output: &AnyOutput) -> Option<serde_json::Value> {
        None
    }

    /// Wrap a successful output into a [`crate::cache::CachedOutput`] for storage.
    ///
    /// Returns `Some(cached)` when the task was wrapped with [`erase_serializable`]
    /// and the output type implements `Clone + Send + Sync`. Returns `None` otherwise,
    /// in which case the result is not stored in the cache.
    fn make_cached_erased(&self, _output: AnyOutput) -> Option<crate::cache::CachedOutput> {
        None
    }
}

impl Clone for Box<dyn DynTask> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

/// Type alias for the optional params-extraction closure stored in [`ErasedTask`].
///
/// Returns `(stable_type_name, params_json)` when called.
type ParamsFn = Arc<dyn Fn() -> (&'static str, serde_json::Value) + Send + Sync>;

/// Type alias for the optional input-hashing closure stored in [`ErasedTask`].
///
/// Receives a `&AnyInput`, tries to downcast and serialize, returns `Some(hash)`
/// or `None` if serialization is not possible.
type InputHashFn = Arc<dyn Fn(&AnyInput) -> Option<u64> + Send + Sync>;

/// Type alias for the optional input-serialization closure stored in [`ErasedTask`].
///
/// Receives a `&AnyInput`, tries to downcast and serialize to JSON, returns
/// `Some(json)` or `None` if serialization is not possible.
type SerializeInputFn = Arc<dyn Fn(&AnyInput) -> Option<serde_json::Value> + Send + Sync>;

/// Type alias for the optional output-serialization closure stored in [`ErasedTask`].
///
/// Receives a `&AnyOutput`, tries to downcast and serialize to JSON, returns
/// `Some(json)` or `None` if serialization is not possible.
type SerializeOutputFn = Arc<dyn Fn(&AnyOutput) -> Option<serde_json::Value> + Send + Sync>;

/// Type alias for the optional output-wrapping closure stored in [`ErasedTask`].
///
/// Receives an `AnyOutput` and wraps it into a [`crate::cache::CachedOutput`]
/// for storage in the cache. Returns `None` if the output can't be cached.
type MakeCachedFn = Arc<dyn Fn(AnyOutput) -> Option<crate::cache::CachedOutput> + Send + Sync>;

/// Concrete wrapper that erases a [`Task`] implementor into a [`DynTask`].
///
/// # Internals
///
/// Stores the task behind an `Arc` so cloning is cheap and `run_erased` can
/// borrow `self` across await points without lifetime issues.
///
/// Optionally stores closures for tasks that implement
/// [`crate::registry::SerializableTask`] and have a serializable input type.
/// Use [`erase_serializable`] to create an erased task with caching support.
pub struct ErasedTask<T> {
    inner: Arc<T>,
    /// Params-extraction closure, present when the task implements
    /// `SerializableTask`. `None` for non-serializable tasks.
    params_fn: Option<ParamsFn>,
    /// Input-hashing closure, present when `T::Input: serde::Serialize`.
    input_hash_fn: Option<InputHashFn>,
    /// Input-serialization closure, present when `T::Input: serde::Serialize`.
    serialize_input_fn: Option<SerializeInputFn>,
    /// Output-serialization closure, present when `T::Output: serde::Serialize`.
    serialize_output_fn: Option<SerializeOutputFn>,
    /// Output-wrapping closure for the cache, present when
    /// `T::Output: Clone + Send + Sync`.
    make_cached_fn: Option<MakeCachedFn>,
}

impl<T: crate::task::Task + fmt::Debug + Clone> ErasedTask<T> {
    /// Wrap a task (no params extraction or input hashing).
    pub fn new(task: T) -> Self {
        Self {
            inner: Arc::new(task),
            params_fn: None,
            input_hash_fn: None,
            serialize_input_fn: None,
            serialize_output_fn: None,
            make_cached_fn: None,
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
            params_fn: self.params_fn.clone(),
            input_hash_fn: self.input_hash_fn.clone(),
            serialize_input_fn: self.serialize_input_fn.clone(),
            serialize_output_fn: self.serialize_output_fn.clone(),
            make_cached_fn: self.make_cached_fn.clone(),
        })
    }

    fn task_as_any(&self) -> &dyn Any {
        self.inner.as_ref()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn serializable_info(&self) -> Option<(&'static str, serde_json::Value)> {
        self.params_fn.as_ref().map(|f| f())
    }

    fn hash_input_erased(&self, input: &AnyInput) -> Option<u64> {
        self.input_hash_fn.as_ref().and_then(|f| f(input))
    }

    fn serialize_input_erased(&self, input: &AnyInput) -> Option<serde_json::Value> {
        self.serialize_input_fn.as_ref().and_then(|f| f(input))
    }

    fn serialize_output_erased(&self, output: &AnyOutput) -> Option<serde_json::Value> {
        self.serialize_output_fn.as_ref().and_then(|f| f(output))
    }

    fn make_cached_erased(&self, output: AnyOutput) -> Option<crate::cache::CachedOutput> {
        self.make_cached_fn.as_ref().and_then(|f| f(output))
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

/// Wrap a concrete task that also implements [`crate::registry::SerializableTask`]
/// as a [`SharedDynTask`] with params extraction, input hashing, and
/// input/output serialization enabled.
///
/// Tasks wrapped with this function will have their serializable info exposed
/// via [`DynTask::serializable_info`], their inputs hashed via
/// [`DynTask::hash_input_erased`] (enabling the runtime cache), and their
/// inputs/outputs serialized via [`DynTask::serialize_input_erased`] /
/// [`DynTask::serialize_output_erased`] (enabling checkpoint/restore).
///
/// `T::Input` and `T::Output` must implement `serde::Serialize`.
/// If serialization fails at runtime, caching and checkpointing are skipped
/// for that invocation (best-effort, no panic).
pub fn erase_serializable<T>(task: T) -> SharedDynTask
where
    T: crate::task::Task
        + crate::registry::SerializableTask
        + fmt::Debug
        + Clone
        + Send
        + Sync
        + 'static,
    T::Input: serde::Serialize,
    T::Output: serde::Serialize + Clone + Send + Sync + 'static,
    T::Error: std::error::Error + Send + Sync + 'static,
{
    let inner = Arc::new(task);
    let inner_for_params = Arc::clone(&inner);
    let params_fn: ParamsFn = Arc::new(move || {
        let type_name = inner_for_params.serializable_type_name();
        let params = inner_for_params.params();
        (type_name, params)
    });
    let input_hash_fn: InputHashFn = Arc::new(|input: &AnyInput| {
        let typed = input.downcast_ref::<T::Input>()?;
        crate::cache::hash_input(typed)
    });
    let serialize_input_fn: SerializeInputFn = Arc::new(|input: &AnyInput| {
        let typed = input.downcast_ref::<T::Input>()?;
        serde_json::to_value(typed).ok()
    });
    let serialize_output_fn: SerializeOutputFn = Arc::new(|output: &AnyOutput| {
        let typed = output.downcast_ref::<T::Output>()?;
        serde_json::to_value(typed).ok()
    });
    let make_cached_fn: MakeCachedFn = Arc::new(|output: AnyOutput| {
        let typed = *output.downcast::<T::Output>().ok()?;
        Some(crate::cache::make_cached(typed))
    });
    Arc::new(ErasedTask {
        inner,
        params_fn: Some(params_fn),
        input_hash_fn: Some(input_hash_fn),
        serialize_input_fn: Some(serialize_input_fn),
        serialize_output_fn: Some(serialize_output_fn),
        make_cached_fn: Some(make_cached_fn),
    })
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
