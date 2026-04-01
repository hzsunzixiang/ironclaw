
//! # CLI Command Handling
//!
//! Extracted from main.rs to keep the entry point clean.
//! Handles all `/` commands (session management, model switching, memory, jobs)
//! and delegates user input to the agentic loop.
//!
//! ## What's new compared to mini-agent-compress:
//!
//! | mini-agent-compress            | mini-agent-multiagent                    |
//! |-------------------------------|------------------------------------------|
//! | No job commands                | /job, /jobs, /status, /cancel             |
//! | No message routing             | Router dispatches to Chat vs Job          |

use crate::agent::process_user_input;
use crate::agentic_loop::AgenticLoopConfig;
use crate::llm::{ChatMessage, HaiConfig, LlmProvider};
use crate::memory::MemoryStore;
use crate::router::MessageIntent;
use crate::scheduler::Scheduler;
use crate::session::{Session, TurnState};
use crate::tools::ToolRegistry;
use crate::utils::truncate_str;

use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Result of handling a command — tells the main loop what to do next.
pub enum CommandResult {
    /// Command was handled, continue the main loop.
    Continue,
    /// User wants to quit.
    Quit,
    /// Not a command — this is user input for the agent.
    NotACommand,
}

/// Handle session management commands (/new, /threads, /switch, /history, /session).
/// Also handles memory commands (/memory, /memory-search, /memory-tree).
pub async fn handle_session_command(
    input: &str,
    session: &mut Session,
    current_model: &str,
    memory_store: &Arc<RwLock<MemoryStore>>,
) -> CommandResult {
    if input == "quit" || input == "exit" {
        info!(
            thread_count = session.threads.len(),
            total_turns = session.threads.values().map(|t| t.turns.len()).sum::<usize>(),
            "👋 User requested exit"
        );
        println!(
            "👋 Goodbye! Session had {} thread(s), {} total turn(s).",
            session.threads.len(),
            session
                .threads
                .values()
                .map(|t| t.turns.len())
                .sum::<usize>()
        );
        return CommandResult::Quit;
    }

    if input == "/new" {
        info!("📝 User requested new thread");
        let thread = session.create_thread();
        println!(
            "✅ New thread created: {}",
            &thread.id.to_string()[..8]
        );
        println!(
            "   Now in thread {} (0 turns)",
            &thread.id.to_string()[..8]
        );
        return CommandResult::Continue;
    }

    if input == "/threads" {
        let summaries = session.list_threads();
        debug!(thread_count = summaries.len(), "Listing threads");
        println!("\n📋 Threads ({}):", summaries.len());
        for (i, s) in summaries.iter().enumerate() {
            let active = if s.is_active { " ← active" } else { "" };
            let preview = s.first_input.as_deref().unwrap_or("(empty)");
            println!(
                "   #{} [{}] {} turn(s) — \"{}\"{}",
                i + 1,
                &s.id.to_string()[..8],
                s.turn_count,
                preview,
                active
            );
        }
        return CommandResult::Continue;
    }

    if input.starts_with("/switch ") {
        let idx_str = input.strip_prefix("/switch ").unwrap().trim();
        debug!(switch_target = %idx_str, "User requested thread switch");
        if let Ok(idx) = idx_str.parse::<usize>() {
            let summaries = session.list_threads();
            if idx >= 1 && idx <= summaries.len() {
                let thread_id = summaries[idx - 1].id;
                if session.switch_thread(thread_id) {
                    let thread = session.active_thread().unwrap();
                    info!(
                        thread_idx = idx,
                        thread_id = %&thread_id.to_string()[..8],
                        turn_count = thread.turns.len(),
                        "✅ Switched to thread"
                    );
                    println!(
                        "✅ Switched to thread #{} [{}] ({} turns)",
                        idx,
                        &thread_id.to_string()[..8],
                        thread.turns.len()
                    );
                } else {
                    warn!(thread_id = %&thread_id.to_string()[..8], "Failed to switch thread");
                    println!("❌ Failed to switch thread");
                }
            } else {
                warn!(idx = idx, max = summaries.len(), "Invalid thread number");
                println!("❌ Invalid thread number. Use /threads to see available threads.");
            }
        } else {
            warn!(input = %idx_str, "Invalid /switch argument");
            println!("❌ Usage: /switch <number>");
        }
        return CommandResult::Continue;
    }

    if input == "/history" {
        debug!("User requested turn history");
        print_history(session);
        return CommandResult::Continue;
    }

    if input == "/session" {
        debug!("User requested session info");
        print_session_info(session, current_model);
        return CommandResult::Continue;
    }

    // ── Memory commands ──────────────────────────────────────

    if input == "/memory" {
        info!("📂 User requested memory content");
        let store = memory_store.read().await;
        let content = store.memory_content();
        println!("\n🧠 Long-Term Memory (MEMORY.md):");
        println!("─────────────────────────────────");
        if content.trim().is_empty() {
            println!("   (empty — no memories saved yet)");
        } else {
            for line in content.lines() {
                println!("   {}", line);
            }
        }
        println!("─────────────────────────────────");
        println!("   📂 Workspace: {}", store.root_path().display());
        return CommandResult::Continue;
    }

    if input.starts_with("/memory-search ") {
        let query = input.strip_prefix("/memory-search ").unwrap().trim();
        if query.is_empty() {
            println!("❌ Usage: /memory-search <query>");
            return CommandResult::Continue;
        }
        info!(query = %query, "🔍 User requested memory search");
        let store = memory_store.read().await;
        let results = store.search(query, 5);
        println!("\n🔍 Memory search results for \"{}\":", query);
        if results.is_empty() {
            println!("   No results found.");
        } else {
            for (i, r) in results.iter().enumerate() {
                println!(
                    "   #{} [score: {:.2}] {} — {}",
                    i + 1,
                    r.score,
                    r.path,
                    truncate_str(&r.content.replace('\n', " "), 60)
                );
            }
        }
        return CommandResult::Continue;
    }

    if input == "/memory-tree" {
        info!("🌳 User requested memory tree");
        let store = memory_store.read().await;
        let tree = store.tree();
        println!("\n🌳 Workspace tree:");
        if tree.is_empty() {
            println!("   (empty)");
        } else {
            for entry in &tree {
                println!("   {}", entry);
            }
        }
        return CommandResult::Continue;
    }

    CommandResult::NotACommand
}

