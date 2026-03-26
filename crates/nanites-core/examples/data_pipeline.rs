//! data_pipeline.rs — a batch data pipeline with fan-out, caching, and checkpointing.
//!
//! Demonstrates:
//! - Fan-out with `Map` combinator (broadcast a task over a collection)
//! - Chaining tasks via `TaskHandle` dependencies inside an orchestrator
//! - Pure Tasks for parsing, classifying, filtering, and aggregation
//! - `ExecGraph` inspection after the run (full lineage tree)
//! - `MemoryCache` — run the pipeline twice, show cache hits on second run
//! - Checkpoint serialization — serialize the checkpoint to JSON, print its size
//!
//! All pure Tasks — no IoTask, no API keys, runs fully offline.
//!
//! Run with: cargo run --example data_pipeline

use std::sync::Arc;

use nanites_core::cache::MemoryCache;
use nanites_core::combinators::Map;
use nanites_core::dyn_task::erase_serializable;
use nanites_core::exec_graph::TerminalState;
use nanites_core::registry::SerializableTask;
use nanites_core::{Ctx, Runtime, Task};
use serde::{Deserialize, Serialize};

// ─── Domain types ───────────────────────────────────────────────────────────

/// A single parsed record from the CSV-like input.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Record {
    id: u32,
    text: String,
    value: f64,
}

/// A record annotated with a category.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct ClassifiedRecord {
    record: Record,
    category: String,
}

/// Summary statistics over a batch of records.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Summary {
    count: usize,
    total_value: f64,
    mean_value: f64,
    categories: Vec<String>,
}

// ─── Error type ─────────────────────────────────────────────────────────────

#[derive(Debug)]
struct PipelineError(String);

impl std::fmt::Display for PipelineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for PipelineError {}

impl From<nanites_core::RuntimeError> for PipelineError {
    fn from(e: nanites_core::RuntimeError) -> Self {
        PipelineError(e.to_string())
    }
}

// ─── Stage 1: Parse ─────────────────────────────────────────────────────────

/// Parse a single CSV-like line into a Record.
///
/// Expected format: "id,text,value"
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ParseRecord;

impl Task for ParseRecord {
    type Input = String;
    type Output = Record;
    type Error = PipelineError;

    async fn run(&self, input: Self::Input, _ctx: &Ctx) -> Result<Self::Output, Self::Error> {
        let parts: Vec<&str> = input.trim().splitn(3, ',').collect();
        if parts.len() != 3 {
            return Err(PipelineError(format!("bad record: {input:?}")));
        }
        let id = parts[0]
            .parse::<u32>()
            .map_err(|e| PipelineError(format!("bad id: {e}")))?;
        let text = parts[1].to_string();
        let value = parts[2]
            .parse::<f64>()
            .map_err(|e| PipelineError(format!("bad value: {e}")))?;
        Ok(Record { id, text, value })
    }
}

impl SerializableTask for ParseRecord {
    fn serializable_type_name(&self) -> &'static str {
        "pipeline::ParseRecord"
    }
    fn params(&self) -> serde_json::Value {
        serde_json::json!({})
    }
}

// ─── Stage 2: Classify ──────────────────────────────────────────────────────

/// Assign a category to a record based on keyword matching.
///
/// No LLM needed — purely deterministic keyword classification.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ClassifyRecord;

impl Task for ClassifyRecord {
    type Input = Record;
    type Output = ClassifiedRecord;
    type Error = std::convert::Infallible;

    async fn run(&self, input: Self::Input, _ctx: &Ctx) -> Result<Self::Output, Self::Error> {
        let text_lower = input.text.to_lowercase();
        let category = if text_lower.contains("error") || text_lower.contains("fail") {
            "error"
        } else if text_lower.contains("warn") || text_lower.contains("slow") {
            "warning"
        } else if text_lower.contains("success") || text_lower.contains("ok") {
            "success"
        } else {
            "info"
        }
        .to_string();

        Ok(ClassifiedRecord {
            record: input,
            category,
        })
    }
}

impl SerializableTask for ClassifyRecord {
    fn serializable_type_name(&self) -> &'static str {
        "pipeline::ClassifyRecord"
    }
    fn params(&self) -> serde_json::Value {
        serde_json::json!({})
    }
}

// ─── Stage 3: Filter ────────────────────────────────────────────────────────

/// Keep only records matching specified categories and above a value threshold.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct FilterRecords {
    /// Only keep records in these categories.
    categories: Vec<String>,
    /// Only keep records with value >= this threshold.
    min_value: f64,
}

impl Task for FilterRecords {
    type Input = Vec<ClassifiedRecord>;
    type Output = Vec<ClassifiedRecord>;
    type Error = std::convert::Infallible;

