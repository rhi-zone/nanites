use rig::completion::CompletionModel;

use crate::turn::{Turn, TurnError};

/// An LLM-backed turn. Takes a prompt construction function and a completion model,
/// produces a string response.
///
/// This is the bridge between the turn primitive and rig's completion API.
/// The `build_request` function constructs a fresh `CompletionRequest` for each call —
/// no accumulated context, no conversation history.
pub struct LlmTurn<M: CompletionModel> {
    model: M,
    system: Option<String>,
}

impl<M: CompletionModel> LlmTurn<M> {
    pub fn new(model: M) -> Self {
        Self {
            model,
            system: None,
        }
    }

    pub fn with_system(mut self, system: impl Into<String>) -> Self {
        self.system = Some(system.into());
        self
    }
}

impl<M: CompletionModel + 'static> Turn<String, String> for LlmTurn<M> {
    async fn call(&self, input: String) -> Result<String, TurnError> {
        let mut builder = self.model.completion_request(input);
        if let Some(ref system) = self.system {
            builder = builder.preamble(system.clone());
        }
        let request = builder.build();
        let response = self.model.completion(request).await?;
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
}

/// An LLM-backed turn with structured input. Takes any serializable input,
/// formats it into a prompt via a user-provided function, and returns the response.
pub struct MappedLlmTurn<M: CompletionModel, I, F> {
    inner: LlmTurn<M>,
    format_input: F,
    _phantom: std::marker::PhantomData<I>,
}

impl<M: CompletionModel, I, F> MappedLlmTurn<M, I, F>
where
    F: Fn(I) -> String + Send + Sync,
{
    pub fn new(model: M, format_input: F) -> Self {
        Self {
            inner: LlmTurn::new(model),
            format_input,
            _phantom: std::marker::PhantomData,
        }
    }

    pub fn with_system(mut self, system: impl Into<String>) -> Self {
        self.inner = self.inner.with_system(system);
        self
    }
}

impl<M, I, F> Turn<I, String> for MappedLlmTurn<M, I, F>
where
    M: CompletionModel + 'static,
    I: Send + Sync + 'static,
    F: Fn(I) -> String + Send + Sync,
{
    async fn call(&self, input: I) -> Result<String, TurnError> {
        let prompt = (self.format_input)(input);
        self.inner.call(prompt).await
    }
}
