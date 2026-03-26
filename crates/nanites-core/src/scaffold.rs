//! Scaffolds — transform a pending task before it executes.
//!
//! A scaffold is `Fn(&dyn DynTask) -> SharedDynTask`. It inspects an incoming
//! task and returns it (possibly transformed). Returning the input unchanged is
//! the no-op / identity case — scaffolds always return a task, never `None`.
//!
//! Scaffolds are applied in order, innermost-first, before the task is spawned
//! on tokio. Common uses:
//! - Inject a different model for specific task types
//! - Wrap tasks with retry/logging/tracing logic
//! - Swap implementations based on environment (test vs. prod)
//!
//! # Example
//!
//! ```no_run
//! use nanites_core::{Scaffold, DynTask, dyn_task::SharedDynTask};
//! use std::sync::Arc;
//!
//! // A scaffold that logs every task launch.
//! let logging_scaffold = Scaffold::new(|task: &dyn DynTask| -> SharedDynTask {
//!     eprintln!("[scaffold] spawning: {}", task.type_name());
//!     // Return the task unchanged (identity).
//!     Arc::from(task.clone_box())
//! });
//! ```

use std::sync::Arc;

use crate::dyn_task::{DynTask, SharedDynTask};

/// The function signature for a scaffold.
type ScaffoldFn = dyn Fn(&dyn DynTask) -> SharedDynTask + Send + Sync;

/// A scaffold function: inspect a pending task and return it transformed.
///
/// `Scaffold` is cheaply cloneable — it holds an `Arc<ScaffoldFn>`.
#[derive(Clone)]
pub struct Scaffold {
    f: Arc<ScaffoldFn>,
}

impl Scaffold {
    /// Create a scaffold from a closure or function.
    pub fn new<F>(f: F) -> Self
    where
        F: Fn(&dyn DynTask) -> SharedDynTask + Send + Sync + 'static,
    {
        Scaffold { f: Arc::new(f) }
    }

    /// Apply this scaffold to a task, returning the (possibly transformed) task.
    pub fn apply(&self, task: SharedDynTask) -> SharedDynTask {
        (self.f)(task.as_ref())
    }

    /// The identity scaffold — returns every task unchanged.
    pub fn identity() -> Self {
        Self::new(|task| Arc::from(task.clone_box()))
    }
}

impl std::fmt::Debug for Scaffold {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Scaffold(<fn>)")
    }
}