    async fn run(&self, input: Self::Input, _ctx: &Ctx) -> Result<Self::Output, Self::Error> {
        let filtered = input
            .into_iter()
            .filter(|r| self.categories.contains(&r.category) && r.record.value >= self.min_value)
            .collect();
        Ok(filtered)
    }
}

impl SerializableTask for FilterRecords {
    fn serializable_type_name(&self) -> &'static str {
        "pipeline::FilterRecords"
    }
    fn params(&self) -> serde_json::Value {
        serde_json::json!({
            "categories": self.categories,
            "min_value": self.min_value,
        })
    }
}

// ─── Stage 4: Aggregate ─────────────────────────────────────────────────────

/// Compute summary statistics over the filtered records.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct AggregateRecords;

impl Task for AggregateRecords {
    type Input = Vec<ClassifiedRecord>;
    type Output = Summary;
    type Error = std::convert::Infallible;

    async fn run(&self, input: Self::Input, _ctx: &Ctx) -> Result<Self::Output, Self::Error> {
        let count = input.len();
        let total_value: f64 = input.iter().map(|r| r.record.value).sum();
        let mean_value = if count > 0 {
            total_value / count as f64
        } else {
            0.0
        };

        let mut categories: Vec<String> = input.iter().map(|r| r.category.clone()).collect();
        categories.sort();
        categories.dedup();

        Ok(Summary {
            count,
            total_value,
            mean_value,
            categories,
        })
    }
}

impl SerializableTask for AggregateRecords {
    fn serializable_type_name(&self) -> &'static str {
        "pipeline::AggregateRecords"
    }
    fn params(&self) -> serde_json::Value {
        serde_json::json!({})
    }
}

// ─── Orchestrator ───────────────────────────────────────────────────────────

/// The full pipeline: parse -> classify (fan-out) -> filter -> aggregate.
///
/// Uses `Map` combinator for fan-out over individual records, then chains
/// filter and aggregate stages via `ctx.spawn`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct DataPipeline {
    filter_categories: Vec<String>,
    min_value: f64,
}

impl Task for DataPipeline {
    type Input = Vec<String>;
    type Output = Summary;
    type Error = PipelineError;

    async fn run(&self, input: Self::Input, ctx: &Ctx) -> Result<Self::Output, Self::Error> {
        // Stage 1: Parse all records in parallel via Map combinator.
        let records: Vec<Record> = ctx
            .spawn(Map { inner: ParseRecord }, input)
            .await
            .map_err(|e| PipelineError(e.to_string()))?;

        println!("  [parse]    {} records parsed", records.len());

        // Stage 2: Classify all records in parallel via Map combinator.
        let classified: Vec<ClassifiedRecord> = ctx
            .spawn(
                Map {
                    inner: ClassifyRecord,
                },
                records,
            )
            .await
            .map_err(|e| PipelineError(e.to_string()))?;

        println!("  [classify] {} records classified", classified.len());
        for r in &classified {
            println!(
                "             #{}: {:?} -> {} (value={:.1})",
                r.record.id, r.record.text, r.category, r.record.value
            );
        }

        // Stage 3: Filter.
        let filtered: Vec<ClassifiedRecord> = ctx
            .spawn(
                FilterRecords {
                    categories: self.filter_categories.clone(),
                    min_value: self.min_value,
                },
                classified,
            )
            .await
            .map_err(|e| PipelineError(e.to_string()))?;

        println!("  [filter]   {} records passed filter", filtered.len());

        // Stage 4: Aggregate.
        let summary: Summary = ctx
            .spawn(AggregateRecords, filtered)
            .await
            .map_err(|e| PipelineError(e.to_string()))?;

        println!("  [agg]      summary computed");

        Ok(summary)
    }
}

impl SerializableTask for DataPipeline {
    fn serializable_type_name(&self) -> &'static str {
        "pipeline::DataPipeline"
    }
    fn params(&self) -> serde_json::Value {
        serde_json::json!({
            "filter_categories": self.filter_categories,
            "min_value": self.min_value,
        })
    }
}

// ─── Helpers ────────────────────────────────────────────────────────────────

/// Hardcoded CSV-like input data.
fn sample_data() -> Vec<String> {
    vec![
        "1,Server error rate spiked,95.2".into(),
        "2,Deploy success confirmed,42.0".into(),
        "3,Slow query detected,78.5".into(),
        "4,Health check ok,12.3".into(),
        "5,Connection failure observed,88.1".into(),
        "6,Build success on CI,55.0".into(),
        "7,Warn: memory usage high,67.9".into(),
        "8,Routine info log entry,5.0".into(),
    ]
}

