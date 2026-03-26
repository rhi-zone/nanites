//! Combinator task wrappers.
//!
//! Each combinator is a **serializable struct** that wraps another task and
//! composes it in some way. They implement [`Task`] themselves, so combinators
//! can be nested arbitrarily.
//!
//! # Available combinators
//!
//! | Combinator | Description |
//! |------------|-------------|
//! | [`Map`]    | Broadcast a task over a `Vec` of inputs (fan-out / fan-in). |
//! | [`Refine`] | Run the inner task in a loop until it signals completion. |
//! | [`Retry`]  | Re-run the inner task on error, up to `max_attempts` times. |

use std::fmt;

use futures::future::join_all;
use serde::{Deserialize, Serialize};

use crate::ctx::Ctx;
use crate::error::RuntimeError;
use crate::task::Task;

// ─── Map ─────────────────────────────────────────────────────────────────────

/// Broadcast a task over a collection of inputs.
///
/// `Map<T>` takes a `Vec<T::Input>`, fans out to one spawned child per element,
/// and collects all outputs into a `Vec<T::Output>`. The children run
/// concurrently; all handles are awaited before returning.
///
/// # Error handling
///
/// If any child task fails, `Map` returns the first [`RuntimeError`] encountered
/// (in input order). The remaining handles are dropped.
///
/// # Example
///
/// ```no_run
/// use nanites_core::{Task, Ctx, Runtime, combinators::Map};
///
/// #[derive(Debug, Clone)]
/// struct Double;
///
/// impl Task for Double {
///     type Input = i64;
///     type Output = i64;
///     type Error = std::convert::Infallible;
///
///     async fn run(&self, input: i64, _ctx: &Ctx) -> Result<i64, Self::Error> {
///         Ok(input * 2)
///     }
/// }
///
/// # async fn example() {
/// let runtime = nanites_core::Runtime::new();
/// let result = runtime.run(Map { inner: Double }, vec![1i64, 2, 3]).await.unwrap();
/// assert_eq!(result, vec![2, 4, 6]);
/// # }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Map<T> {
    /// The inner task, applied to each element.
    pub inner: T,
}

impl<T> Task for Map<T>
where
    T: Task + fmt::Debug + Clone,
    T::Error: std::error::Error + Send + Sync + 'static,
{
    type Input = Vec<T::Input>;
    type Output = Vec<T::Output>;
    type Error = RuntimeError;

    async fn run(&self, input: Self::Input, ctx: &Ctx) -> Result<Self::Output, Self::Error> {
        let handles = ctx.spawn_all(self.inner.clone(), input);
        let results: Vec<Result<T::Output, RuntimeError>> = join_all(handles).await;

        let mut outputs = Vec::with_capacity(results.len());
        for result in results {
            outputs.push(result?);
        }
        Ok(outputs)
    }
}

// ─── Refine ──────────────────────────────────────────────────────────────────

/// The outcome of one refinement step.
///
/// Returned by [`Refineable::into_step`] to signal whether iteration should
/// stop or continue with a new input.
#[derive(Debug)]
pub enum RefinementStep<Final, Input> {
    /// Iteration is complete; yield this value.
    Done(Final),
    /// Continue with this new input on the next iteration.
    Continue(Input),
}

/// Constraint for tasks used with [`Refine`].
///
/// Implement this for a task whose output either signals completion (`Final`)
/// or requests another iteration with a (potentially modified) input. The
/// conversion from `T::Output` to a [`RefinementStep`] is expressed here to
/// keep `Refine` free of extra type parameters.
///
/// # Example
///
/// ```no_run
/// use nanites_core::{Task, Ctx};
/// use nanites_core::combinators::{Refineable, RefinementStep};
///
/// #[derive(Debug, Clone)]
/// struct Improve;
///
/// impl Task for Improve {
///     type Input = String;
///     type Output = Result<String, String>; // Ok = done, Err = retry with new draft
///     type Error = std::convert::Infallible;
///
///     async fn run(&self, input: String, _ctx: &Ctx) -> Result<Self::Output, Self::Error> {
///         if input.len() > 20 {
///             Ok(Ok(input)) // good enough
///         } else {
///             Ok(Err(input + " (expanded)")) // keep going
///         }
///     }
/// }
///
/// impl Refineable for Improve {
///     type Final = String;
///
///     fn into_step(output: Self::Output) -> RefinementStep<Self::Final, Self::Input> {
///         match output {
///             Ok(final_val) => RefinementStep::Done(final_val),
///             Err(next_input) => RefinementStep::Continue(next_input),
///         }
///     }
/// }
/// ```
pub trait Refineable: Task {
    /// The final output type when iteration is complete.
    type Final: Send + 'static;

    /// Convert the task's output into a refinement step.
    fn into_step(output: Self::Output) -> RefinementStep<Self::Final, Self::Input>;
}

