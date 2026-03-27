# Nanites: Composable Orchestration Library

## What it is

A Rust library for describing, executing, inspecting, and optimizing work. Work is data — serializable task structs with typed inputs and outputs. Execution is pluggable — LLMs, ML models, deterministic functions, ecosystem tools, Lua scripts, shell commands. The same task interface regardless of what's behind it.

Not AI-specific. Not a framework. Not a server. You import it, compose tasks, run them.

## Core loop

1. Describe work as task data
2. Execute with pluggable backends
3. Inspect the exec graph (what ran, what spawned what, with what inputs/outputs)
4. Swap backends as needed — the exec graph provides the data, the user decides what to do with it

## Execution backends

A backend is a `TaskExecutor` registered on the runtime. Same task, different executor.

| Backend | Examples | When to use |
|---------|----------|-------------|
| **LLM** | OpenAI, Anthropic, Ollama, ... | World knowledge, fuzzy reasoning, natural language |
| **ML model** | xgboost, tf-idf, sklearn | Classification/extraction once you have training data |
| **Deterministic** | Pure Rust `Task::run()` | Well-defined transforms, parsing, validation |
| **Ecosystem tool** | normalize, tiltshift, paraphase, rescribe, gels | Structural analysis, format conversion, code intelligence |
| **Lua script** | moonlet/crescent | User-authored logic, rapid iteration, plugins |
| **Shell** | Any CLI tool | External tooling, build systems, test runners |

## Task shapes

All tasks are `T -> U`. These semantic categories exist for discoverability and executor swapping:

| Shape | Signature | Example |
|-------|-----------|---------|
| **Transform** | `T -> U` | Parse CSV, format output, apply patch |
| **Classify** | `T -> Enum` | Bug vs feature, file type detection, risk assessment |
| **Extract** | `T -> Structured` | Pull entities from text, parse AST, extract schema |
| **Generate** | `Spec -> Content` | Write code, produce media, fill template |
| **Search** | `Query -> Vec<Result>` | Code search, vector similarity, grep |
| **Validate** | `T -> bool` | Type check, test pass, constraint satisfaction |

These are documentation, not types. The substrate doesn't distinguish them — they're all just `Task` or `IoTask`.

## Combinators

Composable wrappers. Each is a serializable struct holding another task.

| Combinator | What it does | Status |
|------------|-------------|--------|
| **Map** | Broadcast task over a collection | Built |
| **Refine** | Loop until convergence or max iterations | Built |
| **Retry** | Re-run on error up to N times | Built |
| **Route** | Classify, then dispatch to matching branch | Design |
| **Pipeline** | Sequential chain (output of A → input of B) | Expressible via ctx.spawn |
| **Race** | Run N alternatives, take first to complete | Design |
| **Timeout** | Fail or fallback after duration | Design |
| **Gate** | Pause for human approval before proceeding | Design |

## The exec graph as data

Every task execution is logged with full inputs and outputs. This is an audit trail — but it's also a dataset. What you do with it is up to you.

**One possible use:** train cheaper models to replace expensive ones. The exec graph accumulates `(input, output)` pairs. You could train a classifier on them. Or you could use them for evaluation, debugging, compliance, visualization, whatever.

Nanites doesn't judge. It logs everything, makes it queryable, and gets out of the way.

## Ecosystem integration

Every rhi project is a potential executor:

| Project | Task surface |
|---------|-------------|
| **normalize** | `AnalyzeCodeTask`, `ViewStructureTask`, `SearchSymbolTask`, `TraceValueTask` |
| **tiltshift** | `ExtractBinaryStructureTask`, `InferFormatTask` |
| **paraphase** | `PlanConversionTask`, `ConvertFormatTask` |
| **rescribe** | `ConvertDocumentTask` |
| **gels** | `InferGrammarTask`, `ParseWithGrammarTask` |
| **unshape** | `GenerateMeshTask`, `EvaluateGraphTask` |
| **wick** | `EvaluateExpressionTask` |
| **moonlet** | `RunLuaTask` (with capability-based security) |

