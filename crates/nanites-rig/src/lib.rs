//! nanites-rig — LLM completion, embedding, and RAG tasks for the nanites
//! orchestration runtime.
//!
//! # Feature flags
//!
//! | Feature | Default | Description |
//! |---------|---------|-------------|
//! | `completions` | yes | [`CompletionTask`], [`ChatTask`], [`StructuredCompletionTask`] |
//! | `embeddings` | yes | [`EmbeddingTask`], [`RigEmbeddingExecutor`] |
//! | `rag` | yes | [`VectorStoreTask`], [`RetrieveTask`], [`RigVectorStoreExecutor`] |
//!
//! # Design
//!
//! Task structs are **pure data** — they carry configuration but no resource
//! handles. Executing them requires a matching executor registered with the
//! runtime.
//!
//! Executor overview:
//!
//! | Task | Executor |
//! |------|----------|
//! | [`CompletionTask`] | [`RigCompletionExecutor`] |
//! | [`ChatTask`] | [`RigChatExecutor`] |
//! | [`StructuredCompletionTask<T>`] | [`RigStructuredExecutor<T>`] |
//! | [`EmbeddingTask`] | [`RigEmbeddingExecutor`] |
//! | [`VectorStoreTask`] | [`RigVectorStoreExecutor`] |
//!
//! [`RetrieveTask`] is a pure [`nanites_core::Task`] that orchestrates
//! embedding + vector search end-to-end; register both executors and spawn
//! child tasks via the runtime context it receives at run time.

use nanites_core::TaskRegistry;

#[cfg(any(feature = "completions", feature = "embeddings", feature = "rag"))]
use serde::{Deserialize, Serialize};

#[cfg(any(feature = "completions", feature = "embeddings", feature = "rag"))]
use {
    nanites_core::Ctx,
    nanites_core::dyn_task::{AnyInput, AnyOutput, IoTask},
    nanites_core::error::BoxError,
    nanites_core::executor::TaskExecutor,
    std::any::Any,
    std::collections::HashMap,
    std::pin::Pin,
    std::sync::Arc,
};

#[cfg(feature = "completions")]
use {
    nanites_core::SerializableTask,
    rig::agent::AgentBuilder,
    rig::completion::{CompletionModel, TypedPrompt},
    schemars::JsonSchema,
    serde::de::DeserializeOwned,
    std::marker::PhantomData,
};

#[cfg(feature = "embeddings")]
use rig::embeddings::EmbeddingModel;

// ─── Error ────────────────────────────────────────────────────────────────────

/// Error returned by nanites-rig executors.
#[derive(Debug, thiserror::Error)]
pub enum RigTaskError {
    /// The executor has no model registered under the requested name.
    #[error("no model registered with name {0:?}")]
    ModelNotFound(String),

    /// The executor has no collection registered under the requested name.
    #[error("no vector store collection registered with name {0:?}")]
    CollectionNotFound(String),

    #[cfg(feature = "completions")]
    /// The rig completion API returned an error.
    #[error("completion error: {0}")]
    Completion(#[from] rig::completion::CompletionError),

    #[cfg(feature = "completions")]
    /// The structured output prompt failed.
    #[error("structured output error: {0}")]
    StructuredOutput(#[from] rig::completion::StructuredOutputError),

    #[cfg(feature = "embeddings")]
    /// The embedding API returned an error.
    #[error("embedding error: {0}")]
    Embedding(#[from] rig::embeddings::EmbeddingError),

    /// Provider not supported.
    #[error("unsupported provider: {0:?}")]
    UnsupportedProvider(String),
}

// ─── CompletionTask ───────────────────────────────────────────────────────────

#[cfg(feature = "completions")]
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

#[cfg(feature = "completions")]
impl IoTask for CompletionTask {
    type Input = String;
    type Output = String;
}

#[cfg(feature = "completions")]
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

#[cfg(feature = "completions")]
/// A single message in a multi-turn conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    /// `"user"` or `"assistant"`.
    pub role: String,
    /// Message body.
    pub content: String,
}

#[cfg(feature = "completions")]
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

#[cfg(feature = "completions")]
impl IoTask for ChatTask {
    type Input = Vec<ChatMessage>;
    type Output = String;
}

#[cfg(feature = "completions")]
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

#[cfg(feature = "completions")]
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

#[cfg(feature = "completions")]
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

#[cfg(feature = "completions")]
impl Default for RigCompletionExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "completions")]
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

