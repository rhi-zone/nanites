# Use Case: Interactive Tool

A creative or productivity tool where LLM assistance happens in real-time as the user works. Potential integration: Scribble.

## Core Insight

Each user action sees current document state — not accumulated history. The tool responds to what you're doing now. Every call is fresh. Cancellation is first-class: if the user types again, the previous call is irrelevant and must die.

## Task Graph Shape

Flat — each user action is one independent dispatch. No decomposition, no fan-out. Latency is everything.

```
UserActionEvent
└── CompletionTask(model="haiku", system=current_doc_state, prompt=user_action)
```

That's it. One call. Fast model. Fresh context.

## Task Inventory

### IoTasks
- `CompletionTask { model, system }` — haiku or equivalent, low latency
- `StreamingCompletionTask { model, system }` — preferred for interactive; shows tokens as they arrive

### Pure Tasks
- `BuildPromptTask` — constructs the prompt from current document state + action. Pure function.

## Cancellation

The critical feature. The user types character 3 of their next input while the model is still generating character 47 of a response. The old call must be cancelled immediately.

```rust
// Pseudocode: event loop
let mut current = None::<tokio::task::JoinHandle<_>>;

for event in user_events {
    // Cancel previous call
    if let Some(handle) = current.take() {
        handle.abort();
    }

    // Fresh context from current state
    let doc_state = editor.snapshot();
    let system = build_context(&doc_state);

    current = Some(tokio::spawn(async move {
        runtime.run(
            CompletionTask { model: "haiku".into(), system: Some(system) },
            event.to_prompt(),
        ).await
    }));
}
```

The `CancellationToken` in `Ctx` handles graceful cancellation within the task graph. `handle.abort()` handles the tokio-level kill.

## What "Flexible" Means Here

- Swap model without restarting — change `CompletionTask.model` per call
- Adjust context window — `BuildPromptTask` can include more or less document state based on latency budget
- Plugin different context sources — RAG over the document, external knowledge base, style guides

## Gaps in Current Substrate

1. **No streaming** — the biggest gap. Interactive tools need streaming; waiting for the full completion before showing anything breaks the UX. `StreamingCompletionTask` needs to be built, with a channel-based output type.

2. **Latency instrumentation** — no built-in timing on tasks. For an interactive tool you want to know p50/p95 latency per task type to tune model selection and context size.

3. **No debounce/throttle** — the substrate dispatches immediately. Debounce (wait N ms before dispatching) belongs in the consumer, not the substrate. Fine as-is, just worth noting it's the consumer's responsibility.

4. **Runtime::run is heavyweight for interactive use** — each call goes through the full spawn/frontier/exec-graph machinery. For a tool making 10+ calls per second, this overhead may matter. Needs benchmarking.