/// Handle model management commands (/models, /model <name>).
pub fn handle_model_command(
    input: &str,
    hai_config: &HaiConfig,
    current_model: &str,
) -> Option<String> {
    if input == "/models" {
        println!("\n📋 Available models:");
        println!("   (default) → {}", hai_config.model);
        for (shortcut, model_id) in hai_config.list_models() {
            let marker = if current_model == model_id {
                " ← current"
            } else {
                ""
            };
            println!("   {} → {}{}", shortcut, model_id, marker);
        }
        println!("   Current: {}", current_model);
        return Some(current_model.to_string());
    }

    if input.starts_with("/model ") {
        let model_name = input.strip_prefix("/model ").unwrap().trim();
        let resolved = hai_config.resolve_model(Some(model_name));
        return Some(resolved);
    }

    None
}

/// Handle job-related commands via the Scheduler.
///
/// Maps to: IronClaw's Router → Scheduler dispatch flow.
pub async fn handle_job_command(
    intent: &MessageIntent,
    scheduler: &Scheduler,
) -> Option<CommandResult> {
    match intent {
        MessageIntent::CreateJob { description } => {
            println!("\n🚀 Creating background job...");
            match scheduler.dispatch_job(description).await {
                Ok(job_id) => {
                    let short_id = &job_id.to_string()[..8];
                    println!("✅ Job created: {} (id: {})", description, short_id);
                    println!("   Use /jobs to list all jobs");
                    println!("   Use /status {} to check progress", short_id);
                    println!("   Use /cancel {} to stop it", short_id);
                }
                Err(e) => {
                    println!("❌ Failed to create job: {}", e);
                }
            }
            Some(CommandResult::Continue)
        }

        MessageIntent::ListJobs => {
            let jobs = scheduler.list_jobs().await;
            let running = scheduler.running_count().await;
            println!("\n📋 Jobs ({} total, {} running):", jobs.len(), running);
            if jobs.is_empty() {
                println!("   No jobs yet. Use /job <description> to create one.");
            } else {
                for job in &jobs {
                    let duration = job.completed_at
                        .map(|c| {
                            let dur = c.signed_duration_since(job.created_at);
                            format!(" ({}s)", dur.num_seconds())
                        })
                        .unwrap_or_default();

                    println!(
                        "   [{}] {} — \"{}\"{}",
                        job.short_id,
                        job.state,
                        truncate_str(&job.description, 40),
                        duration
                    );

                    if let Some(ref result) = job.result {
                        println!("         Result: {}", truncate_str(result, 60));
                    }
                    if let Some(ref error) = job.error {
                        println!("         Error: {}", error);
                    }
                }
            }
            Some(CommandResult::Continue)
        }

        MessageIntent::CheckJobStatus { job_id } => {
            match scheduler.find_job_by_short_id(job_id).await {
                Some(job) => {
                    println!("\n📊 Job Status:");
                    println!("   ID: {}", job.id);
                    println!("   Description: {}", job.description);
                    println!("   State: {}", job.state);
                    println!("   Created: {}", job.created_at.format("%H:%M:%S"));
                    if let Some(completed) = job.completed_at {
                        println!("   Completed: {}", completed.format("%H:%M:%S"));
                        let duration = completed.signed_duration_since(job.created_at);
                        println!("   Duration: {}s", duration.num_seconds());
                    }
                    println!("   Tool calls: {}", job.tool_calls_count);
                    if let Some(ref result) = job.result {
                        println!("   Result: {}", result);
                    }
                    if let Some(ref error) = job.error {
                        println!("   Error: {}", error);
                    }
                }
                None => {
                    println!("❌ Job not found: {}", job_id);
                    println!("   Use /jobs to list all jobs.");
                }
            }
            Some(CommandResult::Continue)
        }

        MessageIntent::CancelJob { job_id } => {
            match scheduler.find_job_by_short_id(job_id).await {
                Some(job) => {
                    match scheduler.cancel_job(job.id).await {
                        Ok(()) => {
                            println!("🛑 Cancel signal sent to job {}", job.short_id);
                        }
                        Err(e) => {
                            println!("❌ Failed to cancel job: {}", e);
                        }
                    }
                }
                None => {
                    println!("❌ Job not found: {}", job_id);
                    println!("   Use /jobs to list all jobs.");
                }
            }
            Some(CommandResult::Continue)
        }

        MessageIntent::UserInput { .. } => {
            // Not a job command — let the caller handle it
            None
        }
    }
}