#[cfg(feature = "completions")]
/// Executor for [`ChatTask`] — holds a map of model name → model instance.
pub struct RigChatExecutor {
    models: HashMap<String, Arc<dyn ErasedCompletionModel>>,
}

#[cfg(feature = "completions")]
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

#[cfg(feature = "completions")]
impl Default for RigChatExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "completions")]
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

// ─── Internal: type-erased completion model ───────────────────────────────────

#[cfg(feature = "completions")]
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

#[cfg(feature = "completions")]
struct TypedCompletionModel<M>(M);

#[cfg(feature = "completions")]
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
/// JSON snapshots. To actually execute them, register the appropriate executors
/// with the runtime.
///
/// With default features, registered type names include:
/// - `"nanites_rig::CompletionTask"`
/// - `"nanites_rig::ChatTask"`
/// - `"nanites_rig::EmbeddingTask"`
/// - `"nanites_rig::VectorStoreTask"`
pub fn register_all(#[allow(unused_variables)] registry: &mut TaskRegistry) {
    #[cfg(any(feature = "completions", feature = "embeddings", feature = "rag"))]
    use nanites_core::dyn_task::erase_io;

    #[cfg(feature = "completions")]
    {
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

    #[cfg(feature = "embeddings")]
    {
        registry.register_factory("nanites_rig::EmbeddingTask", |params| {
            let task: EmbeddingTask = serde_json::from_value(params)
                .map_err(|e| nanites_core::RegistryError::DeserializationFailed(e.to_string()))?;
            Ok(erase_io(task))
        });
    }

    #[cfg(feature = "rag")]
    {
        registry.register_factory("nanites_rig::VectorStoreTask", |params| {
            let task: VectorStoreTask = serde_json::from_value(params)
                .map_err(|e| nanites_core::RegistryError::DeserializationFailed(e.to_string()))?;
            Ok(erase_io(task))
        });
    }
}

// ─── StructuredCompletionTask ─────────────────────────────────────────────────

#[cfg(feature = "completions")]
/// A serializable task that requests a **typed, structured** LLM completion.
///
/// Like [`CompletionTask`], this is a pure data struct — it carries the model
/// name and an optional system prompt. Execution is delegated to a
/// [`RigStructuredExecutor<T>`] registered with the runtime.
///
/// The type parameter `T` is the output schema. It is not stored in the struct
/// (hence `#[serde(skip)]` on the phantom field); it only exists to fix the
/// executor's `IoTask::Output` associated type.
///
/// Input: `String` (the user prompt).
/// Output: `T` (deserialized from the model's structured JSON response).
///
/// # Example
///
/// ```no_run
/// use schemars::JsonSchema;
/// use serde::{Deserialize, Serialize};
/// use nanites_rig::StructuredCompletionTask;
///
/// #[derive(Debug, Deserialize, Serialize, JsonSchema)]
/// struct Sentiment { label: String, score: f64 }
///
/// let task: StructuredCompletionTask<Sentiment> = StructuredCompletionTask {
///     model: "gpt-4o".into(),
///     system: Some("Classify the sentiment.".into()),
///     _phantom: std::marker::PhantomData,
/// };
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructuredCompletionTask<T> {
    /// Model identifier (e.g. `"gpt-4o"`, `"claude-3-5-haiku"`).
    pub model: String,
    /// Optional system / preamble prompt.
    pub system: Option<String>,
    /// Phantom marker for the output type `T`.
    #[serde(skip)]
    pub _phantom: PhantomData<T>,
}

#[cfg(feature = "completions")]
impl<T> IoTask for StructuredCompletionTask<T>
where
    T: std::fmt::Debug + Clone + Send + Sync + 'static,
{
    type Input = String;
    type Output = T;
}

#[cfg(feature = "completions")]
impl<T: std::fmt::Debug + Clone + Send + Sync + 'static> SerializableTask
    for StructuredCompletionTask<T>
{
    fn serializable_type_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }

    fn params(&self) -> serde_json::Value {
        serde_json::json!({
            "model": self.model,
            "system": self.system,
        })
    }
}