/// Print the exec graph as an indented tree.
fn print_exec_tree(runtime: &Runtime) {
    let mut nodes = runtime.exec_graph().snapshot();
    nodes.sort_by_key(|n| n.id);

    println!("\nExec graph ({} nodes):", nodes.len());

    // Find roots (no parent).
    let roots: Vec<_> = nodes.iter().filter(|n| n.parent.is_none()).collect();

    fn print_node(node: &nanites_core::ExecNode, all: &[nanites_core::ExecNode], indent: usize) {
        let prefix = "  ".repeat(indent);
        let status = match &node.terminal {
            Some(TerminalState::Completed { .. }) => "ok",
            Some(TerminalState::Failed { message }) => message.as_str(),
            Some(TerminalState::Cancelled) => "cancelled",
            None => "running",
        };
        // Shorten the type name: take the last segment after "::"
        let short_type = node
            .task_type
            .rsplit("::")
            .next()
            .unwrap_or(&node.task_type);
        println!("{prefix}[{id}] {short_type} ({status})", id = node.id);

        let children: Vec<_> = all.iter().filter(|n| n.parent == Some(node.id)).collect();
        for child in children {
            print_node(child, all, indent + 1);
        }
    }

    for root in &roots {
        print_node(root, &nodes, 1);
    }
}

// ─── Main ───────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    let data = sample_data();

    let pipeline = DataPipeline {
        filter_categories: vec!["error".into(), "warning".into()],
        min_value: 50.0,
    };

    // ── Run 1: no cache ─────────────────────────────────────────────────────

    println!("=== Run 1: no cache ===\n");

    let runtime = Runtime::new();
    let summary = runtime.run(pipeline.clone(), data.clone()).await.unwrap();

    println!("\nResult: {summary:#?}");
    assert_eq!(summary.count, 4);
    assert!(summary.categories.contains(&"error".to_string()));
    assert!(summary.categories.contains(&"warning".to_string()));

    // Inspect the exec graph — full task tree with parent-child relationships.
    print_exec_tree(&runtime);

    // Checkpoint serialization.
    let checkpoint = runtime.checkpoint();
    let checkpoint_json = serde_json::to_string(&checkpoint).unwrap();
    println!(
        "\nCheckpoint: {} bytes JSON, {} exec nodes, {} frontier nodes",
        checkpoint_json.len(),
        checkpoint.exec_graph.len(),
        checkpoint.frontier.len(),
    );

    // Verify checkpoint roundtrips.
    let restored: nanites_core::Checkpoint = serde_json::from_str(&checkpoint_json).unwrap();
    assert_eq!(restored.exec_graph.len(), checkpoint.exec_graph.len());

    // ── Run 2: with MemoryCache ─────────────────────────────────────────────

    println!("\n=== Run 2: with MemoryCache ===\n");

    let cache = Arc::new(MemoryCache::new());
    let cached_runtime = Runtime::new().with_cache(cache.clone());

    // First run with cache — populates it.
    let summary1 = cached_runtime
        .run_dyn(erase_serializable(pipeline.clone()), Box::new(data.clone()))
        .await
        .unwrap();
    let summary1 = *summary1.downcast::<Summary>().unwrap();
    println!(
        "  First cached run complete. Cache entries: {}",
        cache.len()
    );
    assert_eq!(summary1.count, 4);

    // Second run — should hit the cache for the top-level pipeline task.
    let summary2 = cached_runtime
        .run_dyn(erase_serializable(pipeline.clone()), Box::new(data.clone()))
        .await
        .unwrap();
    let summary2 = *summary2.downcast::<Summary>().unwrap();
    println!(
        "  Second cached run complete. Cache entries: {}",
        cache.len()
    );
    assert_eq!(summary2, summary1);

    // The exec graph on the cached runtime has nodes from both runs, but the
    // second run hit the cache, so its node completed without re-executing.
    let exec_nodes = cached_runtime.exec_graph().snapshot();
    println!(
        "  Exec graph after two runs: {} nodes total",
        exec_nodes.len()
    );

    // Verify the second run's top-level node completed (cache hit).
    let mut sorted = exec_nodes.clone();
    sorted.sort_by_key(|n| n.id);
    let second_root = sorted
        .iter()
        .filter(|n| n.parent.is_none())
        .nth(1)
        .expect("should have two root nodes (one per run)");
    assert!(
        matches!(&second_root.terminal, Some(TerminalState::Completed { .. })),
        "second run should have completed (via cache hit)"
    );
    // The second root should have no children — it was served from cache
    // without spawning subtasks.
    assert!(
        second_root.children.is_empty(),
        "cached run should not have spawned child tasks"
    );
    println!("  Second run served from cache (no child tasks spawned).");

    println!("\ndata_pipeline: ok");
}
