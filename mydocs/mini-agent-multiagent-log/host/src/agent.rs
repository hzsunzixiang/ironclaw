
//! # Chat Agent — ChatDelegate + Session/Memory Orchestration
//!
//! Maps to: `src/agent/dispatcher.rs` + `src/agent/thread_ops.rs` in IronClaw.
//!
//! This module provides:
//! 1. `ChatDelegate` — implements `LoopDelegate` for interactive chat sessions
//! 2. `process_user_input()` — orchestrates Session + Memory + Agentic Loop
//!
//! ## What's new compared to mini-agent-compress:
//!
//! | mini-agent-compress            | mini-agent-multiagent                    |
//! |-------------------------------|------------------------------------------|
//! | Hardcoded agentic loop         | ChatDelegate implements LoopDelegate     |
//! | No background jobs             | Scheduler + JobDelegate for /job commands |
//! | No message routing             | Router dispatches to Chat vs Job          |
//! | Single execution path          | Two paths: ChatDelegate + JobDelegate     |
//!
//! The agentic loop engine is now shared between ChatDelegate (interactive)
//! and JobDelegate (background), just like IronClaw's three delegates.

use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, trace, warn, error, instrument};

use crate::agentic_loop::{
    run_agentic_loop, AgenticLoopConfig, LoopDelegate, LoopOutcome, LoopSignal,
    RecordedToolCall, TextAction,
};
use crate::llm::{ChatMessage, LlmProvider, ToolDefinition};
use crate::memory::{self, MemoryStore};
use crate::session::{Session, Turn};
use crate::tools::{execute_tool_with_safety, ToolRegistry};
use crate::utils::truncate_str;

// ============================================================================
// Context Monitor — Token estimation and compaction triggers
// ============================================================================
// Maps to: `src/agent/context_monitor.rs` in IronClaw.

/// Approximate tokens per word (rough estimate for English).
const TOKENS_PER_WORD: f64 = 1.3;

/// Estimate token count for a ChatMessage.
fn estimate_message_tokens(message: &ChatMessage) -> usize {
    let word_count = message.content.split_whitespace().count();
    let overhead = 4;
    let tokens = (word_count as f64 * TOKENS_PER_WORD) as usize + overhead;
    trace!(
        role = ?message.role,
        word_count = word_count,
        est_tokens = tokens,
        "Token estimate for {:?} message: {} words → ~{} tokens",
        message.role, word_count, tokens
    );
    tokens
}

/// Estimate total tokens for a list of messages.
pub fn estimate_tokens(messages: &[ChatMessage]) -> usize {
    let total: usize = messages.iter().map(estimate_message_tokens).sum();
    trace!(
        message_count = messages.len(),
        est_total_tokens = total,
        "Total token estimate: {} messages → ~{} tokens",
        messages.len(), total
    );
    total
}

/// Check if context compaction is needed.
pub fn needs_compaction(messages: &[ChatMessage], config: &AgenticLoopConfig) -> bool {
    let tokens = estimate_tokens(messages);
    let threshold = (config.context_token_limit as f64 * config.compaction_threshold) as usize;
    let needs = tokens >= threshold;
    trace!(
        est_tokens = tokens,
        threshold = threshold,
        limit = config.context_token_limit,
        needs_compaction = needs,
        "Compaction check: {} tokens vs {} threshold → {}",
        tokens, threshold, if needs { "COMPACT" } else { "OK" }
    );
    needs
}

// ============================================================================
// ChatDelegate — LoopDelegate for interactive chat sessions
// ============================================================================

/// Delegate for the chat (dispatcher) context.
///
/// Maps to: `src/agent/dispatcher.rs` → `ChatDelegate`
///
/// Implements `LoopDelegate` to customize the shared agentic loop for
/// interactive chat sessions. Unlike `JobDelegate`, this holds borrowed
/// references and runs synchronously within the user's input processing.
struct ChatDelegate<'a> {
    llm: &'a dyn LlmProvider,
    registry: &'a ToolRegistry,
}

#[async_trait]
impl<'a> LoopDelegate for ChatDelegate<'a> {
    async fn check_signals(&self) -> LoopSignal {
        // In CLI mode, no external signals to check.
        // In IronClaw, this checks for thread interruption.
        LoopSignal::Continue
    }

    async fn before_llm_call(
        &self,
        _messages: &mut Vec<ChatMessage>,
        _iteration: usize,
    ) -> Option<LoopOutcome> {
        // No pre-LLM hooks needed for chat.
        // In IronClaw, this refreshes tool definitions and enforces cost guards.
        None
    }