// ─── RigStructuredExecutor ────────────────────────────────────────────────────

#[cfg(feature = "completions")]
/// Executor for [`StructuredCompletionTask<T>`].
///
/// Holds a map of model name → model instance. On execution it builds a
/// transient [`rig::agent::Agent`] from the stored model, then calls
/// `prompt_typed::<T>` to obtain a structured response from the LLM.
///
/// The type parameter `T` must match the `T` in the task type. Register one
/// executor per output type.
///
/// # Example
///
/// ```no_run
/// use schemars::JsonSchema;
/// use serde::{Deserialize, Serialize};
/// use nanites_rig::{RigStructuredExecutor, StructuredCompletionTask};
///
/// #[derive(Debug, Deserialize, Serialize, JsonSchema)]
/// struct Sentiment { label: String, score: f64 }
///
/// // let openai = rig::providers::openai::Client::from_env();
/// // let model = openai.completion_model("gpt-4o");
/// // let executor = RigStructuredExecutor::<Sentiment>::new().with_model("gpt-4o", model);
/// ```
pub struct RigStructuredExecutor<T> {
    models: HashMap<String, Arc<dyn ErasedStructuredModel<T>>>,
}

#[cfg(feature = "completions")]
impl<T> RigStructuredExecutor<T>
where
    T: DeserializeOwned + JsonSchema + std::fmt::Debug + Clone + Send + Sync + 'static,
{
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
        self.models.insert(
            name.into(),
            Arc::new(TypedStructuredModel(model, PhantomData)),
        );
        self
    }

    /// Register a model in place.
    pub fn register_model<M>(&mut self, name: impl Into<String>, model: M)
    where
        M: CompletionModel + Send + Sync + 'static,
    {
        self.models.insert(
            name.into(),
            Arc::new(TypedStructuredModel(model, PhantomData)),
        );
    }
}

