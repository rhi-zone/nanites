//! chat_rag.rs — CLI demonstrating retrieval-augmented generation (RAG) with
//! the nanites-chat orchestration layer.
//!
//! Combines a completion model (for chat) with an embedding model (for query
//! and document embedding) and an in-memory vector store (for retrieval).
//! On each turn the user's query is embedded, the top-3 matching documents are
//! retrieved, and their content is injected into the character's knowledge
//! before calling the LLM.
//!
//! # Running
//!
//!   # Seed documents from a directory of .txt files, then exit:
//!   cargo run --example chat_rag -p nanites-chat -- \
//!       anthropic:claude-3-5-haiku-20241022 openai:text-embedding-3-small \
//!       --seed ./docs/
//!
//!   # Chat (reads from stdin, like chat.rs):
//!   cargo run --example chat_rag -p nanites-chat -- \
//!       anthropic:claude-3-5-haiku-20241022 openai:text-embedding-3-small
//!
//! # Arguments
//!
//! 1. `provider:model`  — completion model (same format as chat.rs)
//! 2. `provider:model`  — embedding model
//!
//! Optional flags:
//!   --seed <path>   Read all .txt files in <path>, embed them, store in the
//!                   "knowledge" collection, print a count, then exit.
//!
//! # Required environment variables
//!
//!   Completion provider:
//!     anthropic   → ANTHROPIC_API_KEY
//!     openai      → OPENAI_API_KEY
//!     cohere      → COHERE_API_KEY
//!     gemini      → GEMINI_API_KEY
//!     perplexity  → PERPLEXITY_API_KEY
//!     azure       → AZURE_API_KEY (or AZURE_TOKEN), AZURE_API_VERSION, AZURE_ENDPOINT
//!     deepseek    → DEEPSEEK_API_KEY
//!     galadriel   → GALADRIEL_API_KEY
//!     groq        → GROQ_API_KEY
//!     huggingface → HUGGINGFACE_API_KEY
//!     hyperbolic  → HYPERBOLIC_API_KEY
//!     mira        → MIRA_API_KEY
//!     mistral     → MISTRAL_API_KEY
//!     moonshot    → MOONSHOT_API_KEY
//!     ollama      → (none — connects to http://localhost:11434)
//!     openrouter  → OPENROUTER_API_KEY
//!     together    → TOGETHER_API_KEY
//!     xai         → XAI_API_KEY
//!
//!   Embedding provider:
//!     openai      → OPENAI_API_KEY
//!     cohere      → COHERE_API_KEY
//!     gemini      → GEMINI_API_KEY
//!     mistral     → MISTRAL_API_KEY
//!     ollama      → (none)
//!     together    → TOGETHER_API_KEY
//!     openrouter  → OPENROUTER_API_KEY
//!     azure       → AZURE_API_KEY (or AZURE_TOKEN), AZURE_API_VERSION, AZURE_ENDPOINT
//!
//! Type a message and press Enter. Ctrl-D or Ctrl-C to quit.

use std::io::{self, BufRead, Write as IoWrite};
use std::path::Path;

use nanites_chat::{CharacterState, HandleMessageTask, HistoryEntry};
use nanites_core::Runtime;
use nanites_rig::{
    EmbeddingTask, RetrieveTask, RetrievedDocument, RigCompletionExecutor, RigEmbeddingExecutor,
    RigVectorStoreExecutor, VectorCollection, VectorStoreTask,
};
use rig::client::{CompletionClient as _, EmbeddingsClient as _, ProviderClient as _};

const MAX_HISTORY: usize = 10;
const COLLECTION_NAME: &str = "knowledge";
const RETRIEVE_K: usize = 3;

