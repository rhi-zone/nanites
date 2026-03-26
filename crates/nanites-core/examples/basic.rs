//! basic.rs — sanity check: a task that doubles a number.
//!
//! Run with: cargo run --example basic

use nanites_core::{Ctx, Runtime, Task};

#[derive(Debug, Clone)]
struct Double;

impl Task for Double {
    type Input = i64;
    type Output = i64;
    // friction: Infallible doesn't display a great error message when something
    // goes wrong at the RuntimeError level, but it's the right choice for a
    // task that literally cannot fail.
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

    // Also check the frontier recorded the task.
    // friction: frontier is not automatically cleaned up after task completion,
    // so we see stale Completed nodes here. Fine for observability but callers
    // need to be aware that len() != in-flight tasks.
    println!("Frontier nodes after run: {}", runtime.frontier().len());

    println!("basic: ok");
}
