# TODO

## Immediate

- [ ] Define the `Turn` trait: `T -> U` with async support
- [ ] Integrate rig for model-agnostic LLM calls as one Turn implementation
- [ ] Build context construction primitives (file reading, search, structural slicing)
- [ ] Implement recursive decomposition: a program that decomposes a task into a tree of turns
- [ ] First end-to-end demo: solve a real coding task using only turns

## Open Questions

- What does the context construction API look like? How does the program know what to include?
- How do turn outputs compose when feeding into subsequent turns?
- What's the right interface for the human to provide the initial task?
- How to handle tasks where the decomposition itself requires world knowledge (LLM turn to decide the decomposition)?

## Future

- [ ] Benchmark against Claude Code / Codex on real tasks
- [ ] Cross-model composition (mix Sonnet/Opus/Haiku across turns by cost/capability)
- [ ] Tool integration (filesystem, shell, search) as turn-compatible primitives
