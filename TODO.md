# TODO

## Immediate

- [ ] Refactor: remove `Turn` trait, use bare `async Fn` with generic function combinators
- [ ] Split crates: move LLM/rig code into `nanites-rig`, keep core as lightweight combinators + error types
- [ ] Build context construction primitives (file reading, search, structural slicing)
- [ ] Implement recursive decomposition: a program that decomposes a task into a tree of function calls
- [ ] First end-to-end demo: solve a real coding task using only nanites

## Open Questions

- What does the context construction API look like? How does the program know what to include?
- How do function outputs compose when feeding into subsequent calls?
- What's the right interface for the human to provide the initial task?
- How to handle tasks where the decomposition itself requires world knowledge (LLM call to decide the decomposition)?
- Plugin discovery and registration: how do plugins expose their capabilities to the orchestrator?
- If heterogeneous dispatch is needed: proc macro to lift function shapes into value enums? Or is it never needed in practice?

## Future

- [ ] Benchmark against Claude Code / Codex on real tasks
- [ ] Cross-model composition (mix Sonnet/Opus/Haiku across calls by cost/capability)
- [ ] Tool integration (filesystem, shell, search) as plugin crates
- [ ] Plugin host: discover and load capability plugins at runtime

### [x] Update CLAUDE.md — corrections as documentation lag (2026-03-29)

Add to the corrections section:
> **Corrections are documentation lag, not model failure.** When the same mistake recurs, the fix is writing the invariant down — not repeating the correction. Every correction that doesn't produce a CLAUDE.md edit will happen again. Exception: during active design, corrections are the work itself — don't prematurely document a design that hasn't settled yet.

Add to the Session Handoff section:
> **Initiate a handoff after a significant mid-session correction.** When a correction happens after substantial wrong-path work, the wrong reasoning is still in context and keeps pulling. Writing down the invariant and starting fresh beats continuing with poisoned context — the next session loads the invariant from turn 1 before any wrong reasoning exists.

Conventional commit: `docs: add corrections-as-documentation-lag + context-poisoning handoff rule`

