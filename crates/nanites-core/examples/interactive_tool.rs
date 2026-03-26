//! interactive_tool.rs — event-driven autocomplete with cancellation.
//!
//! Simulates a text editor autocomplete tool where each keystroke triggers a
//! suggestion task. When a new keystroke arrives, the previous in-flight
//! suggestion is cancelled via `runtime.cancel()`. Only the final suggestion
//! in a burst actually completes.
//!
//! Demonstrates:
//! - Event-driven dispatching (each keystroke spawns an independent task)
//! - Cooperative cancellation via `CancellationToken` (checked inside the task)
//! - Fresh context per call (each keystroke gets a fresh runtime + document snapshot)
//! - Exec graph inspection (cancelled vs completed tasks with timing)
//!
//! Run with: cargo run --example interactive_tool

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nanites_core::{Ctx, ExecGraph, Runtime, Task, TerminalState};

// ─── Document state ──────────────────────────────────────────────────────────

/// Accumulates keystrokes into a document buffer. Each task receives a
/// snapshot of the current text — no shared mutable state between tasks.
struct DocumentState {
    text: String,
}

impl DocumentState {
    fn new() -> Self {
        Self {
            text: String::new(),
        }
    }

    fn push(&mut self, ch: char) {
        self.text.push(ch);
    }

    fn snapshot(&self) -> String {
        self.text.clone()
    }
}

// ─── SuggestTask ─────────────────────────────────────────────────────────────

/// A pure task that analyzes the current document text and produces an
/// autocomplete suggestion. Simulates latency with sleep and checks for
/// cancellation periodically.
#[derive(Debug, Clone)]
struct SuggestTask {
    /// Simulated processing latency in milliseconds.
    latency_ms: u64,
}

impl Task for SuggestTask {
    type Input = String;
    type Output = String;
    type Error = SuggestError;

    async fn run(&self, input: Self::Input, ctx: &Ctx) -> Result<Self::Output, Self::Error> {
        let step_duration = Duration::from_millis(self.latency_ms / 4);

        for step in 0..4 {
            if ctx.is_cancelled() {
                return Err(SuggestError::Cancelled);
            }

            tokio::time::sleep(step_duration).await;

            if ctx.is_cancelled() {
                return Err(SuggestError::Cancelled);
            }

            // Simulate doing some analysis mid-way through.
            if step == 1 {
                let _word_count = input.split_whitespace().count();
            }
        }

        // Produce a suggestion based on the last word in the document.
        let suggestion = match input.split_whitespace().last() {
            Some(last_word) => format!("...{last_word}_completion"),
            None => "...empty_completion".to_string(),
        };

        Ok(suggestion)
    }
}

// ─── Error type ──────────────────────────────────────────────────────────────

#[derive(Debug)]
enum SuggestError {
    Cancelled,
}

impl std::fmt::Display for SuggestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SuggestError::Cancelled => write!(f, "suggestion cancelled"),
        }
    }
}

impl std::error::Error for SuggestError {}

// ─── Task record for timing ─────────────────────────────────────────────────

struct TaskRecord {
    keystroke: char,
    #[allow(dead_code)]
    doc_text: String,
    spawned_at: Instant,
    finished_at: Option<Instant>,
    outcome: String,
    suggestion: Option<String>,
}