/// Run the inner task in a loop until it signals completion.
///
/// On each iteration the inner task is spawned with the current input. Its
/// output is passed through [`Refineable::into_step`]:
///
/// - [`RefinementStep::Done`] — return the final value immediately.
/// - [`RefinementStep::Continue`] — use the new input for the next iteration.
///
/// If `max_iterations` is reached before the task signals `Done`, the combinator
/// returns [`RuntimeError::TaskPanicked`] with a descriptive message.
///
/// # Example
///
/// ```no_run
/// use nanites_core::{Runtime, combinators::Refine};
/// # use nanites_core::{Task, Ctx};
/// # use nanites_core::combinators::{Refineable, RefinementStep};
/// # #[derive(Debug, Clone)] struct Improve;
/// # impl Task for Improve {
/// #     type Input = String; type Output = Result<String, String>;
/// #     type Error = std::convert::Infallible;
/// #     async fn run(&self, input: String, _: &Ctx) -> Result<Self::Output, Self::Error> { Ok(Ok(input)) }
/// # }
/// # impl Refineable for Improve {
/// #     type Final = String;
/// #     fn into_step(output: Self::Output) -> RefinementStep<Self::Final, Self::Input> {
/// #         RefinementStep::Done(output.unwrap())
/// #     }
/// # }
///
/// # async fn example() {
/// let runtime = Runtime::new();
/// let result = runtime
///     .run(Refine { inner: Improve, max_iterations: 5 }, "draft".to_string())
///     .await
///     .unwrap();
/// # }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Refine<T: Task> {
    /// The inner task, called on each iteration.
    pub inner: T,
    /// Maximum number of times the inner task may be invoked.
    ///
    /// If the task has not returned `Done` after this many iterations, the
    /// combinator returns an error.
    pub max_iterations: u32,
}

impl<T> Task for Refine<T>
where
    T: Refineable + fmt::Debug + Clone,
    T::Error: std::error::Error + Send + Sync + 'static,
    T::Input: Clone,
{
    type Input = T::Input;
    type Output = T::Final;
    type Error = RuntimeError;

    async fn run(&self, input: Self::Input, ctx: &Ctx) -> Result<Self::Output, Self::Error> {
        let mut current = input;

        for iteration in 0..self.max_iterations {
            let output = ctx.spawn(self.inner.clone(), current).await?;

            match T::into_step(output) {
                RefinementStep::Done(final_val) => return Ok(final_val),
                RefinementStep::Continue(next_input) => {
                    current = next_input;
                }
            }

            if ctx.is_cancelled() {
                return Err(RuntimeError::Cancelled);
            }

            let _ = iteration; // suppress unused warning on last iteration
        }

        Err(RuntimeError::TaskPanicked(format!(
            "Refine: max_iterations ({}) reached without convergence",
            self.max_iterations
        )))
    }
}

// ─── Retry ───────────────────────────────────────────────────────────────────

/// Re-run the inner task on error, up to `max_attempts` times.
///
/// The same input is cloned and passed to each attempt. On success the output
/// is returned immediately; earlier failures are discarded. If every attempt
/// fails, the error from the final attempt is returned.
///
/// `max_attempts = 1` means a single attempt with no retries (equivalent to
/// running `inner` directly). `max_attempts = 0` always returns an error
/// immediately (the loop body never executes).
///
/// # Example
///
/// ```no_run
/// use nanites_core::{Task, Ctx, Runtime, combinators::Retry};
///
/// #[derive(Debug, Clone)]
/// struct Flaky;
///
/// impl Task for Flaky {
///     type Input = ();
///     type Output = String;
///     type Error = std::convert::Infallible;
///
///     async fn run(&self, _: (), _ctx: &Ctx) -> Result<String, Self::Error> {
///         Ok("ok".to_string())
///     }
/// }
///
/// # async fn example() {
/// let runtime = nanites_core::Runtime::new();
/// let result = runtime
///     .run(Retry { inner: Flaky, max_attempts: 3 }, ())
///     .await
///     .unwrap();
/// # }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Retry<T> {
    /// The inner task to attempt.
    pub inner: T,
    /// Maximum number of attempts (including the first).
    ///
    /// Set to `1` for a single attempt; `3` for one attempt plus two retries.
    pub max_attempts: u32,
}

impl<T> Task for Retry<T>
where
    T: Task + fmt::Debug + Clone,
    T::Error: std::error::Error + Send + Sync + 'static,
    T::Input: Clone,
{
    type Input = T::Input;
    type Output = T::Output;
    type Error = RuntimeError;

    async fn run(&self, input: Self::Input, ctx: &Ctx) -> Result<Self::Output, Self::Error> {
        let mut last_error: Option<RuntimeError> = None;

        for attempt in 0..self.max_attempts {
            if ctx.is_cancelled() {
                return Err(RuntimeError::Cancelled);
            }

            match ctx.spawn(self.inner.clone(), input.clone()).await {
                Ok(output) => return Ok(output),
                Err(e) => {
                    last_error = Some(e);
                }
            }

            let _ = attempt; // suppress unused warning
        }

        Err(last_error.unwrap_or_else(|| {
            RuntimeError::TaskPanicked(
                "Retry: max_attempts is 0, no attempts were made".to_string(),
            )
        }))
    }
}
