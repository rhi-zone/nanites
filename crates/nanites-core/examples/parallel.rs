//! parallel.rs — a parent task that spawns multiple child tasks concurrently.
//!
//! Demonstrates: fan-out with ctx.spawn_all, collecting handles, awaiting all
//! results, and inspecting the exec_graph for the lineage record.
//!
//! Run with: cargo run --example parallel

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

/// Simulate an expensive computation — multiplies the input by a factor.
#[derive(Debug, Clone)]
struct Multiply {
    factor: i64,
}

impl Task for Multiply {
    type Input = i64;
    type Output = i64;
    type Error = std::convert::Infallible;

    async fn run(&self, input: Self::Input, _ctx: &Ctx) -> Result<Self::Output, Self::Error> {
        // In a real scenario this would be an async I/O call.
        Ok(input * self.factor)
    }
}

/// A parent task that fans out to N Multiply children and sums their results.
#[derive(Debug, Clone)]
struct FanOutSum {
    factors: Vec<i64>,
}

impl Task for FanOutSum {
    type Input = i64;
    type Output = i64;
    type Error = StringError;

    async fn run(&self, input: Self::Input, ctx: &Ctx) -> Result<Self::Output, Self::Error> {
        // ctx.spawn_all spawns one Multiply child per factor, all concurrently.
        // Each child gets a clone of the task struct and one input value.
        // All children are registered in the frontier before any begin executing.
        let handles: Vec<_> = self
            .factors
            .iter()
            .map(|&factor| ctx.spawn(Multiply { factor }, input))
            .collect();

        // Await handles in order. The children ran concurrently — we collect
        // results sequentially. This is correct and idiomatic; for unordered
        // collection futures::future::join_all or try_join_all also work.
        let mut sum = 0i64;
        for handle in handles {
            let result = handle.await?;
            sum += result;
        }

        Ok(sum)
    }
}

#[tokio::main]
async fn main() {
    let runtime = Runtime::new();

    // Sum of: 10*1, 10*2, 10*3, 10*4, 10*5 = 10+20+30+40+50 = 150
    let factors = vec![1, 2, 3, 4, 5];
    let expected: i64 = factors.iter().map(|&f| 10 * f).sum();

    let result = runtime.run(FanOutSum { factors }, 10i64).await.unwrap();
    println!("FanOutSum(10, factors=[1..5]) = {result}  (expected {expected})");
    assert_eq!(result, expected);

    // The frontier is empty after completion — completed nodes are removed.
    println!("Frontier nodes after run: {}", runtime.frontier().len());
    assert_eq!(runtime.frontier().len(), 0);

    // The exec_graph records the full lineage: 1 FanOutSum + 5 Multiply nodes.
    let graph = runtime.exec_graph();
    println!("Exec graph nodes: {}", graph.len());
    assert_eq!(graph.len(), 6); // 1 parent + 5 children

    let completed = graph.completed_nodes();
    println!("All {} nodes completed successfully.", completed.len());
    assert_eq!(completed.len(), 6);

    println!("parallel: ok");
}