/// Process user input through the Session/Thread/Turn + Memory pipeline.
pub async fn handle_user_input(
    session: &mut Session,
    llm: &dyn LlmProvider,
    registry: &ToolRegistry,
    memory_store: &Arc<RwLock<MemoryStore>>,
    system_prompt: &ChatMessage,
    loop_config: &AgenticLoopConfig,
    input: &str,
) {
    info!(input = %input, "🤖 Processing user input through Session + Memory pipeline");
    println!("\n🤖 Agent thinking...");
    match process_user_input(session, llm, registry, memory_store, system_prompt, loop_config, input).await {
        Ok(text) => {
            info!(response_len = text.len(), "✅ Agent responded ({} chars)", text.len());
            println!("\n🤖 Agent: {}", text);
        }
        Err(e) => {
            warn!(error = %e, "❌ Agent error: {}", e);
            println!("\n❌ Error: {}", e);
        }
    }
}

// ============================================================================
// Display Helpers
// ============================================================================

/// Print turn history for the active thread.
fn print_history(session: &Session) {
    if let Some(thread) = session.active_thread() {
        if thread.turns.is_empty() {
            println!("\n📜 No turns yet in this thread.");
        } else {
            println!(
                "\n📜 Turn history for thread {} ({} turns):",
                &thread.id.to_string()[..8],
                thread.turns.len()
            );
            for turn in &thread.turns {
                let state_icon = match turn.state {
                    TurnState::Completed => "✅",
                    TurnState::Processing => "⏳",
                    TurnState::Failed => "❌",
                };
                println!(
                    "\n   Turn #{} {} [{}]",
                    turn.turn_number + 1,
                    state_icon,
                    turn.started_at.format("%H:%M:%S")
                );
                println!("     🧑 \"{}\"", truncate_str(&turn.user_input, 60));

                for tc in &turn.tool_calls {
                    let result_preview = if let Some(ref err) = tc.error {
                        format!("❌ {}", err)
                    } else if let Some(ref res) = tc.result {
                        let s = match res {
                            serde_json::Value::String(s) => s.clone(),
                            other => other.to_string(),
                        };
                        truncate_str(&s, 60)
                    } else {
                        "⏳ pending".to_string()
                    };
                    println!(
                        "     🔧 {}({}) → {}",
                        tc.name, tc.parameters, result_preview
                    );
                }

                if let Some(ref response) = turn.response {
                    println!("     🤖 \"{}\"", truncate_str(response, 80));
                }
                if let Some(ref error) = turn.error {
                    println!("     ❌ Error: {}", error);
                }
            }
        }
    } else {
        println!("⚠️  No active thread.");
    }
}

/// Print session info.
fn print_session_info(session: &Session, current_model: &str) {
    println!("\n📊 Session info:");
    println!("   ID: {}", session.id);
    println!("   User: {}", session.user_id);
    println!(
        "   Created: {}",
        session.created_at.format("%Y-%m-%d %H:%M:%S")
    );
    println!("   Threads: {}", session.threads.len());
    println!(
        "   Total turns: {}",
        session
            .threads
            .values()
            .map(|t| t.turns.len())
            .sum::<usize>()
    );
    if let Some(thread) = session.active_thread() {
        println!(
            "   Active thread: {} ({} turns, {:?})",
            &thread.id.to_string()[..8],
            thread.turns.len(),
            thread.state
        );
    }
    println!("   LLM: {}", current_model);
}
