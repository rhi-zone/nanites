//! pipeline.rs — a chain of tasks where one task's output feeds into the next.
//!
//! Demonstrates dynamic dependency edges via TaskHandle: the parent spawns
//! task A, passes it along to task B (which awaits it), creating a
//! happens-before relationship at runtime.
//!
//! The exec_graph records the full lineage — use it (not the frontier) for
//! auditing after a run, since the frontier only tracks pending tasks and is
//! empty once all tasks complete.
//!
//! Run with: cargo run --example pipeline

// Task::Error must satisfy `std::error::Error + Send + Sync + 'static`
// (required by ctx.spawn). `Box<dyn Error + Send + Sync>` doesn't work
// directly as T::Error — the blanket impl doesn't apply to unsized dyn types.
// A concrete newtype around String is the simplest escape hatch.

use nanites_core::{Ctx, Runtime, Task, exec_graph::TerminalState};

/// Minimal concrete error type for tasks that produce string error messages.
#[derive(Debug)]
struct StringError(String);

impl std::fmt::Display for StringError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for StringError {}

impl From<std::num::ParseIntError> for StringError {
    fn from(e: std::num::ParseIntError) -> Self {
        StringError(e.to_string())
    }
}

impl From<nanites_core::RuntimeError> for StringError {
    fn from(e: nanites_core::RuntimeError) -> Self {
        StringError(e.to_string())
    }
}

/// Stage 1: parse a string into an integer.
#[derive(Debug, Clone)]
struct ParseInt;

impl Task for ParseInt {
    type Input = String;
    type Output = i64;
    type Error = StringError;

    async fn run(&self, input: Self::Input, _ctx: &Ctx) -> Result<Self::Output, Self::Error> {
        let n = input.trim().parse::<i64>()?;
        Ok(n)
    }
}

/// Stage 2: format an integer as a padded decimal string.
#[derive(Debug, Clone)]
struct FormatPadded {
    width: usize,
}

impl Task for FormatPadded {
    type Input = i64;
    type Output = String;
    type Error = std::convert::Infallible;

    async fn run(&self, input: Self::Input, _ctx: &Ctx) -> Result<Self::Output, Self::Error> {
        Ok(format!("{:0>width$}", input, width = self.width))
    }
}

/// Stage 3: append a suffix to a string.
#[derive(Debug, Clone)]
struct AppendSuffix {
    suffix: String,
}

impl Task for AppendSuffix {
    type Input = String;
    type Output = String;
    type Error = std::convert::Infallible;

    async fn run(&self, input: Self::Input, _ctx: &Ctx) -> Result<Self::Output, Self::Error> {
        Ok(format!("{}{}", input, self.suffix))
    }
}

/// Orchestrator that wires the three stages together.
///
/// Each stage spawns the next and passes the previous result. The exec_graph
/// records the dependency chain automatically because each spawn happens inside
/// a running task.
#[derive(Debug, Clone)]
struct Pipeline {
    pad_width: usize,
    suffix: String,
}

impl Task for Pipeline {
    type Input = String;
    type Output = String;
    type Error = StringError;

    async fn run(&self, input: Self::Input, ctx: &Ctx) -> Result<Self::Output, Self::Error> {
        // Stage 1: parse
        let parsed = ctx.spawn(ParseInt, input).await?;

        // Stage 2: format
        let formatted = ctx
            .spawn(
                FormatPadded {
                    width: self.pad_width,
                },
                parsed,
            )
            .await?;

        // Stage 3: append suffix
        let result = ctx
            .spawn(
                AppendSuffix {
                    suffix: self.suffix.clone(),
                },
                formatted,
            )
            .await?;

        Ok(result)
    }
}

#[tokio::main]
async fn main() {
    let runtime = Runtime::new();

    let result = runtime
        .run(
            Pipeline {
                pad_width: 8,
                suffix: "_done".to_string(),
            },
            "42".to_string(),
        )
        .await
        .unwrap();

    println!("Pipeline(\"42\") = {result:?}");
    assert_eq!(result, "00000042_done");

    // The frontier tracks only *pending* tasks — it is empty after the run.
    // Use exec_graph() for the full lineage audit.
    assert_eq!(runtime.frontier().len(), 0);

    // exec_graph captures the full chain: Pipeline → ParseInt → FormatPadded → AppendSuffix.
    // Parent–child relationships are recorded automatically by the runtime.
    let mut nodes = runtime.exec_graph().snapshot();
    // Sort by id for deterministic output.
    nodes.sort_by_key(|n| n.id);

    println!("Exec graph ({} nodes):", nodes.len());
    for node in &nodes {
        println!(
            "  [{}] {} (parent={:?}, terminal={:?})",
            node.id,
            node.task_type,
            node.parent,
            node.terminal
                .as_ref()
                .map(|t| matches!(t, TerminalState::Completed { .. }))
                .map(|ok| if ok { "Completed" } else { "other" })
                .unwrap_or("None"),
        );
    }

    assert_eq!(nodes.len(), 4); // Pipeline + 3 stages
    assert!(
        nodes
            .iter()
            .all(|n| matches!(n.terminal, Some(TerminalState::Completed { .. })))
    );

    println!("pipeline: ok");
}
