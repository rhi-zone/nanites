//! nanites-core — serializable task substrate for flexible orchestration.
//!
//! The fundamental primitive is a **serializable task struct**, not a closure or agent.
//! Tasks are statically typed at definition; the runtime operates on type-erased
//! [`DynTask`] objects with runtime type checking at the boundary.
//!
//! # Two kinds of tasks
//!
//! **Pure tasks** implement [`Task::run`] for self-contained computation.
//! The runtime executes them directly — no registration needed.
//!
//! **I/O tasks** implement [`IoTask`] instead. They carry only configuration
//! (model name, endpoint URL, etc.) and are executed by a registered
//! [`TaskExecutor`]. This keeps task structs serializable even when the
//! resources they need (HTTP clients, model handles) are not.
//!
//! # Quick start — pure task
//!
//! ```no_run
//! use nanites_core::{Task, Ctx, Runtime};
//!
//! #[derive(Debug, Clone)]
//! struct Double;
//!
//! impl Task for Double {
//!     type Input = i64;
//!     type Output = i64;
//!     type Error = std::convert::Infallible;
//!
//!     async fn run(
//!         &self,
//!         input: Self::Input,
//!         _ctx: &Ctx,
//!     ) -> Result<Self::Output, Self::Error> {
//!         Ok(input * 2)
//!     }
//! }
//!
//! # async fn example() {
//! let runtime = Runtime::new();
//! let result = runtime.run(Double, 21i64).await.unwrap();
//! assert_eq!(result, 42);
//! # }
//! ```

pub mod cancellation;
pub mod ctx;
pub mod dyn_task;
pub mod error;
pub mod exec_graph;
pub mod executor;
pub mod frontier;
pub mod handle;
pub mod registry;
pub mod runtime;
pub mod scaffold;
pub mod task;

pub use cancellation::CancellationToken;
pub use ctx::Ctx;
pub use dyn_task::{AnyInput, AnyOutput, DynTask, ErasedIoTask, ErasedTask, IoTask, SharedDynTask};
pub use error::{BoxError, RuntimeError};
pub use exec_graph::{ExecGraph, ExecNode, TerminalState};
pub use executor::TaskExecutor;
pub use frontier::{Frontier, FrontierHandle, NodeId, TaskNode};
pub use handle::TaskHandle;
pub use registry::{RegistryError, SerializableTask, TaskRegistry, TaskSnapshot};
pub use runtime::Runtime;
pub use scaffold::Scaffold;
pub use task::Task;
