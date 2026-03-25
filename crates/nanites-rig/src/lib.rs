use rig::completion::CompletionModel;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("completion error: {0}")]
    Completion(#[from] rig::completion::CompletionError),
    #[error("{0}")]
    Other(Box<dyn std::error::Error + Send + Sync>),
}

/// One-shot LLM completion. Fresh context, no history, no accumulation.
///
/// This is the bridge between the function primitive and rig's completion API.
/// Each call constructs a fresh request — no conversation, no session.
pub async fn complete(
    model: &impl CompletionModel,
    system: Option<&str>,
    prompt: impl Into<String>,
) -> Result<String, Error> {
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