#[cfg(feature = "completions")]
impl<T> Default for RigStructuredExecutor<T>
where
    T: DeserializeOwned + JsonSchema + std::fmt::Debug + Clone + Send + Sync + 'static,
{
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "completions")]
impl<T> TaskExecutor for RigStructuredExecutor<T>
where
    T: DeserializeOwned + JsonSchema + std::fmt::Debug + Clone + Send + Sync + 'static,
{
    fn task_type_name(&self) -> &'static str {
        std::any::type_name::<StructuredCompletionTask<T>>()
    }

    fn execute_erased<'a>(
        &'a self,
        task_data: &'a dyn Any,
        input: AnyInput,
        _ctx: &'a Ctx,
    ) -> Pin<Box<dyn futures::Future<Output = Result<AnyOutput, BoxError>> + Send + 'a>> {
        let task = match task_data.downcast_ref::<StructuredCompletionTask<T>>() {
            Some(t) => t,
            None => {
                return Box::pin(async move {
                    Err(Box::new(RigTaskError::ModelNotFound(
                        "task_data is not a StructuredCompletionTask".into(),
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
            let result = model.prompt_typed(system.as_deref(), prompt).await?;
            Ok(Box::new(result) as AnyOutput)
        })
    }
}

// ─── Internal: type-erased structured model ───────────────────────────────────

#[cfg(feature = "completions")]
/// Object-safe trait for typed structured prompt execution.
trait ErasedStructuredModel<T>: Send + Sync {
    fn prompt_typed<'a>(
        &'a self,
        system: Option<&'a str>,
        prompt: String,
    ) -> Pin<Box<dyn futures::Future<Output = Result<T, BoxError>> + Send + 'a>>;
}

#[cfg(feature = "completions")]
struct TypedStructuredModel<M, T>(M, PhantomData<T>);

#[cfg(feature = "completions")]
impl<M, T> ErasedStructuredModel<T> for TypedStructuredModel<M, T>
where
    M: CompletionModel + Send + Sync + 'static,
    T: DeserializeOwned + JsonSchema + std::fmt::Debug + Clone + Send + Sync + 'static,
{
    fn prompt_typed<'a>(
        &'a self,
        system: Option<&'a str>,
        prompt: String,
    ) -> Pin<Box<dyn futures::Future<Output = Result<T, BoxError>> + Send + 'a>> {
        // We need to clone the model reference to move it into the async block.
        // AgentBuilder takes ownership of the model, so we wrap it in Arc and
        // build a temporary agent on each call. The model is stored behind Arc
        // in the map already, but ErasedStructuredModel takes &self; we clone
        // via an inner Arc.
        let model = self.0.clone();
        let system = system.map(str::to_owned);
        Box::pin(async move {
            let mut builder = AgentBuilder::new(model);
            if let Some(s) = system {
                builder = builder.preamble(&s);
            }
            let agent = builder.build();
            let result: T = agent
                .prompt_typed(prompt)
                .await
                .map_err(|e| Box::new(RigTaskError::StructuredOutput(e)) as BoxError)?;
            Ok(result)
        })
    }
}

// ─── register_structured ──────────────────────────────────────────────────────

#[cfg(feature = "completions")]
/// Register a [`RigStructuredExecutor<T>`] with a [`TaskRegistry`].
///
/// This records a factory so the registry can reconstruct
/// [`StructuredCompletionTask<T>`] tasks from JSON snapshots. The executor
/// must be registered separately with the runtime (via
/// `runtime.register_executor(executor)`).
///
/// # Example
///
/// ```no_run
/// use nanites_core::TaskRegistry;
/// use nanites_rig::{RigStructuredExecutor, register_structured};
/// use schemars::JsonSchema;
/// use serde::{Deserialize, Serialize};
///
/// #[derive(Debug, Deserialize, Serialize, JsonSchema)]
/// struct Sentiment { label: String, score: f64 }
///
/// let mut registry = TaskRegistry::new();
/// // let executor = RigStructuredExecutor::<Sentiment>::new().with_model("gpt-4o", model);
/// // register_structured::<Sentiment>(&mut registry, &executor);
/// ```
pub fn register_structured<T>(registry: &mut TaskRegistry, _executor: &RigStructuredExecutor<T>)
where
    T: DeserializeOwned + JsonSchema + std::fmt::Debug + Clone + Send + Sync + 'static,
{
    use nanites_core::dyn_task::erase_io;
    registry.register_factory(
        std::any::type_name::<StructuredCompletionTask<T>>(),
        |params| {
            let task: StructuredCompletionTask<T> = {
                let model = params["model"]
                    .as_str()
                    .ok_or_else(|| {
                        nanites_core::RegistryError::DeserializationFailed(
                            "missing 'model' field".into(),
                        )
                    })?
                    .to_owned();
                let system = params["system"].as_str().map(str::to_owned);
                StructuredCompletionTask {
                    model,
                    system,
                    _phantom: PhantomData,
                }
            };
            Ok(erase_io(task))
        },
    );
}

// ─── Standalone completion helper ─────────────────────────────────────────────

#[cfg(feature = "completions")]
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

// ─── EmbeddingTask ────────────────────────────────────────────────────────────

