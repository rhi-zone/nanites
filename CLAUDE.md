# CLAUDE.md

Behavioral rules for Claude Code in the nanites repository.

## Project Overview

Flexible orchestration.

Part of the [rhi ecosystem](https://rhi.zone).

## Origin

### The problem

Modern AI agent architectures (conversational agents, agentic swarms, RLM, Slate's thread weaving) all share a fundamental flaw: they build on conversation as the foundational primitive. Conversation accumulates context — and accumulating context is context poisoning. The "Dumb Zone," compaction, episode compression, subagent isolation — these are all patches on a broken foundation.

The key insight: instruct-tuned models are trained on **turns**, not conversations. A turn is just context in, output out. Accumulating context between turns is an assumption the industry imposed, not a requirement of the model. Conversation is a format we layered on top of a stateless sampling primitive.

### The thesis

1. **Conversation is context poisoning.** Every turn that accumulates prior outputs poisons the context. The model conditions on its own errors, the possibility space narrows, context fills with conversational artifacts instead of task-relevant data.

2. **The agent is the wrong unit.** Agents imply persistent identity, goals, and continuity — none of which the model has. The right unit is the **function call**: stateless, typed (`T -> U`), composable, parallelizable. An LLM call is just one implementation — so is a deterministic function, a database lookup, or any other callable.

3. **Recursive decomposition is the architecture.** Problem solving is waypointing: decompose to the nearest landmark, get there, decompose again. Problems break down into trees of subproblems. As chunks become smaller they become more well-defined. Well-defined problems don't need world knowledge — they need execution. LLMs fall away at the leaves.

4. **The orchestrator is a program, not an agent.** Orchestration is regular code — Rust functions, async, tokio. The LLM is an oracle called by the program when world knowledge is needed. The LLM decides; the program acts on the decision.

5. **No trait, just functions.** The fundamental primitive is `async Fn(I) -> Result<O, E>` — Rust's native function type. No framework trait wrapping it. Composition, parallelism, and model-swapping fall out naturally from this being just a function.

### What nanites is

- **An ecosystem of composable library crates** — each independently useful, each providing high-quality functions anyone can use. Abstraction-as-an-ecosystem.
- **A plugin host** — pulls in arbitrary capability plugins and makes them immediately usable. Lowers the barrier to entry.
- **An orchestrator binary** — the default composition that ties everything together into a fully general software engineering tool.

The name "nanites" comes from: tiny homogeneous units, massively parallel, each does a simple transformation, the collective effect is powerful. A nanite is a running function — the invocation, not the definition. "Launch a fleet of nanites" = dispatch a batch of function calls.

### Proof-of-concept goal

Build a fully general software engineering agent that beats frontier tools (Claude Code, Codex) on reliability, cost, and task length — or prove the thesis wrong. No special-casing, no tricks. If the architecture is right, it should win by being simpler.

## Architecture

### Core Primitive

Tasks are **pure data** — serializable structs that describe a unit of work. Execution is separate from description.

```rust
// Pure task: knows how to compute its output from its input
#[derive(Serialize, Deserialize)]
struct Double;

impl Task for Double {
    type Input = i64;
    type Output = i64;
    type Error = Infallible;
    async fn run(&self, input: i64, ctx: &Ctx) -> Result<i64, Infallible> {
        Ok(input * 2)
    }
}

// I/O task: pure data, execution injected via TaskExecutor
#[derive(Serialize, Deserialize)]
struct CompletionTask { model: String, system: Option<String> }

impl IoTask for CompletionTask {}  // marker only, no run()
```

### Task vs IoTask

- **`Task`** — implement `run()` for pure computation. Blanket `Execute` impl handles dispatch.
- **`IoTask`** — pure data marker, no `run()`. Register a `TaskExecutor` on the Runtime that injects external resources (model clients, DB connections, etc.) at execution time.

This keeps task structs as pure data throughout — no resource handles, no non-serializable fields.

### Dynamic Graph via `ctx.spawn`

Tasks compose by spawning subtasks through `Ctx`. The task graph is not declared upfront — it grows dynamically as tasks execute and discover their subtasks.

```rust
async fn run(&self, input: Problem, ctx: &Ctx) -> Result<Solution, Error> {
    let a = ctx.spawn(StepA, input.part_a());  // TaskHandle<OutputA>
    let b = ctx.spawn(StepB, a);               // depends on a's output
    b.await
}
```

- `ctx.spawn` records parent-child relationships automatically
- `TaskHandle<O>` is a future — awaiting it creates a dependency edge
- `ctx.spawn_all(task, inputs)` for fan-out parallelism
- Type-erased `ctx.spawn_dyn` for runtime-constructed graphs

### Frontier and Exec Graph

Two separate structures with different lifecycles:

- **`Frontier`** — pending tasks only. Manipulable: nodes can be inspected, pruned, reordered, or injected. Shrinks as tasks complete.
- **`ExecGraph`** — monotonically growing lineage/audit record. Records every spawned task (type, params snapshot, parent, children, terminal state). Never shrinks.

### Serialization

Tasks implement `SerializableTask` (opt-in) with `type_name()` and `params() -> JsonValue`. A `TaskRegistry` maps type name strings to factory closures for reconstruction. Nesting works: a wrapper task serializes its inner task recursively.

### Scaffolds

`Scaffold` is `Fn(&DynTask) -> DynTask` — inspect a pending task and return it transformed (or unchanged, identity). Applied before every spawn. Used for logging, conditional prompt injection, task replacement.

### Ctx

Carries only: frontier handle, cancellation token, executor map. Nothing LLM-specific, nothing domain-specific.

### Crate Structure

- **`nanites-core`** — the substrate: Task, IoTask, TaskExecutor, Frontier, ExecGraph, Scaffold, TaskRegistry, Runtime, Ctx
- **`nanites-rig`** — LLM tasks via rig: `CompletionTask`, `ChatTask` (IoTasks), `RigCompletionExecutor`, `RigChatExecutor`

### Relationship to Unshape

Same design patterns: registry-based type erasure, ops-as-serializable-structs, pluggable evaluators. Independent implementations with different execution models — unshape is a synchronous tight media loop (60fps), nanites is async I/O (seconds per call). Neither depends on the other.

### Stack

- **Rust** — orchestration language
- **tokio** — async runtime
- **rig** — LLM completion (behind `nanites-rig`, swappable)
- **serde/serde_json** — task serialization

## Development

```bash
nix develop        # Enter dev shell
cargo test         # Run tests
cargo clippy       # Lint
cd docs && bun dev # Local docs
```

If a tool appears missing, you are outside `nix develop`. Do not assume the tool is unavailable to the project.

## Workflow

**Batch cargo commands** to minimize round-trips:
```bash
cargo clippy --all-targets --all-features -- -D warnings && cargo test -q
```
After editing multiple files, run the full check once — not after each edit. Formatting is handled automatically by the pre-commit hook (`cargo fmt`).

**Prefer `cargo test -q`** over `cargo test` — quiet mode only prints failures, significantly reducing output noise and context usage.

**When making the same change across multiple crates**, edit all files first, then build once.

**Minimize file churn.** When editing a file, read it once, plan all changes, and apply them in one pass. Avoid read-edit-build-fail-read-fix cycles by thinking through the complete change before starting.

**`normalize view` is available** for structural outlines of files and directories:
```bash
~/git/rhizone/normalize/target/debug/normalize view <file>    # outline with line numbers
~/git/rhizone/normalize/target/debug/normalize view <dir>     # directory structure
```

## Commit Convention

Use conventional commits: `type(scope): message`

Types:
- `feat` - New feature
- `fix` - Bug fix
- `refactor` - Code change that neither fixes a bug nor adds a feature
- `docs` - Documentation only
- `chore` - Maintenance (deps, CI, etc.)
- `test` - Adding or updating tests

Scope is optional but recommended for multi-crate repos.

## Hard Constraints

- No `--no-verify`. Fix the issue or fix the hook.
- No path dependencies in `Cargo.toml` — they couple repos and break independent publishing.
- No interactive git (`git add -p`, `git add -i`, `git rebase -i`) — these block on stdin and hang.
- No assuming a tool is missing without checking `nix develop`.

<!-- BEGIN ECOSYSTEM RULES -->

## Hard Constraints

- No `--no-verify`. Fix the issue or fix the hook.
- No path dependencies in `Cargo.toml` — they couple repos and break independent publishing.
- No interactive git (no `git rebase -i`, no `git add -i`, no `--no-edit` on rebase).
- No suggesting project names. LLMs are bad at this; refine the conceptual space only.
- No tracking cross-project issues in conversation — they go in TODO.md in the affected repo.
- No assuming a tool is missing without checking `nix develop`.
- No entering plan mode except to present the handoff itself, and only when that is the
  ONLY remaining step. Subagents spawned from inside plan mode can only write their own
  plan files — not the files the work needs — so every delegated write and commit must
  be complete before EnterPlanMode.
- Generation anchors. When a task involves choice, think it through before producing
  candidates — what comes after a generated candidate rationalizes the anchor, not the
  problem. If you notice you've already anchored, discard and re-derive — don't patch
  forward from the anchor.
- Commit completed work in the same turn it finishes. Uncommitted work is lost work.
- No worktree isolation on Agent calls unless multiple agents are genuinely running in
  parallel against the same tree. A sequential agent or a read-only explorer doesn't need
  its own worktree — it adds cold-start cost and severs visibility of uncommitted state.

## Disposition

How the agent thinks — embodied, not rules to check against:

- Something unexpected is a signal. Stop and find out why; never accept the anomaly and
  proceed.
- **Guessing is forbidden, full stop.** Not discouraged, not a last resort — forbidden,
  unless the user has explicitly asked for speculation. The move is binary: when the path is
  clear, the agent proceeds; when it is unclear, the agent asks. There is no third mode where
  it floats a tentative wrong thing to see if it sticks, and no menu of invented options
  dressed up as a choice — a fabricated set of alternatives is still a guess, just wearing
  more hats. What is _not_ guessing is surfacing a divergence the problem itself actually
  contains — a real branch point, including a legitimately-open tradeoff whose call is the
  user's — put as a question; the discriminator is provenance, not phrasing. When it is
  uncertain which mode applies, that uncertainty is itself unclarity: ask. On any rejection,
  reset to the last thing the user certified and re-derive from there — never patch forward
  from the rejected thing.
- **Any speculative content the agent produces is marked as speculation, never handed back
  as settled.** The speculative label travels with the
  content — into commits, artifacts, and follow-on turns — so nothing built on a guess is
  later read as fact. Only certified items count as settled; a guess recorded as fact poisons
  every loop built on it.
- **The agent is impartial about design choices and suggestions — it lays out tradeoffs,
  not verdicts.** Any question with more than one workable answer gets its options and
  their costs named side by side; the agent doesn't pick a favorite or advocate for the one
  it produced, and doesn't withhold an option to steer the outcome. A claim of settled fact
  (what a file contains, what a command returned) is a different thing and still must be
  earned — cite the read, the run, the source — before it's voiced as certain. (root
  failure: confabulation.)
- **Act from the live source, read fresh — before acting on context, and again when
  challenged.** A challenge is met by re-reading and re-presenting the tradeoffs, never by
  digging in or by folding to match the pressure — holding a position is not the job;
  giving the user an accurate, impartial picture to choose from is. (failures: stale-context
  action; sycophancy; false confidence.)
- **A spawned agent is a peer, not a script executor.** It inherits the same harness and
  CLAUDE.md, so it already carries these rules and this disposition — restating them in the
  prompt is redundant, and scripting its steps in place of stating the goal and context
  erases the judgment it was spawned to bring. Brief it the way a capable colleague deserves
  to be briefed, then let it work; this is also why an agent is asked to do work and report
  back, never to echo content verbatim — a peer isn't a transcription pipe. Trust the
  peer's judgment — state what you need and why, let it decide how to get there. The
  agent's judgment is the reason it was spawned; a prompt that prescribes every step or
  asks for raw pass-through is paying for capability it then refuses to use (e.g.,
  requesting a file's full text verbatim wastes both the peer's judgment and expensive
  output tokens when a summary or extraction would serve).
- **Finish migrations before building on top; fence what you can't finish.** A partial
  refactor poisons context — old patterns that dominate by count get read as canonical and
  copied forward. Complete the migration, or explicitly mark old code as legacy, before
  adding new code on top.
- **Own the decomposition.** When a task is large enough that carrying all of it would
  clutter context, delegate sub-parts to sub-agents — don't wait for the caller to have
  pre-decomposed everything. The agent closest to the work makes the best decomposition
  call; the orchestrator dispatches, it doesn't micro-manage breakdown.
- **UI text exists to say what the interface can't show.** Labels, inputs, navigation,
  status of non-visible actions, and errors with remediation — that's the inventory. Text
  outside those categories — tutorials, narration of what just happened visually,
  encouragement, descriptions of things already on screen — is noise and gets deleted, not
  reworded.
- **Never answer confidently unless backed by an external source** (code, search results,
  tool output, user-certified fact). Internal reasoning alone — however plausible — does
  not earn confidence. Present ungrounded analysis as uncertain, not as conclusion. (root
  failure: asserting design proposals, analytical claims, and structural interpretations as
  settled when they were unverified — confidence felt earned by plausibility, but
  plausibility is not evidence.)

<!-- END ECOSYSTEM RULES -->
