# Consumer Sketches

Use cases that nanites should support. Each describes what the consumer needs from the orchestration layer.

## Software Engineering Agent

A fully general coding agent — the PoC that proves the thesis.

**What it does:** Takes a task description, decomposes it recursively, reads/writes files, runs commands, produces working code.

**Orchestration shape:** Recursive decomposition tree. Top-level LLM call breaks the task into subtasks. Subtasks may be further decomposed or executed directly. Leaves are tool calls (file read, file write, shell exec, search) or small LLM calls with focused context.

**Key properties needed:**
- Recursive decomposition with LLM at decision points
- Parallel execution of independent subtasks
- Tool plugins: filesystem, shell, search, structural analysis (normalize)
- Pause/resume: human can inspect state between any two calls
- Steering: human can redirect mid-task
- Model mixing: cheap model for simple subtasks, expensive for hard ones

**What "flexible" means here:** The human can pause after any step, inspect what was decided, override a decomposition, skip a subtask, or inject new information. The agent never runs away from you.

---

## Chat Frontend (SillyTavern Replacement)

An LLM chat interface that doesn't suck — not conversational in the traditional sense.

**What it does:** Interactive LLM chat with rich context management, evolving characters, multiple backends.

**Orchestration shape:** Not a growing conversation. Each message constructs fresh context from structured character state and selectively retrieved history. The "conversation" is a UI illusion over independent calls.

**Character model:** A character is structured, evolving state — personality traits, current knowledge, relationship graph, emotional state — not a static card. The core persona definition is the seed, but even that can evolve. Context construction pulls what's relevant for *this specific response* from the character's current state.

No "example dialogues," no "jailbreak prompts," no prompt engineering superstition. Those exist because conversation-based systems give users no other lever. With fresh context per call, the character state IS the lever.

**Key properties needed:**
- Context construction from evolving character state
- Selective history retrieval (relevant moments, not everything)
- Multiple LLM backends (swap mid-conversation)
- Plugin system for context sources (RAG, knowledge bases, external state)
- Streaming responses
- History and character state are external, mutable data — not accumulated context

**What "flexible" means here:** You can edit any previous message and regenerate from that point. You can swap models mid-conversation. You can modify character state directly at any point. History is data you control, not context that controls you.

---

## Data Pipeline

Structured data transformation with LLM steps where needed.

**What it does:** Processes data through a series of transformation steps, some deterministic, some LLM-powered.

**Orchestration shape:** DAG of functions. Each node is a pure transformation. LLM nodes handle fuzzy steps (classification, extraction, summarization). Deterministic nodes handle everything else.

**Key properties needed:**
- DAG execution with dependency tracking
- Parallel execution of independent nodes
- Checkpoint/resume: save intermediate state, restart from any node
- Mix of LLM and deterministic steps seamlessly
- Typed inputs/outputs between nodes

**What "flexible" means here:** You can rerun any node with different parameters. You can replace an LLM node with a deterministic one as you learn the patterns. You can fork a pipeline mid-execution.

---

## Interactive Tool (Scribble Integration?)

A creative tool where LLM assists in real-time.

**What it does:** LLM provides suggestions, completions, or transformations on creative content as the user works.

**Orchestration shape:** Event-driven. User actions trigger function calls. Each call is independent — fresh context from the current document state.

**Key properties needed:**
- Low latency (small, focused calls)
- Cancellation (user types more, previous call is irrelevant)
- No history accumulation — each call sees current document state
- Plugin system for different creative domains

**What "flexible" means here:** The tool responds to what you're doing now, not what you did five minutes ago. Every call is fresh.

---

## Common Patterns

Across all consumers:

1. **Fresh context per call** — no accumulation
2. **External state** — the consumer manages state, not the orchestration layer
3. **Plugin system** — capabilities are pluggable
4. **Pause/inspect/resume** — execution is controllable
5. **Model-swappable** — LLM calls don't care which model
6. **Parallel where possible** — independent work runs concurrently

## Open Questions

- Is there a single orchestration binary, or does each consumer write its own?
- What's the plugin interface? How does a plugin declare what it can do?
- How does the orchestration layer differ from "just writing a Rust program that calls functions"?
- Where does the UI live for interactive consumers (chat, creative tool)?
