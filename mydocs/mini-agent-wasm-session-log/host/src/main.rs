//! # Mini Agent Loop + WASM Sandbox + Session — IronClaw Core Distilled
//!
//! Builds on mini-agent-wasm by adding the Session/Thread/Turn model.
//! This is the conversation management layer that IronClaw uses to track
//! multi-turn interactions.
//!
//! ```text
//! Session (per user)
//! └── Thread (per conversation — can have many)
//!     └── Turn (per request/response pair)
//!         ├── user_input: String
//!         ├── response: Option<String>
//!         ├── tool_calls: Vec<TurnToolCall>
//!         └── state: TurnState
//! ```
//!
//! ## What's new compared to mini-agent-wasm:
//!
//! | mini-agent-wasm                | mini-agent-wasm-session                  |
//! |-------------------------------|------------------------------------------|
//! | Single-turn (no history)       | Multi-turn with full conversation memory |
//! | No session concept             | Session → Thread → Turn hierarchy        |
//! | Fresh messages each input      | Messages rebuilt from Turn history        |
//! | No thread management           | /new, /threads, /switch, /history        |
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
//! | `session`           | `src/agent/session.rs`                   | Session/Thread/Turn model      |
//! | `agent`             | `src/agent/agentic_loop.rs`              | The core loop engine           |
//! | `commands`          | (new)                                    | CLI command handling           |
//! | `utils`             | (new)                                    | Shared utilities               |
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
//! │                  │        │   ├── Turn 2: "42+58?" → [calc] 100 │  │
//! │                  │        │   └── Turn 3: (in progress...)      │  │
//! │                  │        └── Thread #2                          │  │
//! │                  └──────────────┬───────────────────────────────┘  │
//! │                                 │                                   │
//! │                                 │ messages from Turn history         │
//! │                                 ▼                                   │
//! │  ┌──────────────────────────────────────────────────────────────┐  │
//! │  │  Agentic Loop                                                │  │
//! │  │    LLM ←→ Tool Execution ←→ WASM Sandbox                    │  │
//! │  └──────────────────────────────────────────────────────────────┘  │
//! └─────────────────────────────────────────────────────────────────────┘
//! ```

mod agent;
mod commands;
mod llm;
mod session;
mod tools;
mod utils;

use std::io::{self, IsTerminal, Write};
use std::sync::Arc;

use tracing::{debug, info, trace, error};

use agent::AgenticLoopConfig;
use commands::{handle_model_command, handle_session_command, handle_user_input, CommandResult};
use llm::{ChatMessage, HaiConfig, LlmProvider, OpenAiCompatibleProvider};
use session::Session;
use tools::{ToolRegistry, WasmTool, WasmToolEngine};

// ============================================================================
// Main — The Entry Point with Session Management
// ============================================================================
// Key differences from mini-agent-wasm:
//   1. Creates a Session at startup
//   2. Each user input goes through process_user_input() which manages Turns
//   3. Supports thread management commands (/new, /threads, /switch, /history)
//   4. Multi-turn: LLM sees full conversation history from previous turns
// ============================================================================

