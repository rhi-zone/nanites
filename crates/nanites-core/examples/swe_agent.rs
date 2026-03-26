//! swe_agent.rs — a simulated software engineering agent with recursive decomposition.
//!
//! Demonstrates the core thesis of nanites: the task graph is not known upfront.
//! It grows dynamically as the agent discovers what needs to be done.
//!
//! Scenario: "Add a greeting function to the user module and update tests."
//!
//! The agent decomposes this into analysis, planning, editing, and testing phases.
//! Each phase spawns subtasks based on the output of the previous phase.
//! All I/O (filesystem reads) goes through an IoTask + TaskExecutor pair.
//!
//! No real LLM calls, no API keys — everything is deterministic.
//!
//! Run with: cargo run --example swe_agent

use std::any::Any;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use nanites_core::dyn_task::{AnyInput, AnyOutput, DynTask, IoTask, SharedDynTask, erase_io};
use nanites_core::error::BoxError;
use nanites_core::exec_graph::TerminalState;
use nanites_core::executor::TaskExecutor;
use nanites_core::{Ctx, Runtime, Scaffold, Task};

// ─── Error type ──────────────────────────────────────────────────────────────

#[derive(Debug)]
struct SweError(String);

impl std::fmt::Display for SweError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for SweError {}

impl From<nanites_core::RuntimeError> for SweError {
    fn from(e: nanites_core::RuntimeError) -> Self {
        SweError(e.to_string())
    }
}

// ─── Simulated filesystem ────────────────────────────────────────────────────

/// Shared fake filesystem: path -> contents.
type FakeFs = Arc<Mutex<HashMap<String, String>>>;

fn initial_filesystem() -> FakeFs {
    let mut fs = HashMap::new();
    fs.insert("src/lib.rs".to_string(), "pub mod user;\n".to_string());
    fs.insert(
        "src/user.rs".to_string(),
        concat!(
            "pub struct User {\n",
            "    pub name: String,\n",
            "}\n",
            "\n",
            "impl User {\n",
            "    pub fn new(name: &str) -> Self {\n",
            "        User { name: name.to_string() }\n",
            "    }\n",
            "}\n",
        )
        .to_string(),
    );
    fs.insert(
        "tests/user_test.rs".to_string(),
        concat!(
            "use myproject::user::User;\n",
            "\n",
            "#[test]\n",
            "fn test_new_user() {\n",
            "    let user = User::new(\"Alice\");\n",
            "    assert_eq!(user.name, \"Alice\");\n",
            "}\n",
        )
        .to_string(),
    );
    Arc::new(Mutex::new(fs))
}

// ─── ReadFileTask (IoTask + executor) ────────────────────────────────────────

/// An I/O task that reads a file from the fake filesystem.
///
/// This is pure data — the path to read. The FakeFilesystemExecutor holds
/// the actual filesystem handle and performs the read.
#[derive(Debug, Clone)]
struct ReadFileTask {
    path: String,
}

impl IoTask for ReadFileTask {
    type Input = ();
    type Output = String;
}

/// Executor that handles ReadFileTask by reading from the fake filesystem.
struct FakeFilesystemExecutor {
    fs: FakeFs,
}

impl TaskExecutor for FakeFilesystemExecutor {
    fn task_type_name(&self) -> &'static str {
        std::any::type_name::<ReadFileTask>()
    }

    fn execute_erased<'a>(
        &'a self,
        task_data: &'a dyn Any,
        input: AnyInput,
        _ctx: &'a Ctx,
    ) -> Pin<Box<dyn futures::Future<Output = Result<AnyOutput, BoxError>> + Send + 'a>> {
        let task = task_data
            .downcast_ref::<ReadFileTask>()
            .expect("executor received wrong task type");
        let _: () = *input.downcast::<()>().expect("input is not ()");

        let path = task.path.clone();
        let fs = self.fs.clone();

        Box::pin(async move {
            let guard = fs.lock().unwrap();
            let contents = guard
                .get(&path)
                .cloned()
                .ok_or_else(|| -> BoxError { format!("file not found: {path}").into() })?;
            Ok(Box::new(contents) as AnyOutput)
        })
    }
}

// ─── Domain types ────────────────────────────────────────────────────────────

/// Summary of a single file in the codebase.
#[derive(Debug, Clone)]
struct FileSummary {
    path: String,
    contents: String,
    has_struct: bool,
    has_impl: bool,
    has_tests: bool,
}

/// Complete analysis of the codebase.
#[derive(Debug, Clone)]
struct AnalysisResult {
    files: Vec<FileSummary>,
}

/// A single planned edit.
#[derive(Debug, Clone)]
struct PlannedEdit {
    path: String,
    description: String,
    new_contents: String,
}

