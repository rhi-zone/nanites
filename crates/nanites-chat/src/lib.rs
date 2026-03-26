//! nanites-chat — chat orchestration layer built on the nanites substrate.
//!
//! Implements the character-based chat use case described in
//! `docs/design/use-case-chat.md`. Each "turn" constructs a fresh context from
//! character state and recent history, calls the LLM, and returns the response.
//! No context accumulation — history is external data, not owned state.
//!
//! # Task graph per turn
//!
//! ```text
//! HandleMessageTask(user_input)
//! └── BuildContextTask(character, history)   ← pure, no LLM
//!     └── CompletionTask { model, system }   ← IoTask via nanites-rig
//! ```
//!
//! # Usage
//!
//! ```no_run
//! use nanites_chat::{CharacterState, HistoryEntry, HandleMessageTask};
//! use nanites_core::Runtime;
//! use nanites_rig::RigCompletionExecutor;
//!
//! # async fn example() {
//! // Register the rig executor so CompletionTask can run.
//! // let model = rig::providers::openai::Client::from_env().completion_model("gpt-4o");
//! // let executor = RigCompletionExecutor::new().with_model("gpt-4o", model);
//! // let runtime = Runtime::new().with_executor(executor);
//!
//! // let character = CharacterState {
//! //     name: "Aria".into(),
//! //     persona: "A thoughtful AI assistant.".into(),
//! //     current_knowledge: vec![],
//! // };
//! // let response = runtime
//! //     .run(
//! //         HandleMessageTask {
//! //             model: "gpt-4o".into(),
//! //             character: character.clone(),
//! //             history: vec![],
//! //         },
//! //         "Hello!".to_string(),
//! //     )
//! //     .await
//! //     .unwrap();
//! // println!("{response}");
//! # }
//! ```

use nanites_core::dyn_task::erase_io;
use nanites_core::{BoxError, Ctx, RuntimeError, Task};
use nanites_rig::CompletionTask;
use serde::{Deserialize, Serialize};

// ─── Data types ───────────────────────────────────────────────────────────────

/// Persistent character identity. Passed into each turn — lives outside the runtime.
///
/// The persona is the seed. No example dialogues, no jailbreaks, no superstition.
/// The character state IS the lever; assembling it into a system prompt is enough.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterState {
    /// Character name (used in the system prompt header).
    pub name: String,
    /// Core identity description — who this character is and how they behave.
    pub persona: String,
    /// Facts, context, or knowledge the character currently holds.
    pub current_knowledge: Vec<String>,
}

/// A single entry in the conversation history.
///
/// Stored externally (caller's responsibility). Pass a recent slice to
/// [`HandleMessageTask`] — the task does not own or accumulate history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// `"user"` or `"assistant"`.
    pub role: String,
    /// Message content.
    pub content: String,
}

// ─── BuildContextTask ─────────────────────────────────────────────────────────

/// Pure task: assembles a system prompt from character state and recent history.
///
/// Input: the current user message (used only for inclusion in context if needed).
/// Output: the system prompt string to pass to the LLM.
///
/// No LLM call. This is pure string assembly — deterministic, fast, testable.
#[derive(Debug, Clone)]
pub struct BuildContextTask {
    /// Character state to embed in the system prompt.
    pub character: CharacterState,
    /// Recent conversation history to include for continuity.
    pub history: Vec<HistoryEntry>,
}

impl Task for BuildContextTask {
    type Input = ();
    type Output = String;
    type Error = std::convert::Infallible;

    async fn run(&self, _input: (), _ctx: &Ctx) -> Result<String, Self::Error> {
        let mut parts: Vec<String> = Vec::new();

        // Identity header.
        parts.push(format!("You are {}.", self.character.name));
        parts.push(self.character.persona.clone());

        // Knowledge, if any.
        if !self.character.current_knowledge.is_empty() {
            parts.push(String::new());
            parts.push("What you currently know:".into());
            for fact in &self.character.current_knowledge {
                parts.push(format!("- {fact}"));
            }
        }

        // Recent history, if any.
        if !self.history.is_empty() {
            parts.push(String::new());
            parts.push("Recent conversation:".into());
            for entry in &self.history {
                parts.push(format!("{}: {}", entry.role, entry.content));
            }
        }

        Ok(parts.join("\n"))
    }
}

// ─── HandleMessageTask ────────────────────────────────────────────────────────

/// Error type for [`HandleMessageTask`].
#[derive(Debug, thiserror::Error)]
pub enum HandleMessageError {
    /// The runtime rejected the task (cancellation, type mismatch, no executor).
    #[error("runtime error: {0}")]
    Runtime(#[from] RuntimeError),
    /// The context-build step failed (should not happen — `BuildContextTask` is infallible).
    #[error("context build failed: {0}")]
    ContextBuild(BoxError),
    /// The completion response was not a `String` (internal invariant violation).
    #[error("unexpected completion output type")]
    BadOutputType,
}

/// Orchestrating task: handles one chat turn end-to-end.
///
/// Steps:
/// 1. Build a system prompt (`BuildContextTask` — pure).
/// 2. Call the LLM (`CompletionTask` — `IoTask` via `RigCompletionExecutor`).
///
/// Input: the user's message (`String`).
/// Output: the assistant's reply (`String`).
///
/// Requires a [`nanites_rig::RigCompletionExecutor`] registered on the runtime
/// under the `model` name provided in this struct.
#[derive(Debug, Clone)]
pub struct HandleMessageTask {
    /// Model identifier — must match a name registered in `RigCompletionExecutor`.
    pub model: String,
    /// Character configuration for this turn.
    pub character: CharacterState,
    /// Recent history to include in context (caller selects which entries).
    pub history: Vec<HistoryEntry>,
}

impl Task for HandleMessageTask {
    type Input = String;
    type Output = String;
    type Error = HandleMessageError;

    async fn run(&self, user_message: String, ctx: &Ctx) -> Result<String, HandleMessageError> {
        // Step 1: build context (pure — no LLM call).
        let system_prompt = ctx
            .spawn(
                BuildContextTask {
                    character: self.character.clone(),
                    history: self.history.clone(),
                },
                (),
            )
            .await
            .map_err(|e| HandleMessageError::ContextBuild(Box::new(e)))?;

        // Step 2: call the LLM via CompletionTask (IoTask — dispatched by executor).
        let completion_task = CompletionTask {
            model: self.model.clone(),
            system: Some(system_prompt),
        };

        let output = ctx
            .spawn_dyn(erase_io(completion_task), Box::new(user_message))
            .await?;

        // Downcast the type-erased output to String.
        let response = *output
            .downcast::<String>()
            .map_err(|_| HandleMessageError::BadOutputType)?;

        Ok(response)
    }
}
