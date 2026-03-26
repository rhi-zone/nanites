//! scaffold.rs — a scaffold that logs task names before they run.
//!
//! A scaffold is a `Fn(&dyn DynTask) -> SharedDynTask` applied to every task
//! before it is spawned. Common uses: inject a different model for specific
//! task types, wrap tasks with logging/tracing, swap implementations for tests.
//!
//! This example shows the simplest scaffold pattern: inspect the task type name
//! and return the task unchanged (identity). The exec_graph records all spawned
//! tasks; the scaffold fires before the graph entry is written.
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

#[tokio::main]
async fn main() {
    // Collect task type names seen by the scaffold.
    let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    let seen_clone = Arc::clone(&seen);
    let logging_scaffold = Scaffold::new(move |task: &dyn DynTask| -> SharedDynTask {
        // The scaffold fires before the task is handed to tokio. Record the type.
        seen_clone
            .lock()
            .unwrap()
            .push(task.type_name().to_string());
        eprintln!("[scaffold] spawning: {}", task.type_name());
        // Return the task unchanged — identity scaffold.
        // SharedDynTask = Arc<dyn DynTask>; clone_box() gives a Box, Arc::from
        // converts it. There is no shorthand on DynTask itself.
        Arc::from(task.clone_box())
    });

    // Register the scaffold with the runtime using the builder pattern.
    let runtime = Runtime::new().with_scaffold(logging_scaffold);

    // Spawn a couple of tasks. The scaffold fires once per spawn.
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

    // The exec_graph is the authoritative audit log — all spawned tasks, even
    // after completion. It is separate from the frontier, which is empty now.
    assert_eq!(runtime.frontier().len(), 0);
    let graph_nodes = runtime.exec_graph().snapshot();
    println!("Exec graph nodes: {}", graph_nodes.len());
    assert_eq!(graph_nodes.len(), 2);

    println!("scaffold: ok");
}
