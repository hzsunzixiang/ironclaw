//! # Mini Agent Loop + WASM Sandbox + Session + Memory + Multi-Agent — IronClaw Core Distilled
//!
//! Builds on mini-agent-compress by adding multi-agent support:
//! - LoopDelegate trait for shared agentic loop engine
//! - ChatDelegate for interactive chat sessions
//! - JobDelegate for background job execution
//! - Scheduler for parallel job management
//! - Router for command dispatching
//!
//! ## Evolution Path
//!
//! ```text
//! 1. mini-agent-loop           → Minimal agent (no sandbox, no state)
//! 2. mini-agent-wasm           → + WASM sandbox (secure tool execution)
//! 3. mini-agent-wasm-session   → + Session/Thread/Turn (short-term memory)
//! 4. mini-agent-memory         → + Workspace memory (long-term memory)
//! 5. mini-agent-compress       → + Memory dedup + quality + compaction
//! 6. mini-agent-multiagent (THIS) → + Multi-agent: LoopDelegate + Scheduler
//! ```
//!
//! ## What's new compared to mini-agent-compress:
//!
//! | mini-agent-compress            | mini-agent-multiagent                    |
//! |-------------------------------|------------------------------------------|
//! | Hardcoded agentic loop         | LoopDelegate trait + shared engine       |
//! | Single execution path          | ChatDelegate + JobDelegate               |
//! | No background jobs             | Scheduler + /job command                 |
//! | No message routing             | Router dispatches to Chat vs Job         |
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │                        Host Process                                 │
//! │                                                                     │
//! │  ┌──────────┐    ┌──────────────────────────────────────────────┐  │
//! │  │  stdin    │───▶│  Router                                      │  │
//! │  │  stdout   │◀──│    ├── UserInput → ChatDelegate (foreground)  │  │
//! │  └──────────┘    │    ├── /job      → Scheduler → JobDelegate   │  │
//! │                  │    ├── /jobs     → List running jobs          │  │
//! │                  │    ├── /status   → Check job status           │  │
//! │                  │    └── /cancel   → Cancel running job         │  │
//! │                  └──────────────────────────────────────────────┘  │
//! │                                                                     │
//! │  ┌──────────────────────────────────────────────────────────────┐  │
//! │  │  Shared Agentic Loop Engine (agentic_loop.rs)                │  │
//! │  │    ├── ChatDelegate (interactive, session-aware)              │  │
//! │  │    └── JobDelegate  (background, independent)                │  │
//! │  │                                                              │  │
//! │  │    LLM ←→ Tool Execution ←→ WASM Sandbox + Native Tools     │  │
//! │  └──────────────────────────────────────────────────────────────┘  │
//! │                                                                     │
//! │  ┌──────────────────────────────────────────────────────────────┐  │
//! │  │  Scheduler (scheduler.rs)                                    │  │
//! │  │    ├── Job #1 (Worker + mpsc channel)                        │  │
//! │  │    ├── Job #2 (Worker + mpsc channel)                        │  │
//! │  │    └── max_parallel_jobs = 3                                 │  │
//! │  └──────────────────────────────────────────────────────────────┘  │
//! │                                                                     │
//! │  ┌──────────────────────────────────────────────────────────────┐  │
//! │  │  Memory Store (workspace/)                                   │  │
//! │  │    ├── MEMORY.md          ← injected into system prompt      │  │
//! │  │    ├── daily/2026-04-01.md ← session logs                   │  │
//! │  │    └── projects/...       ← arbitrary workspace files        │  │
//! │  └──────────────────────────────────────────────────────────────┘  │
//! └─────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Module Structure:
//!
//! | Module              | IronClaw source                          | Purpose                        |
//! |---------------------|------------------------------------------|--------------------------------|
//! | `agentic_loop`      | `src/agent/agentic_loop.rs`              | Shared loop engine + LoopDelegate trait |
//! | `agent`             | `src/agent/dispatcher.rs` + `thread_ops.rs` | ChatDelegate + process_user_input |
//! | `scheduler`         | `src/agent/scheduler.rs` + `worker/job.rs` | Scheduler + JobDelegate        |
//! | `router`            | `src/agent/router.rs`                    | Command routing                |
//! | `llm`               | `src/llm/provider.rs`                    | LLM abstraction trait          |
//! | `tools`             | `src/tools/tool.rs` + `src/wasm/sandbox` | Tool trait & WASM sandbox      |
//! | `session`           | `src/agent/session.rs`                   | Session/Thread/Turn model      |
//! | `memory`            | `src/workspace/`                         | Persistent memory store        |
//! | `commands`          | (new)                                    | CLI command handling           |
//! | `utils`             | (new)                                    | Shared utilities               |

