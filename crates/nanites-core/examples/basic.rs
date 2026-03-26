//! basic.rs — sanity check: a task that doubles a number.
//!
//! # Two execution paths
//!
//! **Pure tasks** (like `Double` below) implement `Task::run`. The runtime
//! calls `run` directly — no registration needed. This is the right choice for
//! self-contained computation that needs no external resources.
//!
//! **I/O tasks** implement `IoTask` (no `run` method) and carry only
//! configuration (e.g. model name, endpoint URL). A `TaskExecutor` registered
//! with the runtime handles their execution and owns the resource handles
//! (HTTP clients, model objects, connection pools). This keeps the task struct
//! serializable even when the resources it needs are not.
//!
//! ```no_run
//! // Pure task — self-contained, no registration:
//! // runtime.run(Double, 21i64).await
//!
//! // I/O task — register an executor first:
//! // let runtime = Runtime::new().with_executor(MyHttpExecutor::new(client));
//! // runtime.run_dyn(erase_io(FetchUrl { url: "..." }), Box::new(())).await
//! ```
//!
//! Run with: cargo run --example basic

use nanites_core::{Ctx, Runtime, Task};

#[derive(Debug, Clone)]
struct Double;

impl Task for Double {
    type Input = i64;
    type Output = i64;
    // Infallible is the right choice for a task that literally cannot fail.
    type Error = std::convert::Infallible;

    async fn run(&self, input: Self::Input, _ctx: &Ctx) -> Result<Self::Output, Self::Error> {
        Ok(input * 2)
    }
}

#[tokio::main]
async fn main() {
    let runtime = Runtime::new();

    let result = runtime.run(Double, 21i64).await.unwrap();
    println!("Double(21) = {result}");
    assert_eq!(result, 42);

    // The frontier tracks only *pending* tasks. Completed tasks are removed
    // automatically via remove_terminal, so the frontier is empty after a run.
    println!("Frontier nodes after run: {}", runtime.frontier().len());
    assert_eq!(runtime.frontier().len(), 0);

    // The exec_graph is the monotonically-growing audit log — it keeps every
    // task that was ever spawned, including completed ones.
    let nodes = runtime.exec_graph().snapshot();
    println!("Exec graph nodes: {}", nodes.len());
    assert_eq!(nodes.len(), 1);
    println!(
        "  task_type={}, terminal={:?}",
        nodes[0].task_type, nodes[0].terminal
    );

    println!("basic: ok");
}