// ─── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();

    // ── Parse arguments ──────────────────────────────────────────────────────

    if args.len() < 3 {
        usage();
    }

    let (completion_provider, completion_model) = parse_provider_model(&args[1]);
    let (embed_provider, embed_model) = parse_provider_model(&args[2]);

    // Optional --seed <path>.
    let seed_path = parse_seed_flag(&args[3..]);

    eprintln!("nanites-chat RAG example");
    eprintln!("Completion: {completion_provider}:{completion_model}");
    eprintln!("Embedding:  {embed_provider}:{embed_model}");

    // ── Build executors ──────────────────────────────────────────────────────

    let completion_executor = build_completion_executor(&completion_provider, &completion_model);
    let embedding_executor = build_embedding_executor(&embed_provider, &embed_model);

    // ── Handle --seed mode ───────────────────────────────────────────────────

    if let Some(path) = seed_path {
        seed_documents(path, &embed_provider, &embed_model, embedding_executor).await;
        return;
    }

    // ── Chat mode ────────────────────────────────────────────────────────────

    let store_executor =
        RigVectorStoreExecutor::new().with_collection(COLLECTION_NAME, VectorCollection::new());

    let runtime = Runtime::new()
        .with_executor(completion_executor)
        .with_executor(embedding_executor)
        .with_executor(store_executor);

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

    let retrieve_task = RetrieveTask {
        embedding_task: EmbeddingTask {
            provider: embed_provider.clone(),
            model: Some(embed_model.clone()),
        },
        store_task: VectorStoreTask {
            collection: COLLECTION_NAME.into(),
            k: RETRIEVE_K,
        },
    };

    eprintln!("(No documents seeded — vector store is empty. Use --seed <path> to populate it.)");
    eprintln!("Type a message and press Enter. Ctrl-D to quit.\n");

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

        // Step 1: retrieve relevant documents via RAG.
        // Chain EmbeddingTask + VectorStoreTask manually via run_dyn, since
        // RetrieveTask::Error = BoxError which can't satisfy the Error bound on
        // runtime.run (Box<dyn Error> as Error requires ?Sized, which erase
        // doesn't accept).
        let retrieved: Vec<RetrievedDocument> = {
            use nanites_core::dyn_task::{AnyInput, erase_io};

            // 1a. Embed the query.
            let embed_task = erase_io(retrieve_task.embedding_task.clone());
            let embed_input: AnyInput = Box::new(vec![user_message.clone()]);
            let embed_out = match runtime.run_dyn(embed_task, embed_input).await {
                Ok(out) => out,
                Err(e) => {
                    eprintln!("embedding error: {e}");
                    break;
                }
            };
            let mut vecs: Vec<Vec<f32>> = match embed_out.downcast::<Vec<Vec<f32>>>() {
                Ok(v) => *v,
                Err(_) => {
                    eprintln!("embedding error: unexpected output type");
                    break;
                }
            };
            let query_vec = match vecs.pop() {
                Some(v) => v,
                None => {
                    eprintln!("embedding error: no vectors returned");
                    break;
                }
            };

            // 1b. Search the vector store.
            let store_task = erase_io(retrieve_task.store_task.clone());
            let store_input: AnyInput = Box::new(query_vec);
            let store_out = match runtime.run_dyn(store_task, store_input).await {
                Ok(out) => out,
                Err(e) => {
                    eprintln!("retrieval error: {e}");
                    break;
                }
            };
            match store_out.downcast::<Vec<RetrievedDocument>>() {
                Ok(docs) => *docs,
                Err(_) => {
                    eprintln!("retrieval error: unexpected output type");
                    break;
                }
            }
        };

        // Step 2: build augmented character knowledge from retrieved docs.
        let mut augmented = character.clone();
        if !retrieved.is_empty() {
            augmented
                .current_knowledge
                .push("Retrieved context (use this to inform your answer):".into());
            for doc in &retrieved {
                augmented
                    .current_knowledge
                    .push(format!("[{:.2}] {}", doc.score, doc.content));
            }
        }

        // Step 3: select recent history.
        let recent: Vec<HistoryEntry> = history
            .iter()
            .rev()
            .take(MAX_HISTORY)
            .rev()
            .cloned()
            .collect();

        // Step 4: run one chat turn with the augmented character.
        let task = HandleMessageTask {
            model: completion_model.clone(),
            character: augmented,
            history: recent,
        };

        match runtime.run(task, user_message.clone()).await {
            Ok(response) => {
                println!("mira> {response}\n");

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

// ─── Seed mode ─────────────────────────────────────────────────────────────────

/// Reads all `.txt` files under `path`, embeds them, stores them in a
/// `VectorCollection`, then prints a summary and exits.
async fn seed_documents(
    path: &str,
    embed_provider: &str,
    embed_model: &str,
    embedding_executor: RigEmbeddingExecutor,
) {
    eprintln!("Seeding documents from: {path}");

    // Collect .txt files.
    let txt_files = collect_txt_files(path);
    if txt_files.is_empty() {
        eprintln!("No .txt files found in {path:?}.");
        std::process::exit(0);
    }

    // Read file contents.
    let mut ids: Vec<String> = Vec::new();
    let mut texts: Vec<String> = Vec::new();
    for file_path in &txt_files {
        match std::fs::read_to_string(file_path) {
            Ok(content) => {
                ids.push(file_path.clone());
                texts.push(content);
            }
            Err(e) => {
                eprintln!("warning: could not read {file_path:?}: {e}");
            }
        }
    }

    if texts.is_empty() {
        eprintln!("No readable .txt files found.");
        std::process::exit(0);
    }

    eprintln!("Embedding {} document(s)...", texts.len());

    // EmbeddingTask is an IoTask, not a Task — use run_dyn.
    let embed_task = EmbeddingTask {
        provider: embed_provider.to_owned(),
        model: Some(embed_model.to_owned()),
    };

    let runtime = Runtime::new().with_executor(embedding_executor);

    let vectors: Vec<Vec<f32>> = {
        use nanites_core::dyn_task::erase_io;
        let task = erase_io(embed_task);
        let input: nanites_core::dyn_task::AnyInput = Box::new(texts.clone());
        match runtime.run_dyn(task, input).await {
            Ok(out) => match out.downcast::<Vec<Vec<f32>>>() {
                Ok(vecs) => *vecs,
                Err(_) => {
                    eprintln!("embedding error: unexpected output type");
                    std::process::exit(1);
                }
            },
            Err(e) => {
                eprintln!("embedding error: {e}");
                std::process::exit(1);
            }
        }
    };

    // Populate the collection.
    let mut collection = VectorCollection::new();
    let mut indexed = 0usize;
    for (i, vec) in vectors.into_iter().enumerate() {
        collection.add(&ids[i], &texts[i], vec);
        indexed += 1;
    }

    eprintln!("Indexed {indexed} document(s) into collection {COLLECTION_NAME:?}.");
    eprintln!();
    eprintln!("Note: the in-memory collection is not persisted between runs.");
    eprintln!("Re-run without --seed to chat (the store will be empty).");
}

/// Recursively collect paths to all `.txt` files under `dir`.
fn collect_txt_files(dir: &str) -> Vec<String> {
    let mut result = Vec::new();
    collect_txt_files_rec(Path::new(dir), &mut result);
    result.sort();
    result
}

fn collect_txt_files_rec(dir: &Path, out: &mut Vec<String>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_txt_files_rec(&path, out);
        } else if path.extension().map(|e| e == "txt").unwrap_or(false)
            && let Some(s) = path.to_str()
        {
            out.push(s.to_owned());
        }
    }
}

