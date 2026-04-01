
//! # Mini Agent Loop + WASM Sandbox + Session + Memory + Compaction — IronClaw Core Distilled
//!
//! Builds on mini-agent-memory by adding memory deduplication, extraction quality
//! improvements, and context compaction to prevent prompt token bloat.
//!
//! ## Evolution Path
//!
//! ```text
//! 1. mini-agent-loop           → Minimal agent (no sandbox, no state)
//! 2. mini-agent-wasm           → + WASM sandbox (secure tool execution)
//! 3. mini-agent-wasm-session   → + Session/Thread/Turn (short-term memory)
//! 4. mini-agent-memory         → + Workspace memory (long-term memory)
//! 5. mini-agent-compress (THIS)→ + Memory dedup + quality + compaction
//! ```
//!
//! ## What's new compared to mini-agent-memory:
//!
//! | mini-agent-memory              | mini-agent-compress                      |
//! |-------------------------------|------------------------------------------|
//! | No memory deduplication        | Similarity-based dedup before append     |
//! | Questions stored as memories   | Question filtering in extraction         |
//! | Unbounded MEMORY.md injection  | Truncated to max_memory_words            |
//! | No context compaction          | ContextMonitor + truncation compaction   |
//! | LLM + auto both write same     | Skip auto-extract if LLM wrote memory    |
//! | No prompt guidance on dedup    | System prompt tells LLM to skip dupes    |
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │                        Host Process                                 │
//! │                                                                     │
//! │  ┌──────────┐    ┌──────────────────────────────────────────────┐  │
//! │  │  stdin    │───▶│  Session Manager                             │  │
//! │  │  stdout   │◀──│    └── Session (user: "cli")                 │  │
//! │  └──────────┘    │        ├── Thread #1 (active)                │  │
//! │                  │        │   ├── Turn 1: "Hi" → "Hello!"       │  │
//! │                  │        │   └── Turn 2: "Remember X" → [mem]  │  │
//! │                  │        └── Thread #2                          │  │
//! │                  └──────────────┬───────────────────────────────┘  │
//! │                                 │                                   │
//! │                                 │ messages from Turn history         │
//! │                                 ▼                                   │
//! │  ┌──────────────────────────────────────────────────────────────┐  │
//! │  │  Agentic Loop                                                │  │
//! │  │    LLM ←→ Tool Execution ←→ WASM Sandbox + Native Tools     │  │
//! │  └──────────────────────────────────────────────────────────────┘  │
//! │                                 │                                   │
//! │                                 │ memory_write / memory_search       │
//! │                                 ▼                                   │
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
//! | `llm::provider`     | `src/llm/provider.rs`                    | LLM abstraction trait          |
//! | `llm` (types)       | `src/llm/provider.rs`                    | ChatMessage, ToolCall, etc.    |
//! | `llm::openai`       | (replaces real provider configs)         | OpenAI-compatible provider     |
//! | `tools`             | `src/tools/tool.rs` + `src/wasm/sandbox` | Tool trait & WASM sandbox      |
//! | `tools::wasm`       | `src/wasm/sandbox.rs`                    | WASM sandbox infrastructure    |
//! | `tools::memory_tools` | `src/tools/builtin/memory.rs`          | Native memory tools (NEW)      |
//! | `session`           | `src/agent/session.rs`                   | Session/Thread/Turn model      |
//! | `memory`            | `src/workspace/`                         | Persistent memory store (NEW)  |
//! | `agent`             | `src/agent/agentic_loop.rs` + `context_monitor.rs` + `compaction.rs` | Core loop + compaction |
//! | `commands`          | (new)                                    | CLI command handling           |
//! | `utils`             | (new)                                    | Shared utilities               |

mod agent;
mod commands;
mod llm;
mod memory;
mod session;
mod tools;
mod utils;

use std::io::{self, IsTerminal, Write};
use std::sync::Arc;

use tokio::sync::RwLock;
use tracing::{debug, info, trace, error};

