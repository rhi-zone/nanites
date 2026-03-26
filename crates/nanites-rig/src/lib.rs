//! nanites-rig — LLM completion tasks for the nanites orchestration runtime.
//!
//! # Design
//!
//! [`CompletionTask`] and [`ChatTask`] are **pure data structs** — they carry
//! configuration (model name, optional system prompt) but no resource handles.
//! They implement [`IoTask`], not [`Task::run`], because executing them requires
//! an LLM model client that cannot live inside a serializable struct.
//!
//! To run them, register a [`RigCompletionExecutor`] (or [`RigChatExecutor`])
//! with the runtime:
//!
//! ```no_run
//! use nanites_core::Runtime;
//! use nanites_rig::{CompletionTask, RigCompletionExecutor};
//! use nanites_core::dyn_task::erase_io;
//!
//! # async fn example() {
//! // Build your rig model however you like, then register it:
//! // let model = openai_client.completion_model("gpt-4o");
//! // let executor = RigCompletionExecutor::new().with_model("gpt-4o", model);
//! // let runtime = Runtime::new().with_executor(executor);
//! //
//! // Spawn the task — it's pure data:
//! // let task = CompletionTask { model: "gpt-4o".into(), system: None };
//! // let result = runtime.run_dyn(erase_io(task), Box::new("Say hello".to_string())).await;
//! # }
//! ```

use std::any::Any;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

use nanites_core::SerializableTask;
use nanites_core::dyn_task::{AnyInput, AnyOutput, IoTask};
use nanites_core::error::BoxError;
use nanites_core::executor::TaskExecutor;
use nanites_core::{Ctx, TaskRegistry};
use rig::completion::CompletionModel;
use serde::{Deserialize, Serialize};

// ─── Error ────────────────────────────────────────────────────────────────────

/// Error returned by nanites-rig executors.
#[derive(Debug, thiserror::Error)]
pub enum RigTaskError {
    /// The executor has no model registered under the requested name.
    #[error("no model registered with name {0:?}")]
    ModelNotFound(String),

    /// The rig completion API returned an error.
    #[error("completion error: {0}")]
    Completion(#[from] rig::completion::CompletionError),
}

// ─── CompletionTask ───────────────────────────────────────────────────────────

/// A serializable task that represents a single, stateless LLM completion call.
///
/// This is a **pure data struct** — it carries only the model name and an
/// optional system prompt, not a model handle. Execution is delegated to a
/// [`RigCompletionExecutor`] registered with the runtime.
///
/// Input: `String` (the user prompt).
/// Output: `String` (the assistant's reply).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionTask {
    /// Model identifier (e.g. `"gpt-4o"`, `"claude-3-5-haiku"`).
    pub model: String,
    /// Optional system / preamble prompt.
    pub system: Option<String>,
}

impl IoTask for CompletionTask {
    type Input = String;
    type Output = String;
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
/// Like [`CompletionTask`], this is a pure data struct — register a
/// [`RigChatExecutor`] with the runtime to execute it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatTask {
    /// Model identifier.
    pub model: String,
    /// Optional system / preamble prompt.
    pub system: Option<String>,
}

impl IoTask for ChatTask {
    type Input = Vec<ChatMessage>;
    type Output = String;
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

// ─── RigCompletionExecutor ────────────────────────────────────────────────────

/// Executor for [`CompletionTask`] — holds a map of model name → model instance.
///
/// # Example
///
/// ```no_run
/// use nanites_rig::RigCompletionExecutor;
///
/// // let openai = rig::providers::openai::Client::from_env();
/// // let model = openai.completion_model(rig::providers::openai::GPT_4O);
/// // let executor = RigCompletionExecutor::new().with_model("gpt-4o", model);
/// ```
pub struct RigCompletionExecutor {
    models: HashMap<String, Arc<dyn ErasedCompletionModel>>,
}

impl RigCompletionExecutor {
    /// Create an executor with no models registered.
    pub fn new() -> Self {
        Self {
            models: HashMap::new(),
        }
    }

    /// Register a model under the given name.
    ///
    /// The name must match the `model` field of [`CompletionTask`] structs that
    /// this executor should handle.
    pub fn with_model<M>(mut self, name: impl Into<String>, model: M) -> Self
    where
        M: CompletionModel + Send + Sync + 'static,
    {
        self.models
            .insert(name.into(), Arc::new(TypedCompletionModel(model)));
        self
    }

    /// Register a model in place.
    pub fn register_model<M>(&mut self, name: impl Into<String>, model: M)
    where
        M: CompletionModel + Send + Sync + 'static,
    {
        self.models
            .insert(name.into(), Arc::new(TypedCompletionModel(model)));
    }
}

impl Default for RigCompletionExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl TaskExecutor for RigCompletionExecutor {
    fn task_type_name(&self) -> &'static str {
        std::any::type_name::<CompletionTask>()
    }

    fn execute_erased<'a>(
        &'a self,
        task_data: &'a dyn Any,
        input: AnyInput,
        _ctx: &'a Ctx,
    ) -> Pin<Box<dyn futures::Future<Output = Result<AnyOutput, BoxError>> + Send + 'a>> {
        let task = match task_data.downcast_ref::<CompletionTask>() {
            Some(t) => t,
            None => {
                return Box::pin(async move {
                    Err(Box::new(RigTaskError::ModelNotFound(
                        "task_data is not a CompletionTask".into(),
                    )) as BoxError)
                });
            }
        };

        let prompt = match input.downcast::<String>() {
            Ok(p) => *p,
            Err(_) => {
                return Box::pin(async move {
                    Err(
                        Box::new(RigTaskError::ModelNotFound("input is not a String".into()))
                            as BoxError,
                    )
                });
            }
        };

        let model = self.models.get(&task.model).cloned();
        let model_name = task.model.clone();
        let system = task.system.clone();

        Box::pin(async move {
            let model = model
                .ok_or_else(|| Box::new(RigTaskError::ModelNotFound(model_name)) as BoxError)?;
            let result = model.complete(system.as_deref(), prompt).await?;
            Ok(Box::new(result) as AnyOutput)
        })
    }
}

