# Design Decisions

Decisions made during design/development, with the reasoning that led there. Prevents relitigating settled questions.

---

## Substrate

### Tasks are pure data, not closures

**Decision:** Tasks are serializable structs. `run()` is a method on the struct, not a stored closure.

**Why:** Closures aren't serializable. Serialization is required for caching, checkpoint/restore, exec graph audit trails, and pause/resume. The "ceremony" of writing a struct is not a cost — it's the design. The struct IS the task.

---

### Two task kinds: `Task` and `IoTask`

**Decision:** Pure computation tasks implement `Task` + `run()` and get a blanket `Execute` impl. Tasks needing external resources (model clients, DB connections, HTTP) implement `IoTask` (marker, no `run()`), and a `TaskExecutor` is registered on the Runtime to handle them.

**Why:** Keeps task structs as pure data throughout. Resource handles can't be serialized; injecting them via an executor keeps the task/resource boundary clean. The runtime is the resource boundary.

---

### Control flow is the graph

**Decision:** No explicit `Graph` struct with `add_node`/`connect` API. The task graph is constructed dynamically via `ctx.spawn`. Parent-child relationships are recorded automatically.

**Why:** Graphs known upfront are either trivial or fictional. Real decomposition is dynamic — you don't know what subtasks exist until you've done enough work to discover them. Explicit graph construction frameworks force you to lie about what you know upfront.

---

### Frontier vs exec graph are separate structures

**Decision:** `Frontier` = pending tasks only, shrinks as tasks complete. `ExecGraph` = monotonically growing lineage/audit record.

**Why:** They serve different purposes with different lifecycles. Conflating them means either the frontier never shrinks (memory leak) or the audit trail has holes. A graph that doesn't record lineage isn't a graph.

---

### Scaffolds are `Fn(&DynTask) -> DynTask`, always return a task

**Decision:** Scaffolds always return a task — either transformed or unchanged. No `Option`. Identity if no-op.

**Why:** `Option` implies "don't run this task" — that's pruning, which is a frontier manipulation, not a scaffold. Scaffolds shape tasks before execution; frontier manipulation controls which tasks execute. Separate concerns.

---

### `Ctx` carries only runtime concerns

**Decision:** `Ctx` carries frontier handle, cancellation token, executor map. Nothing LLM-specific, nothing domain-specific.

**Why:** LLM-specific context (model names, budgets) belongs in task data or executor configuration. Putting it in `Ctx` would make the runtime LLM-aware, which contradicts the thesis that LLM calls are just one node type among many.

---

### Registry-based type erasure (unshape pattern)

**Decision:** `SerializableTask` supertrait with `type_name() + params()`. `TaskRegistry` maps type name strings to factory closures. Same pattern as unshape's `NodeRegistry`.

**Why:** Nesting works — a wrapper task serializes its inner task recursively via the registry. String type names enable late binding and plugin architectures. Proven in unshape.

---

### `NoCache` is the default

**Decision:** Caching is opt-in. `NoCache` by default, zero overhead.

**Why:** Not all tasks are pure functions of their inputs. Caching a task with side effects or non-deterministic output is wrong. The user must opt in by choosing a cache implementation.

---

## Combinators

### Map, Refine, Retry are wrapper structs, not substrate primitives

**Decision:** `Map<T>`, `Refine<T>`, `Retry<T>` are task combinators — serializable structs wrapping another task. Not new runtime primitives.

**Why:** They're expressible as tasks using existing substrate (`ctx.spawn_all`, recursive `ctx.spawn`). Adding runtime primitives for them would be premature abstraction. A combinator layer on top is the right level.

---

### Iterative refinement is recursive ctx.spawn

**Decision:** A refining task spawns itself with the updated input via `ctx.spawn`. No special "loop" primitive in the runtime.

**Why:** It's just a task that produces `Either<Final, NextInput>`. The runtime doesn't need to understand loops — the task graph expresses the loop as a chain of spawns. `Refine<T>` wraps this pattern.

---

## Streaming

### Streaming is not a graph primitive

**Decision:** No `StreamingTask` trait, no stream output type in the substrate.

**Why:** Streaming has two consumer patterns:
1. **Buffered** — wait for completion, collect into `Vec<Chunk>`. This is just `T -> Vec<Chunk>`, no special case.
2. **Per-chunk processing** — spawn a child task per chunk as it arrives. This is `Map` over an implicit iterator, already expressible.

The progressive UI rendering case (show tokens as they arrive) is a side channel from executor to UI — a callback or `watch` channel on the executor. Not a graph concern. Streaming is an executor + UI implementation detail.

---

## Relationship to Unshape

### Reimplement independently, don't extract or couple

**Decision:** Nanites studies unshape's design as prior art and reimplements the relevant patterns independently. No shared crate, no dependency in either direction.

**Why:** The primitives are fundamentally incompatible — unshape is synchronous (tight 60fps media loop), nanites is async (seconds per LLM call). Extraction would force one to carry the other's baggage. The right time for a shared abstraction is after both projects have discovered their real shapes through use — not now.

If shared code ever makes sense, it should be a third crate that both depend on, not either depending on the other.

### Error bound is `Into<BoxError>`, not `std::error::Error`

**Decision:** Task error bounds use `T::Error: Into<BoxError>` rather than `T::Error: std::error::Error + Send + Sync + 'static`.

**Why:** `BoxError` (`Box<dyn Error + Send + Sync>`) doesn't satisfy `std::error::Error` because `dyn Error` is unsized. This meant any task using `BoxError` as its error type couldn't be erased or spawned. `Into<BoxError>` accepts both: concrete error types (via blanket `From` impl) and `BoxError` itself (via identity).

---

## What nanites is NOT

- Not LLM-specific — LLM calls are one node type among many
- Not a framework you build agents on — orchestration is a Rust program, the substrate just makes it inspectable and composable
- Not conversational — conversation is context poisoning; turns are the primitive, and turns don't accumulate
- Not a graph framework — no `add_node`/`connect`; control flow is the graph
