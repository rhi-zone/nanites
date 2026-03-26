//! parallel.rs — a parent task that spawns multiple child tasks concurrently.
//!
//! Demonstrates: collecting handles, awaiting all results.
//!
//! Run with: cargo run --example parallel

// friction: Task::Error: std::error::Error + Send + Sync + 'static is required
// by ctx.spawn. Box<dyn Error + Send + Sync> doesn't satisfy this bound because
// `dyn Error` is unsized, so the Box<dyn Error> blanket impl doesn't apply.
// See pipeline.rs for a detailed explanation. We use a StringError newtype here.

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
        // Spawn all children — they all start immediately (tokio tasks).
        // friction: we have to collect into a Vec first because TaskHandle is
        // not Clone and futures::future::join_all wants ownership. That's
        // fine, but it means we can't easily do "spawn and forget" while also
        // collecting results. The pattern is: collect handles first, await all.
        let handles: Vec<_> = self
            .factors
            .iter()
            .map(|&factor| ctx.spawn(Multiply { factor }, input))
            .collect();

        // Await all handles. In tokio this gives us actual concurrency since each
        // handle is a oneshot receiver, not a blocking wait.
        // friction: there's no built-in `ctx.join_all` or similar combinator —
        // callers must reach for futures::future::try_join_all themselves.
        // Not a blocker, but worth noting as a common pattern.
        let mut sum = 0i64;
        for handle in handles {
            // Each await here is sequential in the loop body, but the underlying
            // tokio tasks are already running in parallel — we're just collecting
            // results. This is correct but not as ergonomic as try_join_all.
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

    println!(
        "Frontier nodes after run: {}",
        runtime.frontier().len() // Note: includes the parent + 5 children, all in Completed state.
    );

    println!("parallel: ok");
}
