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

```
async Fn(I) -> Result<O, E>
```

A nanite is a stateless function invocation. No accumulated context, no identity, no session. The orchestration program constructs the input, calls the function, receives the output. LLM calls, deterministic transforms, tool invocations — all just functions.

### Execution Model

- **Orchestration is a Rust program.** Control flow, decomposition, and state management are regular code.
- **LLM as oracle.** The program calls the LLM when world knowledge is needed. The LLM decides; the program acts on the decision.
- **Parallelism via async.** Independent calls run concurrently through tokio. No coordination protocol — just futures.
- **Context construction is explicit.** Each LLM call receives exactly the context it needs, curated by the program. No growing conversation, no compression needed.
- **Recursive decomposition.** Complex tasks decompose into subtasks until the leaves are trivially solvable — potentially without an LLM at all.

### Crate Structure

- **Library crates** — independently useful, composable functions. Parallel combinators, context construction, tool abstractions, structured output parsing.
- **`nanites-rig`** — LLM calls via rig. One implementation of the function primitive. Swappable for any other LLM library.
- **`nanites`** (binary) — the plugin host and orchestrator. Discovers plugins, composes them, presents a unified surface.

### Stack

- **Rust** — the orchestration language
- **rig** — LLM completion primitives (behind `nanites-rig`)
- **tokio** — async runtime for parallel execution

## Development

```bash
nix develop        # Enter dev shell
cargo test         # Run tests
cargo clippy       # Lint
cd docs && bun dev # Local docs
```

## Core Rules

**Note things down immediately — no deferral:**
- Problems, tech debt, issues → TODO.md now, in the same response
- Design decisions, key insights → docs/ or CLAUDE.md
- Future/deferred scope → TODO.md **before** writing any code, not after
- **Every observed problem → TODO.md. No exceptions.** Code comments and conversation mentions are not tracked items. If you write a TODO comment in source, the next action is to open TODO.md and write the entry.

**Conversation is not memory.** Anything said in chat evaporates at session end. If it implies a future behavior change, write it to CLAUDE.md immediately — or it will not happen.

**Warning — these phrases mean something needs to be written down right now:**
- "I won't do X again" / "I'll remember to..." / "I've learned that..."
- "Next time I'll..." / "From now on I'll..."
- Any acknowledgement of a recurring error without a corresponding CLAUDE.md edit

**Triggers:** User corrects you, 2+ failed attempts, "aha" moment, framework quirk discovered → document before proceeding.

**When the user corrects you:** Ask what rule would have prevented this, and write it before proceeding. **"The rule exists, I just didn't follow it" is never the diagnosis** — a rule that doesn't prevent the failure it describes is incomplete; fix the rule, not your behavior.

**Something unexpected is a signal, not noise.** Surprising output, anomalous numbers, files containing what they shouldn't — stop and ask why before continuing. Don't accept anomalies and move on.

**Do the work properly.** Don't leave workarounds or hacks undocumented. When asked to analyze X, actually read X — don't synthesize from conversation.

## Design Principles

**Unify, don't multiply.** One interface for multiple cases > separate interfaces. Plugin systems > hardcoded switches.

**Simplicity over cleverness.** HashMap > inventory crate. OnceLock > lazy_static. Functions > traits until you need the trait. Use ecosystem tooling over hand-rolling.

**Explicit over implicit.** Log when skipping. Show what's at stake before refusing.

**Separate niche from shared.** Don't bloat shared config with feature-specific data. Use separate files for specialized data.

## Workflow

**Batch cargo commands** to minimize round-trips:
```bash
cargo clippy --all-targets --all-features -- -D warnings && cargo test
```
After editing multiple files, run the full check once — not after each edit. Formatting is handled automatically by the pre-commit hook (`cargo fmt`).

**When making the same change across multiple crates**, edit all files first, then build once.

**Minimize file churn.** When editing a file, read it once, plan all changes, and apply them in one pass. Avoid read-edit-build-fail-read-fix cycles by thinking through the complete change before starting.

**`normalize view` is available** for structural outlines of files and directories:
```bash
~/git/rhizone/normalize/target/debug/normalize view <file>    # outline with line numbers
~/git/rhizone/normalize/target/debug/normalize view <dir>     # directory structure
```

**Always commit completed work.** After tests pass, commit immediately — don't wait to be asked. When a plan has multiple phases, commit after each phase passes. Do not accumulate changes across phases. Uncommitted work is lost work.

## Context Management

**Use subagents to protect the main context window.** For broad exploration or mechanical multi-file work, delegate to an Explore or general-purpose subagent rather than running searches inline. The subagent returns a distilled summary; raw tool output stays out of the main context.

Rules of thumb:
- Research tasks (investigating a question, surveying patterns) → subagent; don't pollute main context with exploratory noise
- Searching >5 files or running >3 rounds of grep/read → use a subagent
- Codebase-wide analysis (architecture, patterns, cross-file survey) → always subagent
- Mechanical work across many files (applying the same change everywhere) → parallel subagents
- Single targeted lookup (one file, one symbol) → inline is fine

## Session Handoff

Use plan mode as a handoff mechanism when:
- A task is fully complete (committed, pushed, docs updated)
- The session has drifted from its original purpose
- Context has accumulated enough that a fresh start would help

**For handoffs:** enter plan mode, write a plan containing only: next tasks, blocked/pending items, and what was done this session (only if it directly affects what comes next). Nothing else — no commands, no build steps, no context summaries. Those belong in CLAUDE.md or TODO.md. The next session reads both fresh. **Do NOT investigate first** — the session is context-heavy and about to be discarded.

**For mid-session planning** on a different topic: investigating inside plan mode is fine — context isn't being thrown away.

**TODO.md is the lossless record.** Flush any new items to TODO.md before the handoff. Anything worth preserving belongs in CLAUDE.md or TODO.md — not in memory files.

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

## Negative Constraints

Do not:
- Use Claude Code's auto-memory system (`~/.claude/projects/.../memory/`) — it is unversioned, invisible to the user, and can't be diffed or backed up. Write behavioral changes and project context directly to CLAUDE.md instead
- Announce actions ("I will now...") - just do them
- Leave work uncommitted
- Use interactive git commands (`git add -p`, `git add -i`, `git rebase -i`) — these block on stdin and hang in non-interactive shells; stage files by name instead
- Use path dependencies in Cargo.toml - causes clippy to stash changes across repos
- Use `--no-verify` - fix the issue or fix the hook
- Assume tools are missing - check if `nix develop` is available for the right environment
