//! The `Task` trait — the fundamental primitive.
//!
//! A task is a **serializable struct** that describes a unit of work.
//! It carries its configuration (e.g. target language, model name, prompt template)
//! but not its inputs — inputs arrive at execution time via [`Task::run`].
//!
//! # Design
//!
//! Tasks are statically typed at definition. Type erasure happens at the
//! runtime boundary via [`crate::DynTask`] / [`crate::ErasedTask`].
//! Scaffolds and the frontier operate on erased tasks; callers get back
//! typed [`crate::TaskHandle<O>`] futures.
//!
//! # Example
//!
//! ```no_run
//! use nanites_core::{Task, Ctx};
//!
//! #[derive(Debug, Clone)]
//! struct TranslateText {
//!     target_language: String,
//! }
//!
//! impl Task for TranslateText {
//!     type Input = String;
//!     type Output = String;
//!     type Error = Box<dyn std::error::Error + Send + Sync>;
//!
//!     async fn run(
//!         &self,
//!         input: Self::Input,
//!         _ctx: &Ctx,
//!     ) -> Result<Self::Output, Self::Error> {
//!         // ... call an LLM, a deterministic function, etc.
//!         Ok(format!("[{}] {}", self.target_language, input))
//!     }
//! }
//! ```

use std::future::Future;

use crate::ctx::Ctx;

/// A serializable, statically-typed unit of work.
///
/// Implement this for every distinct operation in your orchestration graph.
/// The runtime wraps implementors in [`crate::ErasedTask`] for dynamic dispatch.
pub trait Task: Send + Sync + 'static {
    /// The input value this task consumes.
    type Input: Send + 'static;
    /// The value produced on success.
    type Output: Send + 'static;
    /// The error produced on failure.
    type Error: Send + 'static;

    /// Execute the task.
    ///
    /// `ctx` provides cancellation and the ability to spawn child tasks.
    /// Error propagation to the parent is the **caller's** responsibility —
    /// the runtime has no built-in retry or recovery policy.
    fn run(
        &self,
        input: Self::Input,
        ctx: &Ctx,
    ) -> impl Future<Output = Result<Self::Output, Self::Error>> + Send;
}
