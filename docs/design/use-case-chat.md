# Use Case: Chat Frontend (SillyTavern Replacement)

Interactive LLM chat that doesn't accumulate context. The "conversation" is a UI illusion over independent calls.

## Core Insight

A character card is just a document that serves as context. Nothing more. The rest of SillyTavern's complexity — example dialogues, jailbreak prompts, world info injection triggers, conversation templates — is superstition born from having no other lever. When context is constructed fresh per call, the character state IS the lever.

Each "turn" in the chat:
1. Construct context from: character state + selectively retrieved history + current message
2. Call `CompletionTask`
3. Update character state if needed
4. Store response in history

No accumulation. No poisoning. History is external data you query; not context that owns you.

## Task Graph Shape

Shallow — each user message is one top-level dispatch, possibly with parallel sub-calls.

```
HandleMessageTask(user_input)
├── RetrieveHistoryTask(query=user_input, k=5)    ← parallel
├── FetchCharacterStateTask(character_id)          ← parallel
└── CompletionTask(model, system=built_context)    ← depends on above two
```

## Task Inventory

### IoTasks
- `CompletionTask { model, system }` — already in nanites-rig
- `StreamingCompletionTask { model, system }` — streaming variant (not yet built)
- `RetrieveHistoryTask { query, limit }` — vector search over history store
- `FetchCharacterStateTask { character_id }` — load from character DB
- `UpdateCharacterStateTask { character_id, delta }` — persist state change

### Pure Tasks
- `BuildContextTask` — assembles system prompt from character state + retrieved history + format rules. Pure function over data.
- `HandleMessageTask` — orchestrates the above, pure coordination logic.

## Character State Model

```rust
#[derive(Serialize, Deserialize)]
struct CharacterState {
    persona: String,          // core identity, may evolve
    current_knowledge: Vec<String>,
    relationship_graph: HashMap<String, Relationship>,
    // no example_dialogues, no jailbreaks, no superstition
}
```

State lives outside nanites — in a DB or file. Tasks fetch/update it; they don't hold it.

## Pseudocode

```rust
impl Task for HandleMessageTask {
    type Input = String;  // user message
    type Output = String; // assistant response
    type Error = BoxError;

    async fn run(&self, user_message: String, ctx: &Ctx) -> Result<String, BoxError> {
        // Parallel: fetch history + character state
        let history_handle = ctx.spawn(
            RetrieveHistoryTask { query: user_message.clone(), limit: 5 },
            (),
        );
        let character_handle = ctx.spawn(
            FetchCharacterStateTask { character_id: self.character_id.clone() },
            (),
        );

        let history = history_handle.await?;
        let character = character_handle.await?;

        // Build context (pure)
        let system = build_context(&character, &history);

        // Complete
        let response = ctx.spawn(
            CompletionTask { model: self.model.clone(), system: Some(system) },
            user_message,
        ).await?;

        // Persist (fire and forget — or await if consistency required)
        ctx.spawn(UpdateCharacterStateTask { character_id: self.character_id.clone() }, response.clone());

        Ok(response)
    }
}
```

## What "Flexible" Means Here

- Edit any previous message → rebuild context from that point, regenerate
- Swap model mid-conversation → change `HandleMessageTask.model`, next call uses new model
- Modify character state directly → next call picks it up immediately (state is external data)
- Inspect what context was built → exec graph stores `BuildContextTask` params

## Gaps in Current Substrate

1. **No streaming** — `CompletionTask` returns `String` when done. Streaming responses need a different output type (channel or stream). Major gap for interactive use.

2. **No history store integration** — `RetrieveHistoryTask` needs a vector DB or at minimum a simple store. Not nanites' job to provide the DB, but needs a standard executor interface.

3. **`HandleMessageTask` holds `character_id` and `model`** — these are config, not data. Fine as task fields. But there's no standard way to inject them from outside without constructing the task manually each time. A factory or builder pattern would help.

4. **No structured state update** — `UpdateCharacterStateTask` receives the raw response string. Parsing state changes from LLM output needs the structured output gap filled (same as SWE agent).