mod agent;
pub(crate) mod agentic_loop;
mod commands;
mod llm;
mod memory;
mod router;
mod scheduler;
mod session;
mod tools;
mod utils;

use std::io::{self, IsTerminal, Write};
use std::sync::Arc;

use tokio::sync::RwLock;
use tracing::{debug, info, trace, error};

use agentic_loop::AgenticLoopConfig;
use commands::{handle_model_command, handle_session_command, handle_job_command, handle_user_input, CommandResult};
use llm::{HaiConfig, LlmProvider, OpenAiCompatibleProvider};
use memory::MemoryStore;
use router::{MessageIntent, Router};
use scheduler::Scheduler;
use session::Session;
use tools::{ToolRegistry, WasmTool, WasmToolEngine, MemorySearchTool, MemoryWriteTool, MemoryReadTool};

// ============================================================================
// Main — The Entry Point with Multi-Agent Support
// ============================================================================

#[tokio::main]
async fn main() {
    // ── Initialize tracing subscriber ──
    let default_filter = "mini_agent_memory=info,wasmtime=warn,cranelift=warn,regalloc=warn,wasi=warn,info";
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default_filter)),
        )
        .with_target(true)
        .with_thread_names(true)
        .with_level(true)
        .with_ansi(std::io::stderr().is_terminal())
        .init();

    info!("🚀 Starting Mini Agent + Multi-Agent — tracing initialized");

    print_banner();

    // ── Step 1: Load WASM tool ──
    let wasm_path = std::env::args().nth(1).unwrap_or_else(|| {
        "../guest/target/wasm32-wasip2/release/guest_tool.wasm".to_string()
    });

    info!(wasm_path = %wasm_path, "🔧 Loading WASM tool...");
    println!("🔧 Loading WASM tool from: {}", wasm_path);
    let wasm_engine = match WasmToolEngine::new(&wasm_path, 1_000_000) {
        Ok(engine) => {
            info!(
                tool_name = %engine.tool_name,
                tool_description = %engine.tool_description,
                fuel_limit = engine.fuel_limit,
                "✅ WASM tool loaded and compiled"
            );
            println!("✅ WASM tool loaded and compiled");
            println!("   Name: {}", engine.tool_name);
            println!("   Description: {}", engine.tool_description);
            println!("   Fuel limit: {} units per execution", engine.fuel_limit);
            engine
        }
        Err(e) => {
            error!(error = %e, "❌ Failed to load WASM tool");
            eprintln!("❌ Failed to load WASM tool: {}", e);
            eprintln!("   Make sure to build the guest first:");
            eprintln!("   cd ../guest && cargo build --target wasm32-wasip2 --release");
            std::process::exit(1);
        }
    };
    println!();

    // ── Step 2: Setup LLM provider ──
    info!("📂 Loading LLM configuration from HAI_WOA.json ...");
    let hai_config = HaiConfig::load().unwrap_or_else(|e| {
        error!("❌ Failed to load HAI_WOA.json: {}", e);
        eprintln!("❌ Failed to load HAI_WOA.json: {}", e);
        std::process::exit(1);
    });

    info!(
        model = %hai_config.model,
        base_url = %hai_config.base_url().unwrap_or_default(),
        "✅ Config loaded successfully"
    );

    println!(
        "✅ Loaded config from {}",
        if std::path::Path::new("HAI_WOA.json").exists() {
            "./HAI_WOA.json"
        } else {
            "~/HAI_WOA.json"
        }
    );
    println!("   Model: {}", hai_config.model);
    println!("   Base URL: {}", hai_config.base_url().unwrap_or_default());
    println!(
        "   Available shortcuts: {}",
        hai_config
            .list_models()
            .iter()
            .map(|(k, v)| format!("{} → {}", k, v))
            .collect::<Vec<_>>()
            .join(", ")
    );

    let mut current_model = hai_config.resolve_model(None);
    info!(current_model = %current_model, "🤖 Initial model selected");

    let make_provider =
        |model: &str, config: &HaiConfig| -> Result<Box<dyn LlmProvider>, String> {
            debug!(model = %model, "Creating new LLM provider instance");
            Ok(Box::new(OpenAiCompatibleProvider::new(
                config.api_key()?,
                config.base_url()?,
                model.to_string(),
            )))
        };

    let mut llm: Box<dyn LlmProvider> = make_provider(&current_model, &hai_config)
        .unwrap_or_else(|e| {
            error!("❌ Failed to create LLM provider: {}", e);
            eprintln!("❌ Failed to create LLM provider: {}", e);
            std::process::exit(1);
        });

    println!();

    // ── Step 3: Initialize Memory Store ──
    let workspace_dir = std::env::args().nth(2).unwrap_or_else(|| {
        "workspace".to_string()
    });
    let memory_store = Arc::new(RwLock::new(MemoryStore::new(&workspace_dir)));

    {
        let store = memory_store.read().await;
        let memory_content = store.memory_content();
        let word_count = memory_content.split_whitespace().count();
        info!(
            workspace = %workspace_dir,
            memory_words = word_count,
            "✅ Memory store initialized"
        );
        println!("✅ Memory store initialized");
        println!("   Workspace: {}", store.root_path().display());
        println!("   MEMORY.md: {} words", word_count);
    }
    println!();

    // ── Step 4: Register tools (WASM + Native Memory) ──
    let registry = {
        let mut r = ToolRegistry::new();

        // WASM sandboxed tool (calculator)
        r.register(Box::new(WasmTool {
            engine: Arc::new(wasm_engine),
        }));

        // Native memory tools
        r.register(Box::new(MemorySearchTool::new(Arc::clone(&memory_store))));
        r.register(Box::new(MemoryWriteTool::new(Arc::clone(&memory_store))));
        r.register(Box::new(MemoryReadTool::new(Arc::clone(&memory_store))));

        r
    };

    let registry = Arc::new(registry);

    info!(
        tools = ?registry.definitions().iter().map(|t| &t.name).collect::<Vec<_>>(),
        "🔧 Tool registry initialized (WASM + native memory tools)"
    );

    println!("✅ Tool registry ready:");
    for def in registry.definitions() {
        let tool_type = if def.name.starts_with("memory_") {
            "native"
        } else {
            "WASM"
        };
        println!("   📦 {} [{}] — {}", def.name, tool_type, def.description);
    }
    println!();

    // ── Step 5: Create Session ──
    let mut session = Session::new("cli-user");
    session.create_thread();
    info!(
        session_id = %&session.id.to_string()[..8],
        thread_id = %&session.active_thread.unwrap().to_string()[..8],
        "✅ Session created with initial thread"
    );
    println!(
        "✅ Session created: {}",
        &session.id.to_string()[..8]
    );
    println!(
        "   Thread: {}",
        &session.active_thread.unwrap().to_string()[..8]
    );
    println!();

    // ── Step 6: Create Scheduler (NEW — multi-agent support) ──
    let llm_for_scheduler: Arc<dyn LlmProvider> = Arc::from(
        make_provider(&current_model, &hai_config).unwrap_or_else(|e| {
            error!("❌ Failed to create LLM provider for scheduler: {}", e);
            eprintln!("❌ Failed to create LLM provider for scheduler: {}", e);
            std::process::exit(1);
        })
    );
    let scheduler = Scheduler::new(
        llm_for_scheduler,
        Arc::clone(&registry),
        Arc::clone(&memory_store),
        current_model.clone(),
    );
    info!("✅ Scheduler initialized (max 3 parallel jobs)");
    println!("✅ Scheduler initialized (max 3 parallel jobs)");
    println!();

    // ── Step 7: Configure agentic loop ──
    let loop_config = AgenticLoopConfig::default();
    info!(max_iterations = loop_config.max_iterations, "⚙️  Agentic loop config");

    // ── Step 8: Main loop with Router ──
    info!("🔄 Entering main input loop — waiting for user input...");

    loop {
        // Show thread info in prompt
        let thread_info = session
            .active_thread()
            .map(|t| format!("T{}:{}", t.turns.len() + 1, &t.id.to_string()[..4]))
            .unwrap_or_else(|| "?".to_string());

        let running_jobs = scheduler.running_count().await;
        let job_indicator = if running_jobs > 0 {
            format!(" 🔄{}", running_jobs)
        } else {
            String::new()
        };

        print!("\n🧑 [{}{}] You: ", thread_info, job_indicator);
        io::stdout().flush().unwrap();

        let mut input = String::new();
        match io::stdin().read_line(&mut input) {
            Ok(0) => break,
            Err(_) => break,
            _ => {}
        }
        let input = input.trim();

        if input.is_empty() {
            trace!("Empty input, skipping");
            continue;
        }

        info!(input = %input, "📝 User input received");

        // ── Session + Memory management commands ──
        match handle_session_command(input, &mut session, &current_model, &memory_store).await {
            CommandResult::Quit => {
                // Log final session to daily log
                let store = memory_store.read().await;
                let total_turns: usize = session.threads.values().map(|t| t.turns.len()).sum();
                let _ = store.append_daily_log(&format!(
                    "Session ended: {} thread(s), {} total turn(s)",
                    session.threads.len(),
                    total_turns
                ));
                info!(
                    thread_count = session.threads.len(),
                    total_turns = total_turns,
                    "👋 User requested exit"
                );
                break;
            }
            CommandResult::Continue => continue,
            CommandResult::NotACommand => {}
        }

        // ── LLM model commands ──
        if let Some(resolved) = handle_model_command(input, &hai_config, &current_model) {
            if resolved != current_model {
                info!(
                    old_model = %current_model,
                    new_model = %resolved,
                    "🔄 Switching model"
                );
                match make_provider(&resolved, &hai_config) {
                    Ok(p) => {
                        llm = p;
                        current_model = resolved;
                        info!(current_model = %current_model, "✅ Model switched");
                        println!("✅ Switched to model: {}", current_model);
                    }
                    Err(e) => {
                        error!(error = %e, "❌ Failed to switch model");
                        println!("❌ Failed to switch model: {}", e);
                    }
                }
            }
            continue;
        }

        // ── Route through Router (NEW — multi-agent dispatch) ──
        let intent = Router::route(input);
        info!(intent = ?intent, "🔀 Router intent: {:?}", intent);

        // Try job commands first
        if let Some(result) = handle_job_command(&intent, &scheduler).await {
            match result {
                CommandResult::Continue => continue,
                CommandResult::Quit => break,
                CommandResult::NotACommand => {} // Fall through
            }
        }

        // If it's a UserInput intent, process through chat pipeline
        if let MessageIntent::UserInput { content } = &intent {
            // Build system prompt with current memory (refreshed each turn!)
            let memory_content = {
                let store = memory_store.read().await;
                store.memory_content_truncated(loop_config.max_memory_words)
            };
            let system_prompt = agent::build_system_prompt_with_memory(&current_model, &memory_content);

            // Process user input through Session/Thread/Turn + Memory pipeline
            handle_user_input(
                &mut session,
                llm.as_ref(),
                &registry,
                &memory_store,
                &system_prompt,
                &loop_config,
                content,
            )
            .await;
        }
    }
}