#[cfg(feature = "embeddings")]
/// A serializable task that generates text embeddings.
///
/// Carries the provider name and an optional model name (uses provider default
/// if `None`). Execution is delegated to a [`RigEmbeddingExecutor`] registered
/// with the runtime.
///
/// Input: `Vec<String>` (texts to embed).
/// Output: `Vec<Vec<f32>>` (one embedding vector per input text).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingTask {
    /// Provider name: `"openai"`, `"cohere"`, `"gemini"`, `"mistral"`,
    /// `"ollama"`, `"together"`, `"voyageai"`, `"openrouter"`, `"azure"`.
    pub provider: String,
    /// Model name; `None` means use the provider's default.
    pub model: Option<String>,
}

#[cfg(feature = "embeddings")]
impl IoTask for EmbeddingTask {
    type Input = Vec<String>;
    type Output = Vec<Vec<f32>>;
}

// ─── RigEmbeddingExecutor ─────────────────────────────────────────────────────

#[cfg(feature = "embeddings")]
/// Executor for [`EmbeddingTask`].
///
/// Holds named [`ErasedEmbeddingModel`] instances keyed by `"provider/model"`.
/// When executing, it looks up the model registered for the task's provider
/// and model, calls `embed_texts`, and returns the embedding vectors as
/// `Vec<Vec<f32>>`.
///
/// Register models with [`RigEmbeddingExecutor::with_model`]:
///
/// ```no_run
/// use nanites_rig::RigEmbeddingExecutor;
///
/// // let openai = rig::providers::openai::Client::from_env();
/// // let model = openai.embedding_model("text-embedding-3-small");
/// // let executor = RigEmbeddingExecutor::new()
/// //     .with_model("openai", "text-embedding-3-small", model);
/// ```
///
/// For Cohere, the model must be registered with `embedding_model(model,
/// input_type)` (rig's Cohere client requires an `input_type` param):
///
/// ```no_run
/// use nanites_rig::RigEmbeddingExecutor;
///
/// // let cohere = rig::providers::cohere::Client::from_env();
/// // let model = cohere.embedding_model("embed-english-v3.0", "search_document");
/// // let executor = RigEmbeddingExecutor::new()
/// //     .with_model("cohere", "embed-english-v3.0", model);
/// ```
pub struct RigEmbeddingExecutor {
    /// Key: `"provider/model"` or `"provider"` for the default model.
    models: HashMap<String, Arc<dyn ErasedEmbeddingModel>>,
}

#[cfg(feature = "embeddings")]
impl RigEmbeddingExecutor {
    /// Create an executor with no models registered.
    pub fn new() -> Self {
        Self {
            models: HashMap::new(),
        }
    }

    /// Register an embedding model under the given provider and model name.
    ///
    /// The executor looks up models by `"provider/model"` when the task
    /// specifies a model, or by `"provider"` when `model` is `None`.
    pub fn with_model<M>(
        mut self,
        provider: impl Into<String>,
        model_name: impl Into<String>,
        model: M,
    ) -> Self
    where
        M: EmbeddingModel + Send + Sync + 'static,
    {
        let provider = provider.into();
        let model_name = model_name.into();
        // Register under both the full key and the provider-only key (as default).
        let full_key = format!("{provider}/{model_name}");
        let erased: Arc<dyn ErasedEmbeddingModel> = Arc::new(TypedEmbeddingModel(model));
        self.models.insert(full_key, Arc::clone(&erased));
        // Register as provider default only if not already set.
        self.models.entry(provider).or_insert(erased);
        self
    }

    /// Register an embedding model in place.
    pub fn register_model<M>(
        &mut self,
        provider: impl Into<String>,
        model_name: impl Into<String>,
        model: M,
    ) where
        M: EmbeddingModel + Send + Sync + 'static,
    {
        let provider = provider.into();
        let model_name = model_name.into();
        let full_key = format!("{provider}/{model_name}");
        let erased: Arc<dyn ErasedEmbeddingModel> = Arc::new(TypedEmbeddingModel(model));
        self.models.insert(full_key, Arc::clone(&erased));
        self.models.entry(provider).or_insert(erased);
    }

