//! decompose.rs — a task that decomposes its input into a dynamic number of subtasks.
//!
//! This is the core use case for nanites: the input (a list of items) determines
//! how many subtasks are spawned at runtime. The structure of the task graph is
//! not known statically — it emerges from the data.
//!
//! Scenario: process a batch of text lines. Each line is processed independently
//! (trimmed and uppercased). The orchestrator collects the results in order.
//!
//! Run with: cargo run --example decompose

use nanites_core::{Ctx, Runtime, Task};

#[derive(Debug)]
struct StringError(String);

impl std::fmt::Display for StringError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for StringError {}

impl From<nanites_core::RuntimeError> for StringError {
    fn from(e: nanites_core::RuntimeError) -> Self {
        StringError(e.to_string())
    }
}

/// A leaf task: process a single text line.
#[derive(Debug, Clone)]
struct ProcessLine;

impl Task for ProcessLine {
    type Input = String;
    type Output = String;
    type Error = std::convert::Infallible;

    async fn run(&self, input: Self::Input, _ctx: &Ctx) -> Result<Self::Output, Self::Error> {
        // Simple transformation: trim whitespace and uppercase.
        Ok(input.trim().to_uppercase())
    }
}

/// Orchestrator: receives a batch of lines, decomposes into one subtask per line.
///
/// The number of subtasks is entirely determined by the input — neither the
/// task struct nor the runtime knows it ahead of time.
#[derive(Debug, Clone)]
struct ProcessBatch;

impl Task for ProcessBatch {
    type Input = Vec<String>;
    type Output = Vec<String>;
    type Error = StringError;

    async fn run(&self, input: Self::Input, ctx: &Ctx) -> Result<Self::Output, Self::Error> {
        // ctx.spawn_all fans out over a collection of inputs, spawning one
        // child task per item. All children are registered in the frontier
        // before any of them begin executing.
        let handles = ctx.spawn_all(ProcessLine, input);

        // Await handles in order to preserve input ordering in the output.
        // The underlying tokio tasks ran concurrently — we are just collecting
        // results sequentially here.
        let mut results = Vec::with_capacity(handles.len());
        for handle in handles {
            results.push(handle.await?);
        }

        Ok(results)
    }
}

#[tokio::main]
async fn main() {
    let runtime = Runtime::new();

    let lines = vec![
        "  hello world  ".to_string(),
        "  foo bar  ".to_string(),
        "  the quick brown fox  ".to_string(),
    ];
    let n = lines.len();

    let results = runtime.run(ProcessBatch, lines).await.unwrap();

    println!("Processed {n} lines:");
    for (i, result) in results.iter().enumerate() {
        println!("  [{i}] {result}");
    }

    assert_eq!(results[0], "HELLO WORLD");
    assert_eq!(results[1], "FOO BAR");
    assert_eq!(results[2], "THE QUICK BROWN FOX");

    // The frontier tracks only pending tasks and is empty after completion.
    // Use exec_graph for the full lineage audit: 1 ProcessBatch + n ProcessLine.
    let graph_nodes = runtime.exec_graph().snapshot();
    println!(
        "Exec graph nodes: {} (1 parent + {n} children)",
        graph_nodes.len()
    );
    assert_eq!(graph_nodes.len(), n + 1);

    // exec_graph.get_children(id) lets you query parent→child relationships
    // directly without scanning the full snapshot.
    let parent_node = graph_nodes
        .iter()
        .find(|n| n.parent.is_none())
        .expect("root node");
    let children = runtime.exec_graph().get_children(parent_node.id);
    println!(
        "Root node {} spawned {} children",
        parent_node.task_type,
        children.len()
    );
    assert_eq!(children.len(), n);

    // Demonstrate with a larger dynamic batch to stress the fan-out.
    let large_batch: Vec<String> = (0..20).map(|i| format!("item {i}")).collect();
    let large_results = runtime.run(ProcessBatch, large_batch).await.unwrap();
    assert_eq!(large_results.len(), 20);
    assert_eq!(large_results[0], "ITEM 0");
    assert_eq!(large_results[19], "ITEM 19");
    println!("Large batch (20 items): ok");

    println!("decompose: ok");
}
