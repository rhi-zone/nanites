# Use Case: Data Pipeline

Structured data transformation with LLM steps where needed. The canonical "boring but important" use case.

## Task Graph Shape

DAG of transformation steps. Topology may be static (known upfront) or dynamic (discovered from data).

```
IngestTask(source)
├── ParseTask                          ← pure, parallel per-record
├── ClassifyTask(model="haiku")        ← LLM, parallel per-record
│   ├── EnrichTask(classified_record)  ← pure, depends on classify
│   └── FilterTask(threshold=0.9)      ← pure
└── AggregateTask                      ← waits for all above
    └── ExportTask(destination)
```

## Task Inventory

### IoTasks
- `CompletionTask { model, system }` — for any LLM step
- `FetchRecordTask { source, record_id }` — external data fetch
- `ExportTask { destination }` — write results out

### Pure Tasks
- `ParseTask` — deterministic format parsing
- `ClassifyTask` — wraps `CompletionTask` via `ctx.spawn`, adds schema parsing
- `FilterTask { threshold }` — keeps records above confidence threshold
- `AggregateTask` — collects results, computes stats
- `EnrichTask` — joins additional data sources

## Key Pattern: Fan-Out + Fan-In

```rust
impl Task for ProcessBatchTask {
    type Input = Vec<RawRecord>;
    type Output = Vec<ProcessedRecord>;
    type Error = BoxError;

    async fn run(&self, records: Vec<RawRecord>, ctx: &Ctx) -> Result<Vec<ProcessedRecord>, BoxError> {
        // Fan-out: classify all records in parallel
        let handles = ctx.spawn_all(
            ClassifyTask { model: self.model.clone() },
            records,
        );

        // Fan-in: collect results
        let mut results = Vec::with_capacity(handles.len());
        for h in handles {
            results.push(h.await?);
        }
        Ok(results)
    }
}
```

## Checkpoint/Resume

The exec graph + task serialization enable checkpointing:
1. Serialize the pending frontier + completed exec graph to disk
2. On restart: replay completed nodes from cache, resume pending

Currently aspirational — the serialization layer exists but checkpoint/restore is not yet wired up.

## What "Flexible" Means Here

- Rerun any node with different parameters without rerunning its dependencies (if cached)
- Replace LLM classification with a deterministic rule-based classifier — same task interface, different executor
- Fork a pipeline at any intermediate node to try two different downstream approaches
- Inspect every LLM call's input/output via the exec graph after the run

## Gaps in Current Substrate

1. **No result caching** — completed tasks are in the exec graph but their outputs are not persisted. Re-running the same task re-executes it. Need a cache keyed on (task type, params, input hash).

2. **No checkpoint/restore** — serialization exists per task, but no mechanism to serialize the full runtime state (frontier + exec graph) to disk and restore it.

3. **Fan-in patterns are manual** — collecting `Vec<TaskHandle<O>>` and awaiting in a loop works, but there's no `ctx.join_all` that gives you back `Result<Vec<O>, E>` like `futures::try_join_all`. `spawn_all` gives handles but not a join.

4. **No typed DAG declaration** — for static pipelines, it's ergonomic to declare the topology once. Right now you'd write it as nested `ctx.spawn` calls. A builder API for static DAGs could help, but may be premature.
