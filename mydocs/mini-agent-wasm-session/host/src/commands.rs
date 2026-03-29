//! # CLI Command Handling
//!
//! Extracted from main.rs to keep the entry point clean.
//! Handles all `/` commands (session management, model switching)
//! and delegates user input to the agentic loop.

use crate::agent::{process_user_input, AgenticLoopConfig};
use crate::llm::{ChatMessage, HaiConfig, LlmProvider};
use crate::session::{Session, TurnState};
use crate::tools::ToolRegistry;
use crate::utils::truncate_str;

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
pub fn handle_session_command(input: &str, session: &mut Session, current_model: &str) -> CommandResult {
    if input == "quit" || input == "exit" {
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
        if let Ok(idx) = idx_str.parse::<usize>() {
            let summaries = session.list_threads();
            if idx >= 1 && idx <= summaries.len() {
                let thread_id = summaries[idx - 1].id;
                if session.switch_thread(thread_id) {
                    let thread = session.active_thread().unwrap();
                    println!(
                        "✅ Switched to thread #{} [{}] ({} turns)",
                        idx,
                        &thread_id.to_string()[..8],
                        thread.turns.len()
                    );
                } else {
                    println!("❌ Failed to switch thread");
                }
            } else {
                println!("❌ Invalid thread number. Use /threads to see available threads.");
            }
        } else {
            println!("❌ Usage: /switch <number>");
        }
        return CommandResult::Continue;
    }

    if input == "/history" {
        print_history(session);
        return CommandResult::Continue;
    }

    if input == "/session" {
        print_session_info(session, current_model);
        return CommandResult::Continue;
    }

    CommandResult::NotACommand
}

/// Handle model management commands (/models, /model <name>).
///
/// Returns `Some(new_model)` if the model was switched, `None` otherwise.
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
        return Some(current_model.to_string()); // Signal: handled, no switch
    }

    if input.starts_with("/model ") {
        let model_name = input.strip_prefix("/model ").unwrap().trim();
        let resolved = hai_config.resolve_model(Some(model_name));
        return Some(resolved);
    }

    None
}

/// Process user input through the Session/Thread/Turn pipeline.
pub async fn handle_user_input(
    session: &mut Session,
    llm: &dyn LlmProvider,
    registry: &ToolRegistry,
    system_prompt: &ChatMessage,
    loop_config: &AgenticLoopConfig,
    input: &str,
) {
    println!("\n🤖 Agent thinking...");
    match process_user_input(session, llm, registry, system_prompt, loop_config, input).await {
        Ok(text) => {
            println!("\n🤖 Agent: {}", text);
        }
        Err(e) => {
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
