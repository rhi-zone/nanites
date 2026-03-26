//! io_task.rs — contrast: a pure Task vs an IoTask dispatched by a TaskExecutor.
//!
//! # The two execution paths
//!
//! **Pure Task** — implements `Task::run`. The runtime calls `run` directly.
//! The task struct can own whatever it needs (since it only lives in memory).
//! Use this for deterministic, self-contained computation.
//!
//! **IoTask** — implements `IoTask` instead of `Task`. No `run` method.
//! The struct carries only *configuration* (a model name, an endpoint URL, …),
//! not the actual resource handle. A `TaskExecutor` registered with the runtime
//! owns the resource and performs the real work. This keeps the task struct
//! serializable: configuration is data, resource handles are not.
//!
//! # Why the split matters
//!
//! When you replay tasks from a serialized execution graph, IoTask structs
//! round-trip cleanly through JSON — they are just `{ "model": "gpt-4o" }`.
//! Pure tasks with embedded runtime state (e.g. a live HTTP client) could not
//! be serialized at all. Keeping configuration and execution separate is what
//! makes the task graph auditable and replayable.
//!
//! Run with: cargo run --example io_task

use std::any::Any;
use std::pin::Pin;

use nanites_core::dyn_task::{AnyInput, AnyOutput, IoTask, erase_io};
use nanites_core::error::BoxError;
use nanites_core::executor::TaskExecutor;
use nanites_core::{Ctx, Runtime, Task};

// ─── Pure Task ────────────────────────────────────────────────────────────────

/// A pure task: self-contained computation, no external resources.
///
/// The runtime calls `Task::run` directly — no executor registration needed.
#[derive(Debug, Clone)]
struct Double;

impl Task for Double {
    type Input = i64;
    type Output = i64;
    type Error = std::convert::Infallible;

    async fn run(&self, input: Self::Input, _ctx: &Ctx) -> Result<Self::Output, Self::Error> {
        // Pure computation — no I/O, no resource handles.
        Ok(input * 2)
    }
}

// ─── IoTask ───────────────────────────────────────────────────────────────────

/// An I/O task: pure configuration struct, no `run` method.
///
/// This struct is serializable — it carries only the data needed to describe
/// the operation, not the resource handle that performs it. Wrap it with
/// `erase_io(task)` before passing to the runtime, and register a
/// `FakeHttpExecutor` (below) to provide the actual execution.
#[derive(Debug, Clone)]
struct FetchText {
    /// The URL to fetch — pure config, no connection object.
    url: String,
}

// IoTask: no run() — just Input and Output type aliases.
impl IoTask for FetchText {
    type Input = (); // no additional input needed beyond the URL in the struct
    type Output = String; // the fetched text
}

// ─── TaskExecutor ─────────────────────────────────────────────────────────────

/// The executor that handles FetchText tasks.
///
/// This is where the actual resource lives — in a real application this would
/// hold an `Arc<reqwest::Client>` or similar. Here we fake the HTTP call.
struct FakeHttpExecutor {
    /// Simulated response body (stands in for a real HTTP client).
    canned_response: String,
}

impl TaskExecutor for FakeHttpExecutor {
    /// The runtime uses this name to look up the executor for a given task.
    /// Must match `std::any::type_name::<FetchText>()`.
    fn task_type_name(&self) -> &'static str {
        std::any::type_name::<FetchText>()
    }

    fn execute_erased<'a>(
        &'a self,
        task_data: &'a dyn Any,
        input: AnyInput,
        _ctx: &'a Ctx,
    ) -> Pin<Box<dyn futures::Future<Output = Result<AnyOutput, BoxError>> + Send + 'a>> {
        // Downcast task_data to access the configuration fields.
        let task = task_data
            .downcast_ref::<FetchText>()
            .expect("executor received wrong task type");

        // Downcast input — FetchText::Input = ().
        let _: () = *input.downcast::<()>().expect("input is not ()");

        // Use the URL from the task config to "fetch" content.
        let response = format!("Response from {}: {}", task.url, self.canned_response);

        Box::pin(async move { Ok(Box::new(response) as AnyOutput) })
    }
}

// ─── Main ─────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    // Build the runtime and register the executor for FetchText tasks.
    // Pure tasks (like Double) do not need any registration.
    let runtime = Runtime::new().with_executor(FakeHttpExecutor {
        canned_response: "hello from the server".to_string(),
    });

    // --- Pure task path ---
    // The runtime calls Double::run directly.
    let doubled = runtime.run(Double, 21i64).await.unwrap();
    println!("Double(21) = {doubled}");
    assert_eq!(doubled, 42);

    // --- IoTask path ---
    // Wrap FetchText in erase_io() — this produces a SharedDynTask backed by
    // ErasedIoTask<FetchText>. ErasedIoTask::run_erased returns NoExecutor;
    // the runtime detects the registered FakeHttpExecutor and dispatches there.
    let task = FetchText {
        url: "https://example.com/data".to_string(),
    };
    let output = runtime.run_dyn(erase_io(task), Box::new(())).await.unwrap();

    // Downcast the AnyOutput back to String.
    let text = *output.downcast::<String>().unwrap();
    println!("FetchText output: {text:?}");
    assert!(text.contains("hello from the server"));
    assert!(text.contains("example.com"));

    // The exec_graph recorded both tasks.
    let graph = runtime.exec_graph();
    println!("Exec graph nodes: {}", graph.len());
    assert_eq!(graph.len(), 2);

    let completed = graph.completed_nodes();
    println!("Completed: {}", completed.len());
    assert_eq!(completed.len(), 2);

    // Inspect the task types recorded — shows the full type names.
    let mut task_types: Vec<_> = completed.iter().map(|n| n.task_type.as_str()).collect();
    task_types.sort_unstable();
    for name in &task_types {
        println!("  {name}");
    }

    println!("io_task: ok");
}
