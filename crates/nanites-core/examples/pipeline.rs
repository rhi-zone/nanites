//! pipeline.rs — a chain of tasks where one task's output feeds into the next.
//!
//! Demonstrates dynamic dependency edges via TaskHandle: the parent spawns
//! task A, passes it along to task B (which awaits it), creating a
//! happens-before relationship at runtime.
//!
//! Run with: cargo run --example pipeline

// friction: Task::Error must satisfy `std::error::Error + Send + Sync + 'static`
// (required by ctx.spawn). That means `Box<dyn Error + Send + Sync>` doesn't
// work directly as T::Error — `dyn Error` is unsized and `Box<dyn Error>` only
// implements Error via a blanket impl that requires `Box<E: Error>`. The bound
// `T::Error: Error` fails for the boxed-dyn form because the *trait object
// itself* isn't Sized. This rules out the idiomatic "just use anyhow::Error"
// approach. Callers who want open-ended error composition need a concrete
// wrapper type. A newtype around String is the simplest escape hatch.

use nanites_core::{Ctx, Runtime, Task};

/// Minimal concrete error type for tasks that produce string error messages.
/// friction: every example that needs a fallible task has to define or import
/// one of these. A `nanites_core::StringError` (or re-export of a common type)
/// would reduce boilerplate considerably.
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
/// This is the interesting part: each stage spawns the next, passing the
/// previous result. The frontier records the dependency chain automatically
/// because each spawn happens inside a running task.
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
        // friction: the input has to be moved into spawn here — we can't
        // re-use it for anything else after this point. That's expected
        // (single ownership) but it means if you want to log the raw input
        // you have to clone before spawning. Minor but appears repeatedly.
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
        // friction: we have to clone `self.suffix` here to move it into the
        // AppendSuffix struct. The task config fields are cloned at construction
        // of the child task, which is expected, but it becomes slightly verbose
        // when the orchestrator owns configuration that multiple children need.
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

    // The frontier captures the full chain: Pipeline → ParseInt → FormatPadded → AppendSuffix.
    // Parent-child relationships are recorded automatically.
    let nodes = runtime.frontier().snapshot();
    println!("Frontier ({} nodes):", nodes.len());
    for node in &nodes {
        println!(
            "  [{:?}] {} (parent={:?})",
            node.status,
            node.task.type_name(),
            node.parent,
        );
    }

    println!("pipeline: ok");
}