    fn resolve_model(
        &self,
        provider: &str,
        model: Option<&str>,
    ) -> Option<Arc<dyn ErasedEmbeddingModel>> {
        let key = match model {
            Some(m) => format!("{provider}/{m}"),
            None => provider.to_owned(),
        };
        self.models.get(&key).cloned()
    }
}

#[cfg(feature = "embeddings")]
impl Default for RigEmbeddingExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "embeddings")]
impl TaskExecutor for RigEmbeddingExecutor {
    fn task_type_name(&self) -> &'static str {
        std::any::type_name::<EmbeddingTask>()
    }

    fn execute_erased<'a>(
        &'a self,
        task_data: &'a dyn Any,
        input: AnyInput,
        _ctx: &'a Ctx,
    ) -> Pin<Box<dyn futures::Future<Output = Result<AnyOutput, BoxError>> + Send + 'a>> {
        let task = match task_data.downcast_ref::<EmbeddingTask>() {
            Some(t) => t,
            None => {
                return Box::pin(async move {
                    Err(Box::new(RigTaskError::ModelNotFound(
                        "task_data is not an EmbeddingTask".into(),
                    )) as BoxError)
                });
            }
        };

        let texts = match input.downcast::<Vec<String>>() {
            Ok(v) => *v,
            Err(_) => {
                return Box::pin(async move {
                    Err(Box::new(RigTaskError::ModelNotFound(
                        "input is not Vec<String>".into(),
                    )) as BoxError)
                });
            }
        };

        let model = self.resolve_model(&task.provider, task.model.as_deref());
        let provider = task.provider.clone();
        let model_key = task.model.clone();

        Box::pin(async move {
            let model = model.ok_or_else(|| {
                let key = match &model_key {
                    Some(m) => format!("{provider}/{m}"),
                    None => provider.clone(),
                };
                Box::new(RigTaskError::ModelNotFound(key)) as BoxError
            })?;
            let vecs = model.embed_texts(texts).await?;
            Ok(Box::new(vecs) as AnyOutput)
        })
    }
}

// ─── Internal: type-erased embedding model ────────────────────────────────────

#[cfg(feature = "embeddings")]
/// Object-safe wrapper around [`EmbeddingModel`].
trait ErasedEmbeddingModel: Send + Sync {
    #[allow(clippy::type_complexity)]
    fn embed_texts<'a>(
        &'a self,
        texts: Vec<String>,
    ) -> Pin<Box<dyn futures::Future<Output = Result<Vec<Vec<f32>>, BoxError>> + Send + 'a>>;
}

#[cfg(feature = "embeddings")]
struct TypedEmbeddingModel<M>(M);

#[cfg(feature = "embeddings")]
impl<M: EmbeddingModel + Send + Sync> ErasedEmbeddingModel for TypedEmbeddingModel<M> {
    fn embed_texts<'a>(
        &'a self,
        texts: Vec<String>,
    ) -> Pin<Box<dyn futures::Future<Output = Result<Vec<Vec<f32>>, BoxError>> + Send + 'a>> {
        Box::pin(async move {
            let embeddings = self
                .0
                .embed_texts(texts)
                .await
                .map_err(|e| Box::new(RigTaskError::Embedding(e)) as BoxError)?;
            let vecs = embeddings
                .into_iter()
                .map(|e| e.vec.into_iter().map(|v| v as f32).collect())
                .collect();
            Ok(vecs)
        })
    }
}

// ─── RAG layer ────────────────────────────────────────────────────────────────

#[cfg(feature = "rag")]
/// A document retrieved from a vector store search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievedDocument {
    /// Cosine similarity score (0–1, higher is more similar).
    pub score: f64,
    /// Document ID within the collection.
    pub id: String,
    /// Document content.
    pub content: String,
}