/// The output of the planning phase.
#[derive(Debug, Clone)]
struct EditPlan {
    edits: Vec<PlannedEdit>,
}

/// Result of applying a single edit.
#[derive(Debug, Clone)]
struct EditResult {
    path: String,
    success: bool,
}

/// Result of running tests.
#[derive(Debug, Clone)]
struct TestResult {
    passed: bool,
    message: String,
}

/// Final validation result.
#[derive(Debug, Clone)]
struct ValidationResult {
    success: bool,
    summary: String,
}

// ─── AnalyzeTask ─────────────────────────────────────────────────────────────

/// Reads all relevant files in parallel, returns a codebase summary.
///
/// This task discovers which files exist and spawns one ReadFileTask per file.
/// The set of subtasks is not known until runtime — it depends on what files
/// the "codebase" contains.
#[derive(Debug, Clone)]
struct AnalyzeTask;

impl Task for AnalyzeTask {
    type Input = Vec<String>; // file paths to analyze
    type Output = AnalysisResult;
    type Error = SweError;

    async fn run(&self, input: Self::Input, ctx: &Ctx) -> Result<Self::Output, Self::Error> {
        // Spawn one ReadFileTask per file — all run in parallel.
        let mut handles = Vec::new();
        let mut paths = Vec::new();
        for path in input {
            let handle = ctx.spawn_dyn(erase_io(ReadFileTask { path: path.clone() }), Box::new(()));
            handles.push(handle);
            paths.push(path);
        }

        // Collect results and build summaries.
        let mut files = Vec::new();
        for (handle, path) in handles.into_iter().zip(paths) {
            let output = handle.await?;
            let contents = *output
                .downcast::<String>()
                .expect("ReadFileTask returns String");

            let has_struct = contents.contains("struct ");
            let has_impl = contents.contains("impl ");
            let has_tests = contents.contains("#[test]");

            files.push(FileSummary {
                path,
                contents,
                has_struct,
                has_impl,
                has_tests,
            });
        }

        Ok(AnalysisResult { files })
    }
}

// ─── PlanTask ────────────────────────────────────────────────────────────────

/// Decides what edits to make based on the analysis.
///
/// A real agent would call an LLM here. We simulate the decision deterministically:
/// - Add a `greet()` method to `src/user.rs`
/// - Add a test for `greet()` to `tests/user_test.rs`
#[derive(Debug, Clone)]
struct PlanTask;

impl Task for PlanTask {
    type Input = AnalysisResult;
    type Output = EditPlan;
    type Error = SweError;

    async fn run(&self, input: Self::Input, _ctx: &Ctx) -> Result<Self::Output, Self::Error> {
        let mut edits = Vec::new();

        // Find the user module source.
        let user_file = input
            .files
            .iter()
            .find(|f| f.path == "src/user.rs")
            .ok_or_else(|| SweError("src/user.rs not found in analysis".to_string()))?;

        // Plan edit: add greet() method to User impl block.
        let new_user_contents = user_file.contents.replace(
            "    }\n}\n",
            concat!(
                "    }\n",
                "\n",
                "    pub fn greet(&self) -> String {\n",
                "        format!(\"Hello, {}!\", self.name)\n",
                "    }\n",
                "}\n",
            ),
        );
        edits.push(PlannedEdit {
            path: "src/user.rs".to_string(),
            description: "Add greet() method to User".to_string(),
            new_contents: new_user_contents,
        });

        // Find the test file.
        let test_file = input
            .files
            .iter()
            .find(|f| f.path == "tests/user_test.rs")
            .ok_or_else(|| SweError("tests/user_test.rs not found in analysis".to_string()))?;

        // Plan edit: add test for greet().
        let new_test_contents = format!(
            "{}\n#[test]\nfn test_greet() {{\n    let user = User::new(\"Bob\");\n    assert_eq!(user.greet(), \"Hello, Bob!\");\n}}\n",
            test_file.contents.trim_end()
        );
        edits.push(PlannedEdit {
            path: "tests/user_test.rs".to_string(),
            description: "Add test for greet()".to_string(),
            new_contents: new_test_contents,
        });

        Ok(EditPlan { edits })
    }
}

// ─── EditTask ────────────────────────────────────────────────────────────────

/// Applies a single planned edit to the fake filesystem.
#[derive(Debug, Clone)]
struct EditTask {
    fs: FakeFs,
}

impl Task for EditTask {
    type Input = PlannedEdit;
    type Output = EditResult;
    type Error = SweError;

    async fn run(&self, input: Self::Input, _ctx: &Ctx) -> Result<Self::Output, Self::Error> {
        let mut guard = self.fs.lock().unwrap();
        guard.insert(input.path.clone(), input.new_contents);
        Ok(EditResult {
            path: input.path,
            success: true,
        })
    }
}

