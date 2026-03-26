# Use Case: Software Engineering Agent

The proof-of-concept that validates the thesis. A fully general coding agent built on nanites.

## Task Graph Shape

Dynamic decomposition tree. The graph is not known upfront — it grows as the agent discovers what needs to be done.

```
PlanTask(problem)
├── ReadFileTask("src/foo.rs")
├── SearchTask("pattern")
├── CompletionTask(model="sonnet", system="...")   ← decomposes further
│   ├── EditFileTask("src/foo.rs", patch)
│   ├── RunCommandTask("cargo test")
│   └── CompletionTask(model="haiku", system="...") ← leaf: simple decision
└── CompletionTask(...)
    └── ...
```

Leaves are tool calls (deterministic). Internal nodes are LLM decisions. The LLM decides the shape; the program executes it.

## Task Inventory

### IoTasks (need executors)
- `CompletionTask { model, system }` — LLM call, already in nanites-rig
- `ReadFileTask { path }` — filesystem read
- `WriteFileTask { path }` — filesystem write
- `RunCommandTask { command, args, cwd }` — shell execution
- `SearchTask { pattern, path }` — code search (normalize/grep)

### Pure Tasks (implement `run`)
- `PlanTask` — takes a problem description, uses `ctx.spawn` to launch subtasks based on LLM output
- `PatchApplyTask` — deterministic, applies a diff to text

## Executor Inventory

- `RigCompletionExecutor` (already exists) — model name → model instance
- `FilesystemExecutor` — handles ReadFile/WriteFile
- `ShellExecutor` — handles RunCommand (sandbox concerns: path allowlist, timeout)
- `SearchExecutor` — wraps normalize or ripgrep

## Frontier / Exec Graph Usage

The **frontier** is where human-in-the-loop happens. Before a `RunCommandTask` executes, a scaffold can pause and show the user what's about to run. The user can prune it, modify it, or let it proceed.

The **exec graph** provides the audit trail: what decisions were made, in what order, with what inputs. Essential for debugging a failed run.

## Pseudocode

```rust
#[derive(Serialize, Deserialize)]
struct DecomposeTask {
    model: String,
    problem: String,
}

impl Task for DecomposeTask {
    type Input = ();
    type Output = ();
    type Error = BoxError;

    async fn run(&self, _: (), ctx: &Ctx) -> Result<(), BoxError> {
        // Ask LLM to decompose the problem into subtasks
        let plan_handle = ctx.spawn(
            CompletionTask { model: self.model.clone(), system: Some(DECOMPOSE_SYSTEM.into()) },
            self.problem.clone(),
        );
        let plan: SubtaskList = parse_subtasks(plan_handle.await?)?;

        // Spawn subtasks (may themselves decompose further)
        let handles: Vec<_> = plan.subtasks.iter()
            .map(|s| ctx.spawn_dyn(s.into_task(), s.input()))
            .collect();

        // Await all — errors surface to parent
        for h in handles { h.await?; }
        Ok(())
    }
}
```

## Gaps in Current Substrate

1. **No tool plugin system** — no standard way for `ReadFileTask`, `WriteFileTask` etc. to be registered. Need a registry beyond the executor map, or the executor map is the registry (probably the latter).

2. **No pause/approval scaffold** — scaffolds exist but there's no built-in "pause and ask the user" mechanism. Needs an async scaffold that can block on user input.

3. **No model routing** — `CompletionTask { model: "sonnet" }` is static. Need a way to say "use the cheap model for this subtask" without hardcoding names. Could be a scaffold that rewrites model names based on task depth/complexity.

4. **No structured output** — LLM returns `String`. Parsing into `SubtaskList` is manual. Need typed completion that deserializes into a struct.

5. **No exec graph querying by type** — to show "what files were read during this run", need `exec_graph.nodes_of_type::<ReadFileTask>()`.