// ─── RigChatExecutor ──────────────────────────────────────────────────────────

/// Executor for [`ChatTask`] — holds a map of model name → model instance.
pub struct RigChatExecutor {
    models: HashMap<String, Arc<dyn ErasedCompletionModel>>,
}

impl RigChatExecutor {
    /// Create an executor with no models registered.
    pub fn new() -> Self {
        Self {
            models: HashMap::new(),
        }
    }

    /// Register a model under the given name.
    pub fn with_model<M>(mut self, name: impl Into<String>, model: M) -> Self
    where
        M: CompletionModel + Send + Sync + 'static,
    {
        self.models
            .insert(name.into(), Arc::new(TypedCompletionModel(model)));
        self
    }

    /// Register a model in place.
    pub fn register_model<M>(&mut self, name: impl Into<String>, model: M)
    where
        M: CompletionModel + Send + Sync + 'static,
    {
        self.models
            .insert(name.into(), Arc::new(TypedCompletionModel(model)));
    }
}

impl Default for RigChatExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl TaskExecutor for RigChatExecutor {
    fn task_type_name(&self) -> &'static str {
        std::any::type_name::<ChatTask>()
    }

    fn execute_erased<'a>(
        &'a self,
        task_data: &'a dyn Any,
        input: AnyInput,
        _ctx: &'a Ctx,
    ) -> Pin<Box<dyn futures::Future<Output = Result<AnyOutput, BoxError>> + Send + 'a>> {
        let task = match task_data.downcast_ref::<ChatTask>() {
            Some(t) => t,
            None => {
                return Box::pin(async move {
                    Err(Box::new(RigTaskError::ModelNotFound(
                        "task_data is not a ChatTask".into(),
                    )) as BoxError)
                });
            }
        };

        // Build the full prompt string from conversation history.
        let messages = match input.downcast::<Vec<ChatMessage>>() {
            Ok(m) => *m,
            Err(_) => {
                return Box::pin(async move {
                    Err(Box::new(RigTaskError::ModelNotFound(
                        "input is not Vec<ChatMessage>".into(),
                    )) as BoxError)
                });
            }
        };

        // Flatten conversation history into a single prompt string.
        // rig's low-level completion API takes a single prompt; callers who need
        // true multi-turn support should use rig's chat builder directly.
        let prompt = messages
            .iter()
            .map(|m| format!("{}: {}", m.role, m.content))
            .collect::<Vec<_>>()
            .join("\n");

        let model = self.models.get(&task.model).cloned();
        let model_name = task.model.clone();
        let system = task.system.clone();

        Box::pin(async move {
            let model = model
                .ok_or_else(|| Box::new(RigTaskError::ModelNotFound(model_name)) as BoxError)?;
            let result = model.complete(system.as_deref(), prompt).await?;
            Ok(Box::new(result) as AnyOutput)
        })
    }
}

// ─── Internal: type-erased model ─────────────────────────────────────────────

/// Object-safe wrapper around `CompletionModel`.
///
/// `CompletionModel` is not object-safe (generic methods), so we wrap it in a
/// concrete struct and expose only the operation we need.
trait ErasedCompletionModel: Send + Sync {
    fn complete<'a>(
        &'a self,
        system: Option<&'a str>,
        prompt: String,
    ) -> Pin<Box<dyn futures::Future<Output = Result<String, BoxError>> + Send + 'a>>;
}

struct TypedCompletionModel<M>(M);

impl<M: CompletionModel + Send + Sync> ErasedCompletionModel for TypedCompletionModel<M> {
    fn complete<'a>(
        &'a self,
        system: Option<&'a str>,
        prompt: String,
    ) -> Pin<Box<dyn futures::Future<Output = Result<String, BoxError>> + Send + 'a>> {
        Box::pin(async move {
            let text = complete(&self.0, system, prompt).await?;
            Ok(text)
        })
    }
}

// ─── Registry ─────────────────────────────────────────────────────────────────

/// Register all nanites-rig task types with the given registry.
///
/// This registers the type names so the registry can reconstruct tasks from
/// JSON snapshots. To actually execute them, register a [`RigCompletionExecutor`]
/// and/or [`RigChatExecutor`] with the runtime.
///
/// Registered type names:
/// - `"nanites_rig::CompletionTask"`
/// - `"nanites_rig::ChatTask"`
pub fn register_all(registry: &mut TaskRegistry) {
    use nanites_core::dyn_task::erase_io;
    registry.register_factory("nanites_rig::CompletionTask", |params| {
        let task: CompletionTask = serde_json::from_value(params)
            .map_err(|e| nanites_core::RegistryError::DeserializationFailed(e.to_string()))?;
        Ok(erase_io(task))
    });
    registry.register_factory("nanites_rig::ChatTask", |params| {
        let task: ChatTask = serde_json::from_value(params)
            .map_err(|e| nanites_core::RegistryError::DeserializationFailed(e.to_string()))?;
        Ok(erase_io(task))
    });
}

// ─── Standalone helpers ────────────────────────────────────────────────────────

/// One-shot LLM completion. Fresh context, no history, no accumulation.
///
/// This is the direct bridge to rig's completion API — useful when you have a
/// concrete [`CompletionModel`] in hand and don't need the Task substrate.
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