// ─── Main ────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    let start = Instant::now();

    // Simulated keystrokes: rapid burst typing "hello worl" then a pause.
    // The delay is the time *after* each keystroke before the next arrives.
    let keystrokes: Vec<(char, Duration)> = vec![
        ('h', Duration::from_millis(10)),
        ('e', Duration::from_millis(10)),
        ('l', Duration::from_millis(8)),
        ('l', Duration::from_millis(12)),
        ('o', Duration::from_millis(9)),
        (' ', Duration::from_millis(15)),
        ('w', Duration::from_millis(11)),
        ('o', Duration::from_millis(7)),
        ('r', Duration::from_millis(10)),
        ('l', Duration::from_millis(300)), // pause — gives the last task time to complete
    ];

    let mut doc = DocumentState::new();
    let records: Arc<Mutex<Vec<TaskRecord>>> = Arc::new(Mutex::new(Vec::new()));

    // Each keystroke gets its own Runtime so we can cancel it independently
    // via `runtime.cancel()`. The task inside checks `ctx.is_cancelled()`.
    let mut current_runtime: Option<Runtime> = None;
    let mut current_handle: Option<tokio::task::JoinHandle<()>> = None;

    // Collect all runtimes so we can inspect their exec graphs afterwards.
    let mut all_runtimes: Vec<Runtime> = Vec::new();

    println!("--- Simulating autocomplete keystrokes ---\n");

    for (i, (ch, delay_after)) in keystrokes.iter().enumerate() {
        doc.push(*ch);
        let doc_snapshot = doc.snapshot();

        // Cancel the previous in-flight task by signalling its runtime's
        // cancellation token. The task checks ctx.is_cancelled() and bails.
        if let Some(prev_runtime) = current_runtime.take() {
            prev_runtime.cancel();
        }

        // Also abort the tokio task as a fallback — if the task is blocked
        // in a sleep, the abort kills it immediately.
        if let Some(prev_handle) = current_handle.take() {
            prev_handle.abort();
        }

        // Fresh runtime per keystroke — independent cancellation token.
        let runtime = Runtime::new();
        current_runtime = Some(runtime.clone());
        all_runtimes.push(runtime.clone());

        let spawned_at = Instant::now();
        let records_clone = Arc::clone(&records);

        {
            let mut recs = records_clone.lock().unwrap();
            recs.push(TaskRecord {
                keystroke: *ch,
                doc_text: doc_snapshot.clone(),
                spawned_at,
                finished_at: None,
                outcome: "pending".to_string(),
                suggestion: None,
            });
        }

        let task = SuggestTask { latency_ms: 120 };
        let idx = i;

        let handle = tokio::spawn(async move {
            let result = runtime.run(task, doc_snapshot).await;
            let mut recs = records_clone.lock().unwrap();
            let record = &mut recs[idx];
            record.finished_at = Some(Instant::now());
            match result {
                Ok(suggestion) => {
                    record.outcome = "completed".to_string();
                    record.suggestion = Some(suggestion);
                }
                Err(e) => {
                    let msg = e.to_string();
                    record.outcome = if msg.contains("cancelled") {
                        "cancelled".to_string()
                    } else {
                        format!("failed: {msg}")
                    };
                }
            }
        });

        current_handle = Some(handle);

        println!(
            "  [{:>5.1?}] keystroke '{ch}' -> spawned SuggestTask #{idx} (doc: \"{}\")",
            spawned_at.duration_since(start),
            doc.snapshot(),
        );

        tokio::time::sleep(*delay_after).await;
    }

    // Wait for the final task to complete.
    if let Some(handle) = current_handle.take() {
        let _ = handle.await;
    }

    // Give aborted tasks a moment to settle.
    tokio::time::sleep(Duration::from_millis(50)).await;

    // ─── Cross-reference records with exec graph terminal states ─────────
    // When a tokio task is aborted, the record-updating code never runs.
    // Fill in outcomes from the exec graph for those tasks.
    {
        let mut recs = records.lock().unwrap();
        for (i, rt) in all_runtimes.iter().enumerate() {
            if i >= recs.len() {
                break;
            }
            let record = &mut recs[i];
            if record.outcome == "pending" {
                // Check this task's exec graph for its terminal state.
                let nodes = rt.exec_graph().snapshot();
                if let Some(node) = nodes.first() {
                    match &node.terminal {
                        Some(TerminalState::Completed { .. }) => {
                            record.outcome = "completed".to_string();
                        }
                        Some(TerminalState::Failed { message }) => {
                            record.outcome = if message.contains("cancelled") {
                                "cancelled".to_string()
                            } else {
                                format!("failed: {message}")
                            };
                        }
                        Some(TerminalState::Cancelled) => {
                            record.outcome = "cancelled".to_string();
                        }
                        None => {
                            // Task was aborted before the runtime could record a terminal state.
                            record.outcome = "aborted".to_string();
                        }
                    }
                    record.finished_at = Some(Instant::now());
                }
            }
        }
    }

    // ─── Inspect exec graphs ─────────────────────────────────────────────

    println!("\n--- Exec graph summary ---\n");

    let mut total = 0;
    let mut completed_count = 0;
    let mut cancelled_count = 0;
    let mut failed_count = 0;
    let mut aborted_count = 0;

    for rt in &all_runtimes {
        let graph: &ExecGraph = rt.exec_graph();
        for node in graph.snapshot() {
            total += 1;
            match &node.terminal {
                Some(TerminalState::Completed { .. }) => completed_count += 1,
                Some(TerminalState::Cancelled) => cancelled_count += 1,
                Some(TerminalState::Failed { .. }) => failed_count += 1,
                None => aborted_count += 1,
            }
        }
    }

    println!("  Total tasks spawned:  {total}");
    println!("  Completed:            {completed_count}");
    println!("  Cancelled (token):    {cancelled_count}");
    println!("  Failed (cancelled):   {failed_count}");
    println!("  Aborted (tokio):      {aborted_count}");

    // ─── Per-task timing ─────────────────────────────────────────────────

    println!("\n--- Per-task timing ---\n");
    let header = format!(
        "  {:>3}  {:>5}  {:>7}  {:>10}  {:>10}  {}",
        "#", "Key", "Offset", "Outcome", "Duration", "Suggestion"
    );
    println!("{header}");
    println!("  {}", "-".repeat(68));

    let recs = records.lock().unwrap();
    for (i, record) in recs.iter().enumerate() {
        let offset = record.spawned_at.duration_since(start);
        let duration = record
            .finished_at
            .map(|f| f.duration_since(record.spawned_at));
        let dur_str = match duration {
            Some(d) => format!("{d:>8.1?}"),
            None => "       -".to_string(),
        };
        let suggestion = record
            .suggestion
            .as_deref()
            .unwrap_or("-")
            .chars()
            .take(25)
            .collect::<String>();

        println!(
            "  {:>3}  {:>5}  {:>5.1?}  {:>10}  {dur_str}  {suggestion}",
            i,
            format!("'{}'", record.keystroke),
            offset,
            record.outcome,
        );
    }

    // ─── Assertions ──────────────────────────────────────────────────────

    // The last task should have completed with a suggestion.
    let last = recs.last().expect("should have records");
    assert_eq!(last.outcome, "completed", "last task should complete");
    assert!(
        last.suggestion.is_some(),
        "last task should produce a suggestion"
    );
    println!(
        "\nFinal suggestion: {:?}",
        last.suggestion.as_deref().unwrap()
    );

    // Count how many earlier tasks completed despite cancellation.
    let early_completed = recs[..recs.len() - 1]
        .iter()
        .filter(|r| r.outcome == "completed")
        .count();
    let early_cancelled_or_aborted = recs[..recs.len() - 1]
        .iter()
        .filter(|r| r.outcome != "completed")
        .count();
    println!("Early tasks completed:  {early_completed}");
    println!("Early tasks cancelled:  {early_cancelled_or_aborted}");

    // At least half the early tasks should have been cancelled/aborted.
    assert!(
        early_cancelled_or_aborted >= 5,
        "expected most early tasks to be cancelled, got {early_cancelled_or_aborted}/{}",
        recs.len() - 1
    );

    println!("\ninteractive_tool: ok");
}
