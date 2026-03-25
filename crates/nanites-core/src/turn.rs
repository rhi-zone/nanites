use std::future::Future;

#[derive(Debug, thiserror::Error)]
pub enum TurnError {
    #[error("completion error: {0}")]
    Completion(#[from] rig::completion::CompletionError),
    #[error("prompt error: {0}")]
    Prompt(#[from] rig::completion::PromptError),
    #[error("{0}")]
    Other(Box<dyn std::error::Error + Send + Sync>),
}

/// The fundamental primitive: a stateless typed transformation.
///
/// A turn takes an input of type `I` and produces an output of type `O`.
/// It has no identity, no accumulated context, no session.
/// The orchestration program constructs the input, calls the turn, receives the output.
///
/// Implementations include:
/// - LLM calls (via rig)
/// - Deterministic functions
/// - Database lookups
/// - Any callable that transforms `I -> O`
pub trait Turn<I, O>: Send + Sync {
    fn call(&self, input: I) -> impl Future<Output = Result<O, TurnError>> + Send;
}

// Blanket impl: any async Fn(I) -> Result<O, TurnError> is a Turn.
impl<I, O, F, Fut> Turn<I, O> for F
where
    I: Send + 'static,
    O: Send + 'static,
    F: Fn(I) -> Fut + Send + Sync,
    Fut: Future<Output = Result<O, TurnError>> + Send,
{
    fn call(&self, input: I) -> impl Future<Output = Result<O, TurnError>> + Send {
        (self)(input)
    }
}

/// Call a turn. Convenience function for `turn.call(input)`.
pub async fn call<I, O>(turn: &impl Turn<I, O>, input: I) -> Result<O, TurnError> {
    turn.call(input).await
}

/// Create a turn from a closure.
pub fn from_fn<I, O, F, Fut>(f: F) -> F
where
    I: Send + 'static,
    O: Send + 'static,
    F: Fn(I) -> Fut + Send + Sync,
    Fut: Future<Output = Result<O, TurnError>> + Send,
{
    f
}
