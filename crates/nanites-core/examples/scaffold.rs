//! scaffold.rs — a scaffold that logs task names before they run.
//!
//! Also demonstrates a type-matching scaffold: inspects the incoming task's
//! type_name and replaces it with a wrapped version for a specific type.
//!
//! Run with: cargo run --example scaffold

use std::sync::{Arc, Mutex};

use nanites_core::{Ctx, DynTask, Runtime, Scaffold, Task, dyn_task::SharedDynTask};

/// A simple task we'll intercept with a scaffold.
#[derive(Debug, Clone)]
struct Greet {
    name: String,
}

impl Task for Greet {
    type Input = ();
    type Output = String;
    type Error = std::convert::Infallible;

    async fn run(&self, _input: (), _ctx: &Ctx) -> Result<Self::Output, Self::Error> {
        Ok(format!("Hello, {}!", self.name))
    }
}

/// A "wrapper" task that delegates to an inner DynTask but logs around it.
///
/// friction: wrapping a DynTask to add behaviour around it is awkward.
/// There's no `WrapTask` abstraction — you have to implement DynTask directly,
/// which requires boxing futures, dealing with AnyInput/AnyOutput, and
/// reimplementing clone_box. This is the main rough edge of the scaffold story:
/// the identity scaffold (just inspect + return) is easy, but actually wrapping
/// execution requires dropping down to DynTask manually.
///
/// For now we demonstrate the simpler case: a logging scaffold that records
/// which task types were spawned without wrapping their execution.

#[tokio::main]
async fn main() {
    // Collect task type names seen by the scaffold.
    let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    let seen_clone = Arc::clone(&seen);
    let logging_scaffold = Scaffold::new(move |task: &dyn DynTask| -> SharedDynTask {
        // Log the task type name before it runs.
        seen_clone
            .lock()
            .unwrap()
            .push(task.type_name().to_string());
        eprintln!("[scaffold] spawning: {}", task.type_name());
        // Return the task unchanged — identity.
        // friction: `Arc::from(task.clone_box())` is the only way to do this.
        // There's no `task.into_shared()` or similar shorthand on SharedDynTask.
        // The pattern is a bit surprising to discover: you have to know that
        // SharedDynTask = Arc<dyn DynTask> and that clone_box() gives you a Box.
        Arc::from(task.clone_box())
    });

    let runtime = Runtime::new().with_scaffold(logging_scaffold);

    // Spawn a couple of tasks. The scaffold fires for each.
    let h1 = runtime.spawn(
        Greet {
            name: "Alice".to_string(),
        },
        (),
    );
    let h2 = runtime.spawn(
        Greet {
            name: "Bob".to_string(),
        },
        (),
    );

    let r1 = h1.await.unwrap();
    let r2 = h2.await.unwrap();
    println!("{r1}");
    println!("{r2}");

    let seen = seen.lock().unwrap();
    println!("Scaffold saw {} task launches:", seen.len());
    for name in seen.iter() {
        println!("  {name}");
    }
    assert_eq!(seen.len(), 2);

    println!("scaffold: ok");
}