    async fn call_llm(
        &self,
        messages: &mut Vec<ChatMessage>,
        tool_defs: &[ToolDefinition],
    ) -> Result<crate::llm::LlmResponse, String> {
        self.llm.chat(messages, tool_defs).await
    }

    async fn handle_text_response(
        &self,
        text: &str,
        _recorded_tool_calls: &[RecordedToolCall],
    ) -> TextAction {
        // For chat, text response always means we're done.
        TextAction::Return(LoopOutcome::Response {
            text: text.to_string(),
            tool_calls: Vec::new(), // Already recorded by the loop
        })
    }

    async fn execute_tool(
        &self,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<String, String> {
        execute_tool_with_safety(self.registry, tool_name, arguments).await
    }

    fn tool_definitions(&self) -> Vec<ToolDefinition> {
        self.registry.definitions()
    }

    async fn after_iteration(&self, _iteration: usize) {
        // No post-iteration hooks for chat.
    }
}

// ============================================================================
// Record Tool Calls to Turn
// ============================================================================

/// Record a list of tool calls into a Turn.
fn record_tool_calls_to_turn(turn: &mut Turn, tool_calls: &[RecordedToolCall]) {
    for tc in tool_calls {
        turn.record_tool_call(&tc.name, tc.params.clone());
        match &tc.result {
            Ok(output) => {
                turn.record_tool_result(serde_json::Value::String(output.clone()))
            }
            Err(e) => turn.record_tool_error(e.clone()),
        }
    }
}

// ============================================================================
// System Prompt Builder
// ============================================================================

/// Build the system prompt with long-term memory injected.
///
/// Maps to: `Workspace::system_prompt()` in IronClaw.
pub fn build_system_prompt_with_memory(model: &str, memory_content: &str) -> ChatMessage {
    let memory_section = if memory_content.trim().is_empty()
        || memory_content.trim() == "# Memory"
    {
        String::new()
    } else {
        format!(
            "\n\n## Long-Term Memory\n\
             The following is your persistent memory from previous sessions. \
             Use this information when relevant:\n\n\
             <memory>\n{}\n</memory>",
            memory_content.trim()
        )
    };

    ChatMessage::system(format!(
        "You are a helpful assistant powered by the {} model. \
         You have access to tools: a calculator tool for math, and memory tools \
         (memory_search, memory_write, memory_read) for persistent memory. \
         When the user asks a math question, use the calculator tool. \
         When the user asks you to remember something, use memory_write with target='memory'. \
         When the user asks about something you might have noted before, use memory_search first. \
         You have conversation memory — you can reference previous turns in this thread. \
         When asked about your identity, truthfully state that you are based on {}. \
         IMPORTANT: Do NOT call memory_write if the information is already in your Long-Term Memory above.{}",
        model, model, memory_section
    ))
}

// ============================================================================
// Process User Input (Session + Memory aware)
// ============================================================================

/// Process user input through the Session/Thread/Turn + Memory pipeline.
///
/// Maps to: `src/agent/thread_ops.rs` → `process_user_input()`
///
/// This is the orchestration layer that ties Session + Memory + Agentic Loop.
/// Now uses `ChatDelegate` to implement `LoopDelegate` for the shared engine.
#[instrument(skip(session, llm, registry, memory_store, system_prompt, loop_config), fields(user_input = %user_input))]
pub async fn process_user_input(
    session: &mut Session,
    llm: &dyn LlmProvider,
    registry: &ToolRegistry,
    memory_store: &Arc<RwLock<MemoryStore>>,
    system_prompt: &ChatMessage,
    loop_config: &AgenticLoopConfig,
    user_input: &str,
) -> Result<String, String> {
    // 1. Get or create the active thread
    let thread = session.get_or_create_thread();
    let thread_id = thread.id;

    info!(
        thread_id = %&thread_id.to_string()[..8],
        existing_turns = thread.turns.len(),
        "📋 Processing user input in thread"
    );

    // 2. Start a new Turn
    thread.start_turn(user_input);
    let turn_number = thread.turns.len();
    info!(turn_number = turn_number, "📝 Turn #{} started", turn_number);
    println!(
        "  📝 Turn #{} started in thread {}",
        turn_number,
        &thread_id.to_string()[..8]
    );

    // 3. Check if context compaction is needed
    {
        let test_messages = thread.messages();
        let est_tokens = estimate_tokens(&test_messages);
        let usage_pct = (est_tokens as f64 / loop_config.context_token_limit as f64) * 100.0;

        if needs_compaction(
            &[&[system_prompt.clone()], test_messages.as_slice()].concat(),
            loop_config,
        ) {
            let before_turns = thread.turns.len();
            info!(
                est_tokens = est_tokens,
                usage_pct = format!("{:.1}%", usage_pct),
                "✂️  Context approaching limit — compacting"
            );
            println!(
                "  ✂️  Context compaction: {} turns → keeping {} most recent",
                before_turns, loop_config.compaction_keep_recent
            );

            {
                let store = memory_store.read().await;
                let removed_count = before_turns.saturating_sub(loop_config.compaction_keep_recent);
                if removed_count > 0 {
                    let summary = format!(
                        "Context compaction: removed {} old turns (kept {} recent). Est. tokens: {}",
                        removed_count, loop_config.compaction_keep_recent, est_tokens
                    );
                    let _ = store.append_daily_log(&summary);
                }
            }

            thread.truncate_turns(loop_config.compaction_keep_recent);
        } else {
            debug!(
                est_tokens = est_tokens,
                usage_pct = format!("{:.1}%", usage_pct),
                "Context within limits"
            );
        }
    }

    // 4. Rebuild messages from Turn history
    let mut messages = vec![system_prompt.clone()];
    let history_messages = thread.messages();
    let history_count = history_messages.len();
    messages.extend(history_messages);

    let est_total_tokens = estimate_tokens(&messages);
    info!(
        total_messages = messages.len(),
        history_messages = history_count,
        est_tokens = est_total_tokens,
        "📨 Context rebuilt: {} messages (~{} tokens)",
        messages.len(), est_total_tokens
    );
    println!(
        "  📨 Context: {} messages (from {} turns, ~{} tokens)",
        messages.len(),
        thread.turns.len(),
        est_total_tokens
    );

    // 5. Run the agentic loop via ChatDelegate
    info!("🔄 Launching agentic loop via ChatDelegate...");
    let delegate = ChatDelegate { llm, registry };
    let loop_start = std::time::Instant::now();
    let outcome = run_agentic_loop(&delegate, &mut messages, loop_config).await;
    let loop_elapsed = loop_start.elapsed();

    info!(
        elapsed_ms = loop_elapsed.as_millis(),
        is_ok = outcome.is_ok(),
        "Agentic loop completed in {}ms",
        loop_elapsed.as_millis()
    );

    // 6. Record results into the Turn
    let thread = session.thread_mut(thread_id).unwrap();

    match outcome {
        Ok(LoopOutcome::Response { text, tool_calls }) => {
            info!(
                response_len = text.len(),
                tool_call_count = tool_calls.len(),
                "✅ Turn completed with response"
            );
            if let Some(turn) = thread.last_turn_mut() {
                record_tool_calls_to_turn(turn, &tool_calls);
            }
            thread.complete_turn(&text);

            // 7. Auto-extract memories
            let llm_already_wrote_memory = tool_calls.iter().any(|tc| tc.name == "memory_write" && tc.result.is_ok());
            let memories = if llm_already_wrote_memory {
                info!("🧠 Skipping auto-extraction: LLM already called memory_write this turn");
                Vec::new()
            } else {
                memory::extract_memories_from_turn(user_input, &text)
            };
            if !memories.is_empty() {
                info!(memory_count = memories.len(), "🧠 Auto-extracted {} memory/memories", memories.len());
                let store = memory_store.write().await;
                for mem in &memories {
                    match store.append_memory(mem) {
                        Ok(_) => {
                            info!(memory = %mem, "🧠 Auto-saved memory");
                            println!("  🧠 Auto-saved memory: {}", truncate_str(mem, 60));
                        }
                        Err(e) => {
                            warn!(error = %e, "Failed to auto-save memory");
                        }
                    }
                }
                let summary = format!(
                    "Turn #{}: User asked '{}' → {} tool call(s), {} auto-memory(s)",
                    turn_number, truncate_str(user_input, 40), tool_calls.len(), memories.len()
                );
                let _ = store.append_daily_log(&summary);
            }

            Ok(text)
        }
        Ok(LoopOutcome::Stopped { tool_calls }) => {
            warn!("⚠️  Turn stopped by signal");
            if let Some(turn) = thread.last_turn_mut() {
                record_tool_calls_to_turn(turn, &tool_calls);
            }
            thread.fail_turn("Stopped by signal");
            Err("Turn was stopped.".to_string())
        }
        Ok(LoopOutcome::MaxIterations { tool_calls }) => {
            warn!("⚠️  Turn failed: reached max iterations");
            if let Some(turn) = thread.last_turn_mut() {
                record_tool_calls_to_turn(turn, &tool_calls);
            }
            thread.fail_turn("Reached maximum iterations");
            Err("Reached maximum iterations without a final response.".to_string())
        }
        Err(e) => {
            error!(error = %e, "❌ Turn failed with error");
            thread.fail_turn(&e);
            Err(e)
        }
    }
}