/// Print the startup banner.
fn print_banner() {
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║  Mini Agent + WASM + Session + Memory + Multi-Agent        ║");
    println!("║  IronClaw Core Distilled — Multi-Agent Edition             ║");
    println!("╠══════════════════════════════════════════════════════════════╣");
    println!("║                                                            ║");
    println!("║  NEW: Multi-Agent support via LoopDelegate + Scheduler!    ║");
    println!("║    • LoopDelegate trait: shared agentic loop engine        ║");
    println!("║    • ChatDelegate: interactive chat (foreground)           ║");
    println!("║    • JobDelegate: background job execution                 ║");
    println!("║    • Scheduler: parallel job management (max 3)            ║");
    println!("║    • Router: command dispatching                           ║");
    println!("║                                                            ║");
    println!("║  Session commands:                                         ║");
    println!("║    /new              Create a new conversation thread      ║");
    println!("║    /threads          List all threads                      ║");
    println!("║    /switch <n>       Switch to thread #n                   ║");
    println!("║    /history          Show turn history                     ║");
    println!("║    /session          Show session info                     ║");
    println!("║                                                            ║");
    println!("║  Memory commands:                                          ║");
    println!("║    /memory           Show MEMORY.md content                ║");
    println!("║    /memory-search <q> Search workspace memory              ║");
    println!("║    /memory-tree      Show workspace file tree              ║");
    println!("║                                                            ║");
    println!("║  Job commands (NEW):                                       ║");
    println!("║    /job <desc>       Create a background job               ║");
    println!("║    /jobs             List all jobs                         ║");
    println!("║    /status <id>      Check job status                      ║");
    println!("║    /cancel <id>      Cancel a running job                  ║");
    println!("║                                                            ║");
    println!("║  Other commands:                                           ║");
    println!("║    /model <name>     Switch LLM model                      ║");
    println!("║    /models           List available models                  ║");
    println!("║    quit              Exit                                   ║");
    println!("║                                                            ║");
    println!("║  Try multi-agent:                                          ║");
    println!("║    1. \"What is 42 + 58?\"  (foreground chat)               ║");
    println!("║    2. /job Calculate 100 * 200 and save to memory          ║");
    println!("║    3. /jobs  (see running/completed jobs)                  ║");
    println!("║    4. /status <id>  (check job result)                     ║");
    println!("║                                                            ║");
    println!("║  Config: ./HAI_WOA.json (or ~/HAI_WOA.json)                ║");
    println!("║  Workspace: ./workspace/ (persistent memory files)          ║");
    println!("║                                                            ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();
}