// ─── TestTask ────────────────────────────────────────────────────────────────

/// Validates that the edits are consistent — checks that functions referenced
/// in tests exist in the source.
#[derive(Debug, Clone)]
struct TestTask {
    fs: FakeFs,
}

impl Task for TestTask {
    type Input = Vec<EditResult>;
    type Output = TestResult;
    type Error = SweError;

    async fn run(&self, _input: Self::Input, _ctx: &Ctx) -> Result<Self::Output, Self::Error> {
        let guard = self.fs.lock().unwrap();

        let source = guard
            .get("src/user.rs")
            .ok_or_else(|| SweError("src/user.rs missing after edits".to_string()))?;
        let tests = guard
            .get("tests/user_test.rs")
            .ok_or_else(|| SweError("tests/user_test.rs missing after edits".to_string()))?;

        // Check: if tests reference greet(), source must define it.
        if tests.contains("greet()") && !source.contains("fn greet(") {
            return Ok(TestResult {
                passed: false,
                message: "tests reference greet() but source does not define it".to_string(),
            });
        }

        // Check: if tests reference User::new, source must define it.
        if tests.contains("User::new") && !source.contains("fn new(") {
            return Ok(TestResult {
                passed: false,
                message: "tests reference User::new but source does not define it".to_string(),
            });
        }

        Ok(TestResult {
            passed: true,
            message: "all tests pass".to_string(),
        })
    }
}

// ─── ValidateTask ────────────────────────────────────────────────────────────

/// Final validation: checks that tests passed and all edits succeeded.
#[derive(Debug, Clone)]
struct ValidateTask;

impl Task for ValidateTask {
    type Input = TestResult;
    type Output = ValidationResult;
    type Error = SweError;

    async fn run(&self, input: Self::Input, _ctx: &Ctx) -> Result<Self::Output, Self::Error> {
        if input.passed {
            Ok(ValidationResult {
                success: true,
                summary: format!("All changes validated: {}", input.message),
            })
        } else {
            Ok(ValidationResult {
                success: false,
                summary: format!("Validation failed: {}", input.message),
            })
        }
    }
}

// ─── SolveTask (orchestrator) ────────────────────────────────────────────────

/// The top-level orchestrator. Receives a task description and dynamically
/// decomposes it into analysis, planning, editing, testing, and validation.
///
/// The decomposition tree is not known upfront — it emerges from the data:
/// - AnalyzeTask discovers which files exist
/// - PlanTask decides what edits to make based on analysis
/// - EditTasks run in parallel (one per planned edit)
/// - TestTask depends on all edits completing
/// - ValidateTask depends on test results
#[derive(Debug, Clone)]
struct SolveTask {
    fs: FakeFs,
}

impl Task for SolveTask {
    type Input = String; // task description
    type Output = ValidationResult;
    type Error = SweError;

    async fn run(&self, input: Self::Input, ctx: &Ctx) -> Result<Self::Output, Self::Error> {
        println!("  [solve] Task: {input}");

        // Phase 1: Analyze — discover what files exist and read them.
        // The list of files comes from the filesystem; in a real agent,
        // this would be a directory listing or search.
        let file_paths: Vec<String> = {
            let guard = self.fs.lock().unwrap();
            guard.keys().cloned().collect()
        };
        println!("  [solve] Phase 1: Analyzing {} files", file_paths.len());

        let analysis: AnalysisResult = ctx.spawn(AnalyzeTask, file_paths).await?;

        println!("  [solve] Analysis complete:");
        for f in &analysis.files {
            println!(
                "          {} (struct={}, impl={}, tests={})",
                f.path, f.has_struct, f.has_impl, f.has_tests
            );
        }

        // Phase 2: Plan — decide what edits to make.
        println!("  [solve] Phase 2: Planning edits");
        let plan: EditPlan = ctx.spawn(PlanTask, analysis).await?;

        println!("  [solve] Plan: {} edits", plan.edits.len());
        for edit in &plan.edits {
            println!("          {} — {}", edit.path, edit.description);
        }

        // Phase 3: Edit — apply all edits in parallel.
        println!("  [solve] Phase 3: Applying edits in parallel");
        let edit_task = EditTask {
            fs: self.fs.clone(),
        };
        let edit_handles = ctx.spawn_all(edit_task, plan.edits);

        let mut edit_results = Vec::with_capacity(edit_handles.len());
        for handle in edit_handles {
            edit_results.push(handle.await?);
        }

        for result in &edit_results {
            println!("          {} — success={}", result.path, result.success);
        }

        // Phase 4: Test — validate edits are consistent.
        println!("  [solve] Phase 4: Running tests");
        let test_result: TestResult = ctx
            .spawn(
                TestTask {
                    fs: self.fs.clone(),
                },
                edit_results,
            )
            .await?;

        println!(
            "  [solve] Tests: passed={}, {}",
            test_result.passed, test_result.message
        );

        // Phase 5: Validate — final check.
        println!("  [solve] Phase 5: Validating");
        let validation: ValidationResult = ctx.spawn(ValidateTask, test_result).await?;

        println!("  [solve] Result: {}", validation.summary);

        Ok(validation)
    }
}

