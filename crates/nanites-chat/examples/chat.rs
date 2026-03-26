//! chat.rs — CLI loop demonstrating the nanites-chat orchestration layer.
//!
//! Runs a real multi-turn conversation with a hardcoded character. History is
//! capped at the last 10 entries; context is rebuilt fresh on every turn.
//!
//! # Running
//!
//!   cargo run --example chat -p nanites-chat -- provider:model
//!
//! Examples:
//!   cargo run --example chat -p nanites-chat -- anthropic:claude-3-5-haiku-20241022
//!   cargo run --example chat -p nanites-chat -- openai:gpt-4o-mini
//!   cargo run --example chat -p nanites-chat -- cohere:command-r
//!   cargo run --example chat -p nanites-chat -- gemini:gemini-1.5-flash
//!   cargo run --example chat -p nanites-chat -- perplexity:llama-3.1-sonar-small-128k-online
//!   cargo run --example chat -p nanites-chat -- azure:gpt-4o
//!   cargo run --example chat -p nanites-chat -- deepseek:deepseek-chat
//!   cargo run --example chat -p nanites-chat -- galadriel:llama3.1-70b
//!   cargo run --example chat -p nanites-chat -- groq:llama-3.3-70b-versatile
//!   cargo run --example chat -p nanites-chat -- huggingface:Qwen/Qwen2.5-72B-Instruct
//!   cargo run --example chat -p nanites-chat -- hyperbolic:meta-llama/Meta-Llama-3.1-70B-Instruct
//!   cargo run --example chat -p nanites-chat -- mira:claude-3-5-sonnet-20241022
//!   cargo run --example chat -p nanites-chat -- mistral:mistral-small-latest
//!   cargo run --example chat -p nanites-chat -- moonshot:moonshot-v1-8k
//!   cargo run --example chat -p nanites-chat -- ollama:llama3.2
//!   cargo run --example chat -p nanites-chat -- openrouter:openai/gpt-4o-mini
//!   cargo run --example chat -p nanites-chat -- together:meta-llama/Meta-Llama-3.1-8B-Instruct-Turbo
//!   cargo run --example chat -p nanites-chat -- xai:grok-beta
//!
//! Required environment variables per provider:
//!   anthropic   → ANTHROPIC_API_KEY
//!   openai      → OPENAI_API_KEY
//!   cohere      → COHERE_API_KEY
//!   gemini      → GEMINI_API_KEY
//!   perplexity  → PERPLEXITY_API_KEY
//!   azure       → AZURE_API_KEY (or AZURE_TOKEN), AZURE_API_VERSION, AZURE_ENDPOINT
//!   deepseek    → DEEPSEEK_API_KEY
//!   galadriel   → GALADRIEL_API_KEY
//!   groq        → GROQ_API_KEY
//!   huggingface → HUGGINGFACE_API_KEY
//!   hyperbolic  → HYPERBOLIC_API_KEY
//!   mira        → MIRA_API_KEY
//!   mistral     → MISTRAL_API_KEY
//!   moonshot    → MOONSHOT_API_KEY
//!   ollama      → (none — connects to http://localhost:11434)
//!   openrouter  → OPENROUTER_API_KEY
//!   together    → TOGETHER_API_KEY
//!   xai         → XAI_API_KEY
//!
//! Type a message and press Enter. Ctrl-D or Ctrl-C to quit.

use std::io::{self, BufRead, Write as IoWrite};

use nanites_chat::{CharacterState, HandleMessageTask, HistoryEntry};
use nanites_core::Runtime;
use nanites_rig::RigCompletionExecutor;
use rig::client::{CompletionClient as _, ProviderClient as _};

const MAX_HISTORY: usize = 10;

