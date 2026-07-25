# nanites

Composable orchestration library for Rust: work as serializable data, execution as pluggable backends.

Nanites describes work as typed task structs rather than agent loops. A task is a plain
`T -> U` function call — stateless, serializable, and executed by whatever backend is
registered for it: an LLM, a deterministic Rust function, an ML model, a Lua script, a
shell command, or another rhi-ecosystem tool. The same task can run against different
backends without changing the code that composes it.

## Key concepts

- **Functions, not agents** — the fundamental unit is a typed function call. LLM calls are
  one implementation among several, not a privileged special case.
- **Recursive decomposition** — problems break into trees of subproblems; LLMs sit at the
  leaves where the problem is well-defined, while the orchestrator itself is an ordinary
  program.
- **Parallelism by default** — independent tasks run concurrently via `ctx.spawn`; the
  execution shape follows the problem shape with no separate coordination protocol.
- **The exec graph as data** — every execution is logged with full inputs and outputs,
  giving an inspectable, replayable audit trail that can also be used to train cheaper
  models to replace expensive ones.

## Crates

- `nanites-core` — the task substrate: `Task`, `IoTask`, `TaskExecutor`, `Frontier`,
  `ExecGraph`, `Scaffold`, `TaskRegistry`, `Checkpoint`, `Cache`, combinators (`Map`,
  `Refine`, `Retry`), and the `Runtime`.
- `nanites-rig` — LLM completion tasks built on [rig](https://github.com/0xPlaygrounds/rig):
  `CompletionTask`, `ChatTask`, `StructuredCompletionTask`, `EmbeddingTask`,
  `VectorStoreTask`, `RetrieveTask` (feature-gated: `completions`, `embeddings`, `rag`).
- `nanites-chat` — a chat orchestration layer built on the substrate (proof of concept):
  `HandleMessageTask`, `BuildContextTask`, `CharacterState`.

Documentation: https://docs.rhi.zone/nanites/

## License

Licensed under MIT OR Apache-2.0.