These don't all need to exist as crates today. The point is: the task interface is general enough that any tool can be wrapped as an executor.

## What's built

- **nanites-core** — Task, IoTask, TaskExecutor, Frontier, ExecGraph, Scaffold, TaskRegistry, Checkpoint, Cache, Combinators (Map, Refine, Retry), Runtime
- **nanites-rig** — CompletionTask, ChatTask, StructuredCompletionTask, EmbeddingTask, VectorStoreTask, RetrieveTask (feature-gated: completions, embeddings, rag)
- **nanites-chat** — HandleMessageTask, BuildContextTask, CharacterState (proof of concept)

## What's next

### Combinators
- `Route` — classify → dispatch. Needs `StructuredCompletionTask<Enum>` + branch map.
- `Race` — spawn N, cancel losers when first completes.
- `Timeout` — wrapper with duration, falls back or errors.
- `Gate` — async pause for external approval signal.

### Executor ecosystem
- `NormalizeExecutor` — wraps normalize for code intelligence tasks
- `ShellExecutor` — runs CLI commands with sandboxing
- `MoonletExecutor` — runs Lua scripts in moonlet with capability constraints
- `MlExecutor` — generic ML model inference (sklearn, xgboost via FFI or subprocess)

### Training data extraction
- `ExecGraph::export_training_data(task_type)` — extract `(input, output)` pairs for a given task type
- Standard formats (CSV, JSONL) for offline training

### The SWE agent
- The thesis-prover. Built on all of the above.
- Recursive decomposition with normalize for code understanding
- LLM at decision points, deterministic tools at leaves
- Progressive optimization as exec graph data accumulates

## Protocol exposure via server-less

Tasks with typed input/output can be exposed as protocol endpoints via server-less (derive macros: one impl → many protocols). A nanites task automatically becomes an MCP tool, gRPC service, REST endpoint, etc. No special integration needed in nanites — server-less derives the surface.

## Prior art

| Project | What it does | What nanites learns |
|---------|-------------|-------------------|
| **ComfyUI** | Node-based Stable Diffusion workflows | Graph as project file (serializable, sharable) |
| **n8n** | Workflow automation with AI nodes | Classification/routing as first-class node types |
| **Node-RED** | Flow-based I/O automation | Pluggable node registry |
| **maki** (abandoned, ours) | Node-based AI kitchen on Baklava + MCP + Vercel AI SDK | MCP as typed tool interface, JSON Schema wire types, all providers via Vercel AI SDK |
| **unshape** (ours) | Node-based media generation | Registry-based type erasure, ops as serializable structs, pluggable evaluators |

**Key divergence from all of these:** nanites builds the graph dynamically via code, not visually. Node-based editors are programming with worse ergonomics. Nanites gets the benefits (inspection, serialization, replay) without the visual editor tax.

**maki specifically:** proved that MCP + typed schemas + multi-provider AI is the right surface. But the node-based UI was a dead end. Nanites inherits the architecture without the editor.

## Resolved questions

- **Route fallback** — impossible with structured output. Classification returns a typed enum; unknown variants are a type error, not a runtime concern.
- **Gate scope** — up to the implementor. `Gate` is a combinator wrapping a subtree; whether the runtime blocks elsewhere is the user's decision.
- **ML model serving** — no single answer. FFI, subprocess, gRPC all have different tradeoffs (latency, isolation, language support). Provide executor implementations for each; the user picks what fits.
- **Crescent surface** — crescent reimplements the substrate as a general orchestration library in pure Lua, not called "nanites." Same design patterns, different language, no dependency on the Rust crate.
- **Vercel AI SDK vs rig** — AI SDK is TypeScript-only, not an option for Rust. Rig is fine for the Rust surface. Many providers are just OpenAI-compatible APIs anyway — crescent can hit them directly without a dedicated SDK per provider.

## Open questions

- What does streaming look like at the executor + UI level? (Decided it's not a graph primitive, but the integration story is undesigned)