#[cfg(feature = "rag")]
/// A serializable task that searches a named in-memory vector store collection.
///
/// Input: `Vec<f32>` (the query embedding, already computed).
/// Output: `Vec<RetrievedDocument>` (top-k results by cosine similarity).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorStoreTask {
    /// The named collection within the [`RigVectorStoreExecutor`] to search.
    pub collection: String,
    /// Number of top results to return.
    pub k: usize,
}

#[cfg(feature = "rag")]
impl IoTask for VectorStoreTask {
    type Input = Vec<f32>;
    type Output = Vec<RetrievedDocument>;
}

#[cfg(feature = "rag")]
/// A serializable pure [`nanites_core::Task`] that combines embedding + search.
///
/// Drives [`EmbeddingTask`] and [`VectorStoreTask`] as sub-operations using
/// the executor infrastructure it receives via the [`Ctx`].
///
/// Input: `String` (the query text).
/// Output: `Vec<RetrievedDocument>` (top-k retrieved documents).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrieveTask {
    /// The embedding task configuration to use for query embedding.
    pub embedding_task: EmbeddingTask,
    /// The vector store search task configuration.
    pub store_task: VectorStoreTask,
}

#[cfg(feature = "rag")]
impl nanites_core::Task for RetrieveTask {
    type Input = String;
    type Output = Vec<RetrievedDocument>;
    type Error = BoxError;

    async fn run(&self, input: Self::Input, ctx: &Ctx) -> Result<Self::Output, Self::Error> {
        use nanites_core::dyn_task::erase_io;

        // Step 1: embed the query text.
        let embed_task = erase_io(self.embedding_task.clone());
        let embed_input: AnyInput = Box::new(vec![input]);
        let embed_out: AnyOutput = ctx
            .spawn_dyn(embed_task, embed_input)
            .await
            .map_err(|e| -> BoxError { Box::new(e) })?;
        let mut vecs: Vec<Vec<f32>> = *embed_out
            .downcast::<Vec<Vec<f32>>>()
            .map_err(|_| -> BoxError { "unexpected output type from EmbeddingTask".into() })?;
        let query_vec = vecs
            .pop()
            .ok_or_else(|| -> BoxError { "EmbeddingTask returned no vectors".into() })?;

        // Step 2: search the vector store.
        let store_task = erase_io(self.store_task.clone());
        let store_input: AnyInput = Box::new(query_vec);
        let store_out: AnyOutput = ctx
            .spawn_dyn(store_task, store_input)
            .await
            .map_err(|e| -> BoxError { Box::new(e) })?;
        let docs: Vec<RetrievedDocument> = *store_out
            .downcast::<Vec<RetrievedDocument>>()
            .map_err(|_| -> BoxError { "unexpected output type from VectorStoreTask".into() })?;

        Ok(docs)
    }
}

// ─── RigVectorStoreExecutor ───────────────────────────────────────────────────

#[cfg(feature = "rag")]
/// A named collection of documents stored with their embedding vectors.
///
/// Documents are `(id, content, embedding)` triples. Search is brute-force
/// cosine similarity over the stored embeddings.
#[derive(Default, Clone)]
pub struct VectorCollection {
    /// `(id, content, embedding_vec)` entries.
    documents: Vec<(String, String, Vec<f32>)>,
}

#[cfg(feature = "rag")]
impl VectorCollection {
    /// Create an empty collection.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a document with a pre-computed embedding.
    pub fn add(&mut self, id: impl Into<String>, content: impl Into<String>, embedding: Vec<f32>) {
        self.documents.push((id.into(), content.into(), embedding));
    }