// ─── Argument parsing ──────────────────────────────────────────────────────────

fn usage() -> ! {
    eprintln!(
        "usage: cargo run --example chat_rag -p nanites-chat -- \
         <provider:model> <embed-provider:model> [--seed <path>]"
    );
    eprintln!();
    eprintln!("examples:");
    eprintln!("  anthropic:claude-3-5-haiku-20241022 openai:text-embedding-3-small");
    eprintln!("  openai:gpt-4o-mini                  openai:text-embedding-3-small");
    eprintln!("  ollama:llama3.2                     ollama:nomic-embed-text");
    std::process::exit(1);
}

fn parse_provider_model(arg: &str) -> (String, String) {
    match arg.split_once(':') {
        Some((provider, model)) if !provider.is_empty() && !model.is_empty() => {
            (provider.to_owned(), model.to_owned())
        }
        _ => {
            eprintln!(
                "error: argument must be in the form provider:model \
                 (e.g. anthropic:claude-3-5-haiku-20241022)"
            );
            std::process::exit(1);
        }
    }
}

fn parse_seed_flag(args: &[String]) -> Option<&str> {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--seed" {
            return iter.next().map(String::as_str);
        }
    }
    None
}

// ─── Executor construction ─────────────────────────────────────────────────────

fn build_completion_executor(provider: &str, model_name: &str) -> RigCompletionExecutor {
    match provider {
        "anthropic" => {
            let api_key = require_env("ANTHROPIC_API_KEY", provider);
            let client = rig::providers::anthropic::Client::new(&api_key)
                .unwrap_or_else(|e| fatal_client_error(provider, e));
            let model = client.completion_model(model_name);
            RigCompletionExecutor::new().with_model(model_name, model)
        }
        "azure" => {
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
                "error: unknown completion provider {other:?}\n\
                 supported providers: anthropic, azure, cohere, deepseek, galadriel, gemini, \
                 groq, huggingface, hyperbolic, mira, mistral, moonshot, ollama, openai, \
                 openrouter, perplexity, together, xai"
            );
            std::process::exit(1);
        }
    }
}