// ─── Exec graph printer ─────────────────────────────────────────────────────

fn print_exec_tree(runtime: &Runtime) {
    let mut nodes = runtime.exec_graph().snapshot();
    nodes.sort_by_key(|n| n.id);

    println!("\nDecomposition tree ({} nodes):", nodes.len());

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

    let roots: Vec<_> = nodes.iter().filter(|n| n.parent.is_none()).collect();
    for root in &roots {
        print_node(root, &nodes, 1);
    }
}

// ─── Main ────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    let fs = initial_filesystem();

    // Build the runtime with:
    // 1. A FakeFilesystemExecutor for ReadFileTask I/O
    // 2. A scaffold that logs every task spawn (simulating human oversight)
    let runtime = Runtime::new()
        .with_executor(FakeFilesystemExecutor { fs: fs.clone() })
        .with_scaffold(Scaffold::new(|task: &dyn DynTask| -> SharedDynTask {
            // Shorten the type name for display.
            let short = task
                .type_name()
                .rsplit("::")
                .next()
                .unwrap_or(task.type_name());
            eprintln!("  [scaffold] spawning: {short}");
            Arc::from(task.clone_box())
        }));

    println!("=== SWE Agent Simulation ===\n");

    // Print initial filesystem state.
    {
        let guard = fs.lock().unwrap();
        println!("Initial filesystem:");
        let mut paths: Vec<_> = guard.keys().collect();
        paths.sort();
        for path in paths {
            println!("  {path}");
        }
        println!();
    }

    // Run the agent.
    let result = runtime
        .run(
            SolveTask { fs: fs.clone() },
            "Add a greeting function to the user module and update tests".to_string(),
        )
        .await
        .unwrap();

    assert!(result.success, "agent should succeed");

    // Print the full decomposition tree from the exec graph.
    print_exec_tree(&runtime);

    // Print the final filesystem state to show the edits.
    println!("\nFinal filesystem:");
    {
        let guard = fs.lock().unwrap();
        let mut paths: Vec<_> = guard.keys().collect();
        paths.sort();
        for path in &paths {
            println!("--- {} ---", path);
            println!("{}", guard[*path]);
        }
    }

    // Verify the exec graph structure.
    let graph = runtime.exec_graph();
    let all_nodes = graph.snapshot();
    let completed = graph.completed_nodes();
    let failed = graph.failed_nodes();

    println!("Exec graph summary:");
    println!("  Total nodes:     {}", all_nodes.len());
    println!("  Completed:       {}", completed.len());
    println!("  Failed:          {}", failed.len());

    assert_eq!(failed.len(), 0, "no tasks should fail");
    assert_eq!(
        completed.len(),
        all_nodes.len(),
        "all tasks should complete"
    );

    // Verify the tree shape: SolveTask spawned AnalyzeTask, PlanTask,
    // EditTasks, TestTask, and ValidateTask.
    let root = all_nodes
        .iter()
        .find(|n| n.parent.is_none())
        .expect("should have a root node");
    let root_children = graph.get_children(root.id);
    println!(
        "  Root children:   {} (AnalyzeTask + PlanTask + EditTasks + TestTask + ValidateTask)",
        root_children.len()
    );
    // 5 children: AnalyzeTask, PlanTask, 2x EditTask, TestTask, ValidateTask = 6
    // But PlanTask is separate from AnalyzeTask
    assert!(
        root_children.len() >= 5,
        "root should have at least 5 children, got {}",
        root_children.len()
    );

    // Verify AnalyzeTask spawned ReadFileTask children (one per file).
    let analyze_node = all_nodes
        .iter()
        .find(|n| n.task_type.contains("AnalyzeTask"))
        .expect("should have an AnalyzeTask node");
    let read_children = graph.get_children(analyze_node.id);
    println!(
        "  AnalyzeTask children: {} ReadFileTasks",
        read_children.len()
    );
    assert_eq!(
        read_children.len(),
        3,
        "AnalyzeTask should spawn 3 ReadFileTasks"
    );

    // Verify the ReadFileTask nodes are IoTasks (their type contains ReadFileTask).
    for child_id in &read_children {
        let child = graph.get(*child_id).unwrap();
        assert!(
            child.task_type.contains("ReadFileTask"),
            "AnalyzeTask children should be ReadFileTasks, got {}",
            child.task_type
        );
    }

    println!("\nswe_agent: ok");
}