    /// Search for the top-k most similar documents using cosine similarity.
    fn search(&self, query: &[f32], k: usize) -> Vec<RetrievedDocument> {
        let mut scored: Vec<(f64, &str, &str)> = self
            .documents
            .iter()
            .map(|(id, content, emb)| {
                let score = cosine_similarity(query, emb);
                (score, id.as_str(), content.as_str())
            })
            .collect();

        // Sort descending by score.
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(k);

        scored
            .into_iter()
            .map(|(score, id, content)| RetrievedDocument {
                score,
                id: id.to_owned(),
                content: content.to_owned(),
            })
            .collect()
    }
}

#[cfg(feature = "rag")]
/// Cosine similarity between two f32 slices.
fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    let dot: f64 = a
        .iter()
        .zip(b.iter())
        .map(|(&x, &y)| x as f64 * y as f64)
        .sum();
    let mag_a: f64 = a.iter().map(|&x| (x as f64).powi(2)).sum::<f64>().sqrt();
    let mag_b: f64 = b.iter().map(|&x| (x as f64).powi(2)).sum::<f64>().sqrt();
    if mag_a == 0.0 || mag_b == 0.0 {
        0.0
    } else {
        dot / (mag_a * mag_b)
    }
}

#[cfg(feature = "rag")]
/// Executor for [`VectorStoreTask`].
///
/// Holds named [`VectorCollection`] instances. On execution it looks up the
/// requested collection by name and performs a cosine-similarity search over
/// the stored embedding vectors.
///
/// Register collections with [`RigVectorStoreExecutor::with_collection`]:
///
/// ```no_run
/// use nanites_rig::{RigVectorStoreExecutor, VectorCollection};
///
/// let mut col = VectorCollection::new();
/// col.add("doc0", "Hello world", vec![0.1, 0.2, 0.3]);
///
/// let executor = RigVectorStoreExecutor::new().with_collection("my-docs", col);
/// ```
pub struct RigVectorStoreExecutor {
    collections: HashMap<String, Arc<VectorCollection>>,
}

#[cfg(feature = "rag")]
impl RigVectorStoreExecutor {
    /// Create an executor with no collections.
    pub fn new() -> Self {
        Self {
            collections: HashMap::new(),
        }
    }

    /// Register a collection under the given name.
    pub fn with_collection(mut self, name: impl Into<String>, col: VectorCollection) -> Self {
        self.collections.insert(name.into(), Arc::new(col));
        self
    }

    /// Register a collection in place.
    pub fn register_collection(&mut self, name: impl Into<String>, col: VectorCollection) {
        self.collections.insert(name.into(), Arc::new(col));
    }
}

#[cfg(feature = "rag")]
impl Default for RigVectorStoreExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "rag")]
impl TaskExecutor for RigVectorStoreExecutor {
    fn task_type_name(&self) -> &'static str {
        std::any::type_name::<VectorStoreTask>()
    }

    fn execute_erased<'a>(
        &'a self,
        task_data: &'a dyn Any,
        input: AnyInput,
        _ctx: &'a Ctx,
    ) -> Pin<Box<dyn futures::Future<Output = Result<AnyOutput, BoxError>> + Send + 'a>> {
        let task = match task_data.downcast_ref::<VectorStoreTask>() {
            Some(t) => t,
            None => {
                return Box::pin(async move {
                    Err(Box::new(RigTaskError::CollectionNotFound(
                        "task_data is not a VectorStoreTask".into(),
                    )) as BoxError)
                });
            }
        };

        let query_vec = match input.downcast::<Vec<f32>>() {
            Ok(v) => *v,
            Err(_) => {
                return Box::pin(async move {
                    Err(Box::new(RigTaskError::CollectionNotFound(
                        "input is not Vec<f32>".into(),
                    )) as BoxError)
                });
            }
        };

        let col = self.collections.get(&task.collection).cloned();
        let collection_name = task.collection.clone();
        let k = task.k;

        Box::pin(async move {
            let col = col.ok_or_else(|| {
                Box::new(RigTaskError::CollectionNotFound(collection_name)) as BoxError
            })?;
            let results = col.search(&query_vec, k);
            Ok(Box::new(results) as AnyOutput)
        })
    }
}