#[tokio::main]
async fn main() {
    // ── Initialize tracing subscriber ──
    // Control log level via RUST_LOG env var:
    //   RUST_LOG=mini_agent_wasm_session=trace  — maximum detail for OUR code only (recommended)
    //   RUST_LOG=mini_agent_wasm_session=debug  — detailed (requests, responses, tool calls)
    //   RUST_LOG=info                           — key events only (default)
    //   RUST_LOG=mini_agent_wasm_session::llm=trace  — only LLM module at trace level
    //
    // ⚠️  Avoid RUST_LOG=trace (without module filter) — wasmtime internals
    //    (type_registry, cranelift, regalloc) produce thousands of unreadable lines.
    //    Always scope trace/debug to our crate: mini_agent_wasm_session=trace
    //
    // Default filter: our crate at info, third-party noisy crates at warn
    let default_filter = "mini_agent_wasm_session=info,wasmtime=warn,cranelift=warn,regalloc=warn,wasi=warn,info";
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default_filter)),
        )
        .with_target(true)       // show module path (e.g. mini_agent_wasm_session::llm::openai)
        .with_thread_names(true) // show thread names
        .with_level(true)        // show log level
        .with_ansi(std::io::stderr().is_terminal()) // auto-detect: colors in terminal, plain text in file
        .init();

    info!("🚀 Starting Mini Agent Loop + WASM Sandbox + Session — tracing initialized");

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
            debug!(
                tool_schema = %serde_json::to_string_pretty(&engine.tool_schema).unwrap_or_default(),
                "WASM tool schema"
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
        eprintln!("   Please create HAI_WOA.json in the current directory or ~/HAI_WOA.json.");
        std::process::exit(1);
    });

    info!(
        model = %hai_config.model,
        base_url = %hai_config.base_url().unwrap_or_default(),
        "✅ Config loaded successfully"
    );
    debug!(
        available_models = ?hai_config.list_models(),
        "Available model shortcuts"
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

    // ── Step 3: Register WASM tool ──
    let registry = {
        let mut r = ToolRegistry::new();
        r.register(Box::new(WasmTool {
            engine: Arc::new(wasm_engine),
        }));
        r
    };

    info!(
        tools = ?registry.definitions().iter().map(|t| &t.name).collect::<Vec<_>>(),
        "🔧 Tool registry initialized (WASM sandboxed tools)"
    );
    debug!(
        tool_definitions = %serde_json::to_string_pretty(&registry.definitions().iter().map(|t| {
            serde_json::json!({
                "name": &t.name,
                "description": &t.description,
                "parameters": &t.parameters,
            })
        }).collect::<Vec<_>>()).unwrap_or_default(),
        "Full tool definitions (sent to LLM)"
    );

    println!("✅ Tool registry ready (WASM sandboxed tools)");
    for def in registry.definitions() {
        println!("   📦 {} — {}", def.name, def.description);
    }
    println!();

    // ── Step 4: Create Session ──
    // In IronClaw: SessionManager creates sessions per user
    // Here: single CLI user, one session
    let mut session = Session::new("cli-user");
    session.create_thread(); // Start with one thread
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

    // ── Step 5: System prompt (includes model identity, updated on /model switch) ──
    let make_system_prompt = |model: &str| -> ChatMessage {
        ChatMessage::system(format!(
            "You are a helpful assistant powered by the {} model. \
             You have access to a calculator tool. \
             When the user asks a math question, use the calculator tool to compute the answer. \
             For non-math questions, respond directly. \
             You have conversation memory — you can reference previous turns in this thread. \
             When asked about your identity, truthfully state that you are based on {}.",
            model, model
        ))
    };

    let loop_config = AgenticLoopConfig::default();
    info!(max_iterations = loop_config.max_iterations, "⚙️  Agentic loop config");

    // ── Step 6: Main loop ──
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

        // ── Session management commands ──
        match handle_session_command(input, &mut session, &current_model) {
            CommandResult::Quit => {
                info!(
                    thread_count = session.threads.len(),
                    total_turns = session.threads.values().map(|t| t.turns.len()).sum::<usize>(),
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

        // ── Process user input through Session/Thread/Turn pipeline ──
        debug!(
            system_prompt_model = %current_model,
            "Building system prompt for current model"
        );
        handle_user_input(
            &mut session,
            llm.as_ref(),
            &registry,
            &make_system_prompt(&current_model),
            &loop_config,
            input,
        )
        .await;
    }
}

/// Print the startup banner.
fn print_banner() {
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║   Mini Agent + WASM Sandbox + Session — IronClaw Distilled ║");
    println!("╠══════════════════════════════════════════════════════════════╣");
    println!("║                                                            ║");
    println!("║  NEW: Session/Thread/Turn conversation management!         ║");
    println!("║    • Multi-turn: LLM remembers your conversation history   ║");
    println!("║    • Multiple threads: start new conversations with /new   ║");
    println!("║    • Turn tracking: each Q&A pair is a recorded Turn       ║");
    println!("║                                                            ║");
    println!("║  Session commands:                                         ║");
    println!("║    /new              Create a new conversation thread      ║");
    println!("║    /threads          List all threads                      ║");
    println!("║    /switch <n>       Switch to thread #n                   ║");
    println!("║    /history          Show turn history                     ║");
    println!("║    /session          Show session info                     ║");
    println!("║                                                            ║");
    println!("║  Other commands:                                           ║");
    println!("║    /model <name>     Switch LLM model                      ║");
    println!("║    /models           List available models                  ║");
    println!("║    quit              Exit                                   ║");
    println!("║                                                            ║");
    println!("║  Config: ./HAI_WOA.json (or ~/HAI_WOA.json)                ║");
    println!("║                                                            ║");
    println!("║  Try multi-turn:                                           ║");
    println!("║    1. \"What is 42 + 58?\"                                   ║");
    println!("║    2. \"Now multiply that result by 3\"                      ║");
    println!("║    3. \"/history\" to see the conversation                   ║");
    println!("║                                                            ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();
}