use agent::AgenticLoopConfig;
use commands::{handle_model_command, handle_session_command, handle_user_input, CommandResult};
use llm::{HaiConfig, LlmProvider, OpenAiCompatibleProvider};
use memory::MemoryStore;
use session::Session;
use tools::{ToolRegistry, WasmTool, WasmToolEngine, MemorySearchTool, MemoryWriteTool, MemoryReadTool};

// ============================================================================
// Main — The Entry Point with Session + Memory Management
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

    info!("🚀 Starting Mini Agent + WASM Sandbox + Session + Memory — tracing initialized");

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

    // ── Step 3: Initialize Memory Store (NEW) ──
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

        // Native memory tools (NEW)
        r.register(Box::new(MemorySearchTool::new(Arc::clone(&memory_store))));
        r.register(Box::new(MemoryWriteTool::new(Arc::clone(&memory_store))));
        r.register(Box::new(MemoryReadTool::new(Arc::clone(&memory_store))));

        r
    };

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

    // ── Step 6: Build system prompt with memory ──
    let loop_config = AgenticLoopConfig::default();
    info!(max_iterations = loop_config.max_iterations, "⚙️  Agentic loop config");

    // ── Step 7: Main loop ──
    info!("🔄 Entering main input loop — waiting for user input...");

    loop {
        // Show thread info in prompt
        let thread_info = session
            .active_thread()
            .map(|t| format!("T{}:{}", t.turns.len() + 1, &t.id.to_string()[..4]))
            .unwrap_or_else(|| "?".to_string());

        print!("\n🧑 [{}] You: ", thread_info);
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

        // ── Build system prompt with current memory (refreshed each turn!) ──
        // This is critical: MEMORY.md content may have changed since the last
        // turn (e.g., via memory_write tool call), so we rebuild it each time.
        //
        // IMPROVED: Use memory_content_truncated() to cap the injected memory
        // size, preventing prompt token bloat from unbounded memory growth.
        let memory_content = {
            let store = memory_store.read().await;
            store.memory_content_truncated(loop_config.max_memory_words)
        };
        let system_prompt = agent::build_system_prompt_with_memory(&current_model, &memory_content);

        // ── Process user input through Session/Thread/Turn + Memory pipeline ──
        debug!(
            system_prompt_model = %current_model,
            "Building system prompt for current model (with memory injection)"
        );
        handle_user_input(
            &mut session,
            llm.as_ref(),
            &registry,
            &memory_store,
            &system_prompt,
            &loop_config,
            input,
        )
        .await;
    }
}

/// Print the startup banner.
fn print_banner() {
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║  Mini Agent + WASM + Session + Memory + Compaction         ║");
    println!("║  IronClaw Core Distilled — Optimized Memory Edition        ║");
    println!("╠══════════════════════════════════════════════════════════════╣");
    println!("║                                                            ║");
    println!("║  IMPROVED: Memory dedup + quality + context compaction!    ║");
    println!("║    • Memory dedup: similarity check before append          ║");
    println!("║    • Quality: question filtering, response analysis        ║");
    println!("║    • Compaction: auto-truncate when context grows large    ║");
    println!("║    • Token control: MEMORY.md capped at max_memory_words  ║");
    println!("║    • Smart skip: no auto-extract if LLM wrote memory      ║");
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
    println!("║  Other commands:                                           ║");
    println!("║    /model <name>     Switch LLM model                      ║");
    println!("║    /models           List available models                  ║");
    println!("║    quit              Exit                                   ║");
    println!("║                                                            ║");
    println!("║  Try long-term memory:                                     ║");
    println!("║    1. \"Remember that my favorite color is blue\"            ║");
    println!("║    2. \"What is 42 + 58?\"                                   ║");
    println!("║    3. quit → restart → \"What's my favorite color?\"         ║");
    println!("║    4. /memory  (see what's been saved)                     ║");
    println!("║                                                            ║");
    println!("║  Config: ./HAI_WOA.json (or ~/HAI_WOA.json)                ║");
    println!("║  Workspace: ./workspace/ (persistent memory files)          ║");
    println!("║                                                            ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();
}