#[tokio::main]
async fn main() {
    // ── Parse provider:model from argv ───────────────────────────────────────

    let arg = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: cargo run --example chat -p nanites-chat -- provider:model");
        eprintln!();
        eprintln!(
            "providers: anthropic, azure, cohere, deepseek, galadriel, gemini, groq, \
             huggingface, hyperbolic, mira, mistral, moonshot, ollama, openai, openrouter, \
             perplexity, together, xai"
        );
        eprintln!();
        eprintln!("examples:");
        eprintln!("  anthropic:claude-3-5-haiku-20241022");
        eprintln!("  openai:gpt-4o-mini");
        eprintln!("  cohere:command-r");
        eprintln!("  gemini:gemini-1.5-flash");
        eprintln!("  perplexity:llama-3.1-sonar-small-128k-online");
        eprintln!("  groq:llama-3.3-70b-versatile");
        eprintln!("  mistral:mistral-small-latest");
        eprintln!("  ollama:llama3.2");
        std::process::exit(1);
    });

    let (provider, model_name) = parse_provider_model(&arg);

    eprintln!("nanites-chat example");
    eprintln!("Provider: {provider}  Model: {model_name}");
    eprintln!("Type a message and press Enter. Ctrl-D to quit.\n");

    // ── Build the rig model and register it with the runtime ──────────────────

    let executor = build_executor(&provider, &model_name);
    let runtime = Runtime::new().with_executor(executor);

    // ── Hardcoded character ───────────────────────────────────────────────────

    let character = CharacterState {
        name: "Mira".into(),
        persona: "You are Mira, a curious and direct research companion. \
                  You think out loud, ask good questions, and give honest, \
                  concise answers. You avoid filler phrases and get to the point."
            .into(),
        current_knowledge: vec![
            "This is a proof-of-concept for the nanites orchestration substrate.".into(),
            "Each turn constructs a fresh context — no accumulated context poisoning.".into(),
        ],
    };

    // ── Chat loop ─────────────────────────────────────────────────────────────

    let mut history: Vec<HistoryEntry> = Vec::new();
    let stdin = io::stdin();

    loop {
        // Prompt.
        eprint!("you> ");
        io::stderr().flush().ok();

        // Read user input.
        let mut line = String::new();
        match stdin.lock().read_line(&mut line) {
            Ok(0) => break, // EOF
            Ok(_) => {}
            Err(e) => {
                eprintln!("read error: {e}");
                break;
            }
        }

        let user_message = line.trim().to_string();
        if user_message.is_empty() {
            continue;
        }

        // Select recent history (last MAX_HISTORY entries).
        let recent: Vec<HistoryEntry> = history
            .iter()
            .rev()
            .take(MAX_HISTORY)
            .rev()
            .cloned()
            .collect();

        // Run one turn.
        let task = HandleMessageTask {
            model: model_name.clone(),
            character: character.clone(),
            history: recent,
        };

        match runtime.run(task, user_message.clone()).await {
            Ok(response) => {
                println!("mira> {response}\n");

                // Accumulate history externally.
                history.push(HistoryEntry {
                    role: "user".into(),
                    content: user_message,
                });
                history.push(HistoryEntry {
                    role: "assistant".into(),
                    content: response,
                });
            }
            Err(e) => {
                eprintln!("error: {e}");
                break;
            }
        }
    }

    eprintln!("\ngoodbye.");
}

// ─── Argument parsing ──────────────────────────────────────────────────────────

fn parse_provider_model(arg: &str) -> (String, String) {
    match arg.split_once(':') {
        Some((provider, model)) if !provider.is_empty() && !model.is_empty() => {
            (provider.to_owned(), model.to_owned())
        }
        _ => {
            eprintln!(
                "error: argument must be in the form provider:model (e.g. anthropic:claude-3-5-haiku-20241022)"
            );
            std::process::exit(1);
        }
    }
}

// ─── Executor construction ─────────────────────────────────────────────────────

