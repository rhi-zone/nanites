//! chat.rs — CLI loop demonstrating the nanites-chat orchestration layer.
//!
//! Runs a real multi-turn conversation with a hardcoded character. History is
//! capped at the last 10 entries; context is rebuilt fresh on every turn.
//!
//! # Running
//!
//! Set one of:
//!   OPENAI_API_KEY=sk-...     (uses gpt-4o-mini)
//!   ANTHROPIC_API_KEY=sk-...  (uses claude-3-5-haiku-20241022)
//!
//! Then:
//!   cargo run --example chat
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
    // ── Detect provider from environment ─────────────────────────────────────

    let (provider, model_name) = detect_provider();

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

// ─── Provider detection ────────────────────────────────────────────────────────

fn detect_provider() -> (String, String) {
    if std::env::var("ANTHROPIC_API_KEY").is_ok() {
        ("anthropic".into(), "claude-3-5-haiku-20241022".into())
    } else if std::env::var("OPENAI_API_KEY").is_ok() {
        ("openai".into(), "gpt-4o-mini".into())
    } else {
        eprintln!(
            "error: no API key found.\n\
             Set OPENAI_API_KEY or ANTHROPIC_API_KEY and try again."
        );
        std::process::exit(1);
    }
}

// ─── Executor construction ─────────────────────────────────────────────────────

fn build_executor(provider: &str, model_name: &str) -> RigCompletionExecutor {
    match provider {
        "anthropic" => {
            let client = rig::providers::anthropic::Client::from_env();
            let model = client.completion_model(model_name);
            RigCompletionExecutor::new().with_model(model_name, model)
        }
        "openai" => {
            let client = rig::providers::openai::Client::from_env();
            let model = client.completion_model(model_name);
            RigCompletionExecutor::new().with_model(model_name, model)
        }
        other => {
            eprintln!("unknown provider: {other}");
            std::process::exit(1);
        }
    }
}
