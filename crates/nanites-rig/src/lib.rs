use nanites_core::{Ctx, SerializableTask, Task, TaskRegistry};
use rig::completion::CompletionModel;
use serde::{Deserialize, Serialize};

// ─── Error ────────────────────────────────────────────────────────────────────

/// Error returned by nanites-rig tasks.
#[derive(Debug, thiserror::Error)]
pub enum RigTaskError {
    /// The task was called without a concrete model injected at runtime.
    /// Use the standalone [`complete`] helper when you have a model in hand.
    #[error("task requires a concrete CompletionModel at runtime: {0}")]
    NoModel(String),
}

// ─── CompletionTask ───────────────────────────────────────────────────────────

/// A serializable task that represents a single, stateless LLM completion call.
///
/// Because rig's [`CompletionModel`] is a generic trait that cannot be stored
/// as a trait object inside a `Serialize`/`Deserialize` struct, `CompletionTask`
/// records the intent (model id + system prompt) but cannot execute directly
/// via the Task substrate without a model being supplied externally.
///
/// For orchestration, register this task with the registry via [`register_all`]
/// and supply the model through a wrapper or scaffold. For direct use, call the
/// standalone [`complete`] helper.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionTask {
    /// Model identifier (e.g. `"gpt-4o"`, `"claude-3-5-haiku"`).
    pub model: String,
    /// Optional system / preamble prompt.
    pub system: Option<String>,
}

impl Task for CompletionTask {
    type Input = String;
    type Output = String;
    type Error = RigTaskError;

    async fn run(&self, _input: Self::Input, _ctx: &Ctx) -> Result<Self::Output, Self::Error> {
        Err(RigTaskError::NoModel(self.model.clone()))
    }
}

impl SerializableTask for CompletionTask {
    fn serializable_type_name(&self) -> &'static str {
        "nanites_rig::CompletionTask"
    }

    fn params(&self) -> serde_json::Value {
        serde_json::json!({
            "model": self.model,
            "system": self.system,
        })
    }
}

// ─── ChatTask ─────────────────────────────────────────────────────────────────

/// A single message in a multi-turn conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    /// `"user"` or `"assistant"`.
    pub role: String,
    /// Message body.
    pub content: String,
}

/// A serializable task for multi-turn LLM conversations.
///
/// Input is the full conversation history; output is the assistant's reply.
/// See [`CompletionTask`] for notes on runtime model injection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatTask {
    /// Model identifier.
    pub model: String,
    /// Optional system / preamble prompt.
    pub system: Option<String>,
}

impl Task for ChatTask {
    type Input = Vec<ChatMessage>;
    type Output = String;
    type Error = RigTaskError;

    async fn run(&self, _input: Self::Input, _ctx: &Ctx) -> Result<Self::Output, Self::Error> {
        Err(RigTaskError::NoModel(self.model.clone()))
    }
}

impl SerializableTask for ChatTask {
    fn serializable_type_name(&self) -> &'static str {
        "nanites_rig::ChatTask"
    }

    fn params(&self) -> serde_json::Value {
        serde_json::json!({
            "model": self.model,
            "system": self.system,
        })
    }
}

// ─── Registry ─────────────────────────────────────────────────────────────────

/// Register all nanites-rig task types with the given registry.
///
/// Registered type names:
/// - `"nanites_rig::CompletionTask"`
/// - `"nanites_rig::ChatTask"`
pub fn register_all(registry: &mut TaskRegistry) {
    registry.register_with_name::<CompletionTask>("nanites_rig::CompletionTask");
    registry.register_with_name::<ChatTask>("nanites_rig::ChatTask");
}

// ─── Standalone helpers ────────────────────────────────────────────────────────

/// One-shot LLM completion. Fresh context, no history, no accumulation.
///
/// This is the direct bridge to rig's completion API — useful when you have a
/// concrete [`CompletionModel`] in hand and don't need the Task substrate.
/// For orchestrated use, wrap this call in your own `Task` impl that holds a
/// model reference.
pub async fn complete<M: CompletionModel>(
    model: &M,
    system: Option<&str>,
    prompt: impl Into<String>,
) -> Result<String, rig::completion::CompletionError> {
    let mut builder = model.completion_request(prompt.into());
    if let Some(system) = system {
        builder = builder.preamble(system.to_owned());
    }
    let request = builder.build();
    let response = model.completion(request).await?;
    let text = response
        .choice
        .into_iter()
        .filter_map(|c| match c {
            rig::completion::AssistantContent::Text(t) => Some(t.text),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("");
    Ok(text)
}