fn build_executor(provider: &str, model_name: &str) -> RigCompletionExecutor {
    match provider {
        "anthropic" => {
            let api_key = require_env("ANTHROPIC_API_KEY", provider);
            let client = rig::providers::anthropic::Client::new(&api_key)
                .unwrap_or_else(|e| fatal_client_error(provider, e));
            let model = client.completion_model(model_name);
            RigCompletionExecutor::new().with_model(model_name, model)
        }
        "azure" => {
            // Requires AZURE_API_KEY (or AZURE_TOKEN), AZURE_API_VERSION, AZURE_ENDPOINT.
            let client = rig::providers::azure::Client::from_env();
            let model = client.completion_model(model_name);
            RigCompletionExecutor::new().with_model(model_name, model)
        }
        "cohere" => {
            let api_key = require_env("COHERE_API_KEY", provider);
            let client = rig::providers::cohere::Client::new(&api_key)
                .unwrap_or_else(|e| fatal_client_error(provider, e));
            let model = client.completion_model(model_name);
            RigCompletionExecutor::new().with_model(model_name, model)
        }
        "deepseek" => {
            let api_key = require_env("DEEPSEEK_API_KEY", provider);
            let client = rig::providers::deepseek::Client::new(&api_key)
                .unwrap_or_else(|e| fatal_client_error(provider, e));
            let model = client.completion_model(model_name);
            RigCompletionExecutor::new().with_model(model_name, model)
        }
        "galadriel" => {
            let api_key = require_env("GALADRIEL_API_KEY", provider);
            let client = rig::providers::galadriel::Client::new(&api_key)
                .unwrap_or_else(|e| fatal_client_error(provider, e));
            let model = client.completion_model(model_name);
            RigCompletionExecutor::new().with_model(model_name, model)
        }
        "gemini" => {
            let api_key = require_env("GEMINI_API_KEY", provider);
            let client = rig::providers::gemini::Client::new(&api_key)
                .unwrap_or_else(|e| fatal_client_error(provider, e));
            let model = client.completion_model(model_name);
            RigCompletionExecutor::new().with_model(model_name, model)
        }
        "groq" => {
            let api_key = require_env("GROQ_API_KEY", provider);
            let client = rig::providers::groq::Client::new(&api_key)
                .unwrap_or_else(|e| fatal_client_error(provider, e));
            let model = client.completion_model(model_name);
            RigCompletionExecutor::new().with_model(model_name, model)
        }
        "huggingface" => {
            let api_key = require_env("HUGGINGFACE_API_KEY", provider);
            let client = rig::providers::huggingface::Client::new(&api_key)
                .unwrap_or_else(|e| fatal_client_error(provider, e));
            let model = client.completion_model(model_name);
            RigCompletionExecutor::new().with_model(model_name, model)
        }
        "hyperbolic" => {
            let api_key = require_env("HYPERBOLIC_API_KEY", provider);
            let client = rig::providers::hyperbolic::Client::new(&api_key)
                .unwrap_or_else(|e| fatal_client_error(provider, e));
            let model = client.completion_model(model_name);
            RigCompletionExecutor::new().with_model(model_name, model)
        }
        "mira" => {
            let api_key = require_env("MIRA_API_KEY", provider);
            let client = rig::providers::mira::Client::new(&api_key)
                .unwrap_or_else(|e| fatal_client_error(provider, e));
            let model = client.completion_model(model_name);
            RigCompletionExecutor::new().with_model(model_name, model)
        }
        "mistral" => {
            let api_key = require_env("MISTRAL_API_KEY", provider);
            let client = rig::providers::mistral::Client::new(&api_key)
                .unwrap_or_else(|e| fatal_client_error(provider, e));
            let model = client.completion_model(model_name);
            RigCompletionExecutor::new().with_model(model_name, model)
        }
        "moonshot" => {
            let api_key = require_env("MOONSHOT_API_KEY", provider);
            let client = rig::providers::moonshot::Client::new(&api_key)
                .unwrap_or_else(|e| fatal_client_error(provider, e));
            let model = client.completion_model(model_name);
            RigCompletionExecutor::new().with_model(model_name, model)
        }
        "ollama" => {
            // No API key — connects to http://localhost:11434 by default.
            let client = rig::providers::ollama::Client::new(rig::client::Nothing)
                .unwrap_or_else(|e| fatal_client_error(provider, e));
            let model = client.completion_model(model_name);
            RigCompletionExecutor::new().with_model(model_name, model)
        }
        "openai" => {
            let api_key = require_env("OPENAI_API_KEY", provider);
            let client = rig::providers::openai::Client::new(&api_key)
                .unwrap_or_else(|e| fatal_client_error(provider, e));
            let model = client.completion_model(model_name);
            RigCompletionExecutor::new().with_model(model_name, model)
        }
        "openrouter" => {
            let api_key = require_env("OPENROUTER_API_KEY", provider);
            let client = rig::providers::openrouter::Client::new(&api_key)
                .unwrap_or_else(|e| fatal_client_error(provider, e));
            let model = client.completion_model(model_name);
            RigCompletionExecutor::new().with_model(model_name, model)
        }
        "perplexity" => {
            let api_key = require_env("PERPLEXITY_API_KEY", provider);
            let client = rig::providers::perplexity::Client::new(&api_key)
                .unwrap_or_else(|e| fatal_client_error(provider, e));
            let model = client.completion_model(model_name);
            RigCompletionExecutor::new().with_model(model_name, model)
        }
        "together" => {
            let api_key = require_env("TOGETHER_API_KEY", provider);
            let client = rig::providers::together::Client::new(&api_key)
                .unwrap_or_else(|e| fatal_client_error(provider, e));
            let model = client.completion_model(model_name);
            RigCompletionExecutor::new().with_model(model_name, model)
        }
        "xai" => {
            let api_key = require_env("XAI_API_KEY", provider);
            let client = rig::providers::xai::Client::new(&api_key)
                .unwrap_or_else(|e| fatal_client_error(provider, e));
            let model = client.completion_model(model_name);
            RigCompletionExecutor::new().with_model(model_name, model)
        }
        other => {
            eprintln!(
                "error: unknown provider {other:?}\n\
                 supported providers: anthropic, azure, cohere, deepseek, galadriel, gemini, \
                 groq, huggingface, hyperbolic, mira, mistral, moonshot, ollama, openai, \
                 openrouter, perplexity, together, xai"
            );
            std::process::exit(1);
        }
    }
}

// ─── Helpers ───────────────────────────────────────────────────────────────────

fn require_env(var: &str, provider: &str) -> String {
    std::env::var(var).unwrap_or_else(|_| {
        eprintln!("error: {var} is not set (required for provider {provider:?})");
        std::process::exit(1);
    })
}

fn fatal_client_error<T>(provider: &str, e: impl std::fmt::Display) -> T {
    eprintln!("error: failed to build {provider:?} client: {e}");
    std::process::exit(1);
}