fn build_embedding_executor(provider: &str, model_name: &str) -> RigEmbeddingExecutor {
    match provider {
        "azure" => {
            let client = rig::providers::azure::Client::from_env();
            let model = client.embedding_model(model_name);
            RigEmbeddingExecutor::new().with_model(provider, model_name, model)
        }
        "cohere" => {
            // Cohere requires an input_type; use "search_document" as the default.
            let api_key = require_env("COHERE_API_KEY", provider);
            let client = rig::providers::cohere::Client::new(&api_key)
                .unwrap_or_else(|e| fatal_client_error(provider, e));
            let model = client.embedding_model(model_name, "search_document");
            RigEmbeddingExecutor::new().with_model(provider, model_name, model)
        }
        "gemini" => {
            let api_key = require_env("GEMINI_API_KEY", provider);
            let client = rig::providers::gemini::Client::new(&api_key)
                .unwrap_or_else(|e| fatal_client_error(provider, e));
            let model = client.embedding_model(model_name);
            RigEmbeddingExecutor::new().with_model(provider, model_name, model)
        }
        "mistral" => {
            let api_key = require_env("MISTRAL_API_KEY", provider);
            let client = rig::providers::mistral::Client::new(&api_key)
                .unwrap_or_else(|e| fatal_client_error(provider, e));
            let model = client.embedding_model(model_name);
            RigEmbeddingExecutor::new().with_model(provider, model_name, model)
        }
        "ollama" => {
            let client = rig::providers::ollama::Client::new(rig::client::Nothing)
                .unwrap_or_else(|e| fatal_client_error(provider, e));
            let model = client.embedding_model(model_name);
            RigEmbeddingExecutor::new().with_model(provider, model_name, model)
        }
        "openai" => {
            let api_key = require_env("OPENAI_API_KEY", provider);
            let client = rig::providers::openai::Client::new(&api_key)
                .unwrap_or_else(|e| fatal_client_error(provider, e));
            let model = client.embedding_model(model_name);
            RigEmbeddingExecutor::new().with_model(provider, model_name, model)
        }
        "openrouter" => {
            let api_key = require_env("OPENROUTER_API_KEY", provider);
            let client = rig::providers::openrouter::Client::new(&api_key)
                .unwrap_or_else(|e| fatal_client_error(provider, e));
            let model = client.embedding_model(model_name);
            RigEmbeddingExecutor::new().with_model(provider, model_name, model)
        }
        "together" => {
            let api_key = require_env("TOGETHER_API_KEY", provider);
            let client = rig::providers::together::Client::new(&api_key)
                .unwrap_or_else(|e| fatal_client_error(provider, e));
            let model = client.embedding_model(model_name);
            RigEmbeddingExecutor::new().with_model(provider, model_name, model)
        }
        other => {
            eprintln!(
                "error: unknown embedding provider {other:?}\n\
                 supported providers: azure, cohere, gemini, mistral, ollama, openai, \
                 openrouter, together"
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
