//! nanites-core — serializable task substrate for flexible orchestration.
//!
//! The fundamental primitive is a **serializable task struct**, not a closure or agent.
//! Tasks are statically typed at definition; the runtime operates on type-erased
//! [`DynTask`] objects with runtime type checking at the boundary.
//!
//! # Quick start
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
pub mod frontier;
pub mod handle;
pub mod runtime;
pub mod scaffold;
pub mod task;

pub use cancellation::CancellationToken;
pub use ctx::Ctx;
pub use dyn_task::{AnyInput, AnyOutput, DynTask, ErasedTask, SharedDynTask};
pub use error::RuntimeError;
pub use frontier::{Frontier, FrontierHandle, NodeId, TaskNode};
pub use handle::TaskHandle;
pub use runtime::Runtime;
pub use scaffold::Scaffold;
pub use task::Task;
