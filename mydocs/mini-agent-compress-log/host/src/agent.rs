
//! # Agentic Loop — The Core Engine (with Turn + Memory + Compaction)
//!
//! Corresponds to: `src/agent/agentic_loop.rs` in IronClaw.
//!
//! This is the HEART of the system. The loop:
//!   1. Calls LLM with conversation context + available tools
//!   2. If LLM returns text → done (or continue if delegate says so)
//!   3. If LLM returns tool calls → execute tools → add results to context → goto 1
//!   4. Repeat until text response or max iterations
//!
//! ## What's new compared to mini-agent-memory:
//!
//! | mini-agent-memory              | mini-agent-compress                      |
//! |-------------------------------|------------------------------------------|
//! | No memory deduplication        | Similarity-based dedup before append     |
//! | Questions stored as memories   | Question filtering in extraction         |
//! | Unbounded MEMORY.md injection  | Truncated to max_memory_words            |
//! | No context compaction          | ContextMonitor + truncation compaction   |
//! | No prompt token tracking       | Token estimation and usage reporting     |
//!
//! Key additions:
//!   - Memory deduplication (similarity scoring before append)
//!   - Improved extraction quality (question filtering, response analysis)
//!   - MEMORY.md truncation for prompt injection (max word limit)
//!   - Context monitoring with automatic compaction (truncation strategy)
//!   - Maps to IronClaw's `context_monitor.rs` + `compaction.rs`

use crate::llm::{ChatMessage, FinishReason, LlmOutput, LlmProvider};
use crate::memory::{self, MemoryStore};
use crate::session::{Session, Turn};
use crate::tools::{execute_tool_with_safety, process_tool_result, ToolRegistry};
use crate::utils::truncate_str;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, trace, warn, error, instrument};

/// Configuration for the agentic loop.
/// Maps to: `src/agent/agentic_loop.rs` → `struct AgenticLoopConfig`
pub struct AgenticLoopConfig {
    pub max_iterations: usize,
    /// Maximum words from MEMORY.md to inject into system prompt.
    /// Prevents prompt token bloat from unbounded memory growth.
    /// Maps to: IronClaw's context window management.
    pub max_memory_words: usize,
    /// Maximum estimated tokens before triggering context compaction.
    /// Maps to: `src/agent/context_monitor.rs` → `DEFAULT_CONTEXT_LIMIT`
    pub context_token_limit: usize,
    /// Compaction threshold as a fraction of context_token_limit.
    /// Maps to: `src/agent/context_monitor.rs` → `COMPACTION_THRESHOLD`
    pub compaction_threshold: f64,
    /// Number of recent turns to keep after compaction.
    /// Maps to: `CompactionStrategy::Truncate { keep_recent }`
    pub compaction_keep_recent: usize,
}

impl Default for AgenticLoopConfig {
    fn default() -> Self {
        Self {
            max_iterations: 10,
            max_memory_words: 500,
            context_token_limit: 8_000,  // Conservative for mini-agent demos
            compaction_threshold: 0.8,
            compaction_keep_recent: 5,
        }
    }
}

// ============================================================================
// Context Monitor — Token estimation and compaction triggers
// ============================================================================
// Maps to: `src/agent/context_monitor.rs` in IronClaw.
//
// Monitors the size of the conversation context and triggers compaction
// (turn truncation) when approaching the configured limit.
//
// In IronClaw, this supports three strategies:
//   - Summarize (LLM generates summary of old turns)
//   - Truncate (simple removal of old turns)
//   - MoveToWorkspace (archive to daily log)
//
// Here we implement the Truncate strategy as a lightweight approximation.
// ============================================================================

/// Approximate tokens per word (rough estimate for English).
/// Maps to: `src/agent/context_monitor.rs` → `TOKENS_PER_WORD`
const TOKENS_PER_WORD: f64 = 1.3;

/// Estimate token count for a ChatMessage.
fn estimate_message_tokens(message: &ChatMessage) -> usize {
    let word_count = message.content.split_whitespace().count();
    let overhead = 4; // ~4 tokens for role and message structure
    (word_count as f64 * TOKENS_PER_WORD) as usize + overhead
}

/// Estimate total tokens for a list of messages.
pub fn estimate_tokens(messages: &[ChatMessage]) -> usize {
    messages.iter().map(estimate_message_tokens).sum()
}

/// Check if context compaction is needed.
///
/// Maps to: `ContextMonitor::needs_compaction()`
pub fn needs_compaction(messages: &[ChatMessage], config: &AgenticLoopConfig) -> bool {
    let tokens = estimate_tokens(messages);
    let threshold = (config.context_token_limit as f64 * config.compaction_threshold) as usize;
    tokens >= threshold
}

/// Get context usage as a percentage.
#[allow(dead_code)]
pub fn context_usage_percent(messages: &[ChatMessage], config: &AgenticLoopConfig) -> f64 {
    let tokens = estimate_tokens(messages);
    (tokens as f64 / config.context_token_limit as f64) * 100.0
}

/// A tool call recorded during the agentic loop, for later storage into a Turn.
pub struct RecordedToolCall {
    /// Tool name.
    pub name: String,
    /// Parameters passed to the tool (JSON).
    pub params: serde_json::Value,
    /// Result from the tool execution (Ok = output string, Err = error string).
    pub result: Result<String, String>,
}

/// Outcome of the agentic loop — includes tool call info for Turn recording.
pub enum LoopOutcome {
    /// LLM returned a final text response.
    Response {
        text: String,
        /// Tool calls made during this turn.
        tool_calls: Vec<RecordedToolCall>,
    },
    /// Reached max iterations without a final response.
    MaxIterations {
        tool_calls: Vec<RecordedToolCall>,
    },
}

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

/// Run the agentic loop.
#[instrument(skip(llm, registry, messages, config), fields(max_iter = config.max_iterations))]
async fn run_agentic_loop(
    llm: &dyn LlmProvider,
    registry: &ToolRegistry,
    messages: &mut Vec<ChatMessage>,
    config: &AgenticLoopConfig,
) -> Result<LoopOutcome, String> {
    let tool_defs = registry.definitions();
    let mut recorded_tool_calls: Vec<RecordedToolCall> = Vec::new();

    info!(
        tool_count = tool_defs.len(),
        tools = ?tool_defs.iter().map(|t| &t.name).collect::<Vec<_>>(),
        initial_message_count = messages.len(),
        "🔄 Entering agentic loop"
    );

    for iteration in 1..=config.max_iterations {
        info!(
            iteration = iteration,
            max = config.max_iterations,
            message_count = messages.len(),
            recorded_tool_calls = recorded_tool_calls.len(),
            "━━━ Iteration {}/{} ━━━", iteration, config.max_iterations
        );
        println!(
            "\n--- Iteration {}/{} ---",
            iteration, config.max_iterations
        );

        // ── Step 1: Call LLM ──
        info!("📤 Step 1: Calling LLM...");
        debug!(
            message_count = messages.len(),
            tool_count = tool_defs.len(),
            "Sending {} messages and {} tool definitions to LLM",
            messages.len(),
            tool_defs.len()
        );
        trace!(
            messages_summary = %messages.iter().enumerate().map(|(i, m)| {
                let role = match m.role {
                    crate::llm::Role::System => "SYS",
                    crate::llm::Role::User => "USR",
                    crate::llm::Role::Assistant => "AST",
                    crate::llm::Role::Tool => "TOL",
                };
                format!("[{}]{}", i, role)
            }).collect::<Vec<_>>().join(" → "),
            "Message flow before LLM call"
        );

        let llm_start = std::time::Instant::now();
        let response = llm.chat(messages, &tool_defs).await?;
        let llm_elapsed = llm_start.elapsed();

        info!(
            elapsed_ms = llm_elapsed.as_millis(),
            finish_reason = ?response.finish_reason,
            "📥 LLM responded in {}ms (finish_reason={:?})",
            llm_elapsed.as_millis(),
            response.finish_reason
        );

        match response.result {
            LlmOutput::Text(text) => {
                info!(
                    text_len = text.len(),
                    total_tool_calls = recorded_tool_calls.len(),
                    "💬 LLM returned TEXT response ({} chars, {} tool calls recorded)",
                    text.len(), recorded_tool_calls.len()
                );
                debug!(text = %text, "Full text response from LLM");
                println!("  💬 LLM returned text: {}", truncate_str(&text, 100));
                return Ok(LoopOutcome::Response {
                    text,
                    tool_calls: recorded_tool_calls,
                });
            }

            LlmOutput::ToolCalls {
                tool_calls,
                content,
            } => {
                info!(
                    tool_call_count = tool_calls.len(),
                    has_content = content.is_some(),
                    "🔧 LLM returned TOOL CALLS ({} tool(s))", tool_calls.len()
                );
                for (i, tc) in tool_calls.iter().enumerate() {
                    debug!(
                        index = i,
                        tool_name = %tc.name,
                        tool_call_id = %tc.id,
                        arguments = %serde_json::to_string_pretty(&tc.arguments).unwrap_or_default(),
                        "Tool call [{}]: {} (id={})", i, tc.name, tc.id
                    );
                }
                if let Some(ref text) = content {
                    debug!(content = %text, "Assistant content alongside tool calls");
                }
                println!("  🔧 LLM wants to call {} tool(s)", tool_calls.len());

                if response.finish_reason == FinishReason::Length {
                    warn!(
                        "⚠️  Response was truncated (finish_reason=Length), discarding tool calls"
                    );
                    println!("  ⚠️  Response was truncated, discarding tool calls");
                    if let Some(text) = content {
                        debug!(text = %text, "Adding truncated assistant text to context");
                        messages.push(ChatMessage::assistant(text));
                    }
                    messages.push(ChatMessage::user(
                        "Your previous response was truncated. Please try a simpler approach.",
                    ));
                    info!("Injected truncation recovery message, continuing to next iteration");
                    continue;
                }

                info!("📝 Adding assistant message with tool calls to conversation context");
                messages.push(ChatMessage::assistant_with_tool_calls(
                    content,
                    tool_calls.clone(),
                ));
                debug!(
                    message_count = messages.len(),
                    "Context now has {} messages (after adding assistant tool call message)",
                    messages.len()
                );

                for (i, tc) in tool_calls.iter().enumerate() {
                    info!(
                        index = i,
                        tool = %tc.name,
                        "⚙️  Executing tool [{}/{}]: {}",
                        i + 1, tool_calls.len(), tc.name
                    );
                    debug!(
                        tool_name = %tc.name,
                        tool_call_id = %tc.id,
                        arguments = %serde_json::to_string_pretty(&tc.arguments).unwrap_or_default(),
                        "Tool execution input"
                    );

                    let tool_start = std::time::Instant::now();
                    let result =
                        execute_tool_with_safety(registry, &tc.name, tc.arguments.clone()).await;
                    let tool_elapsed = tool_start.elapsed();

                    let result_msg = process_tool_result(&tc.name, &tc.id, &result);

                    match &result {
                        Ok(output) => {
                            info!(
                                tool = %tc.name,
                                elapsed_ms = tool_elapsed.as_millis(),
                                output_len = output.len(),
                                "✅ Tool '{}' succeeded in {}ms ({} chars output)",
                                tc.name, tool_elapsed.as_millis(), output.len()
                            );
                            debug!(
                                tool = %tc.name,
                                output = %output,
                                "Full tool output"
                            );
                            println!(
                                "  ✅ Tool '{}' succeeded: {}",
                                tc.name,
                                truncate_str(output, 80)
                            );
                        }
                        Err(e) => {
                            error!(
                                tool = %tc.name,
                                elapsed_ms = tool_elapsed.as_millis(),
                                error = %e,
                                "❌ Tool '{}' failed in {}ms: {}",
                                tc.name, tool_elapsed.as_millis(), e
                            );
                            println!("  ❌ Tool '{}' failed: {}", tc.name, e);
                        }
                    }

                    // Record for Turn
                    debug!(
                        tool = %tc.name,
                        is_success = result.is_ok(),
                        "Recording tool call for Turn history"
                    );
                    recorded_tool_calls.push(RecordedToolCall {
                        name: tc.name.clone(),
                        params: tc.arguments.clone(),
                        result,
                    });

                    info!(
                        "📝 Adding tool result for '{}' (call_id={}) to conversation context",
                        tc.name, tc.id
                    );
                    messages.push(result_msg);
                    debug!(
                        message_count = messages.len(),
                        "Context now has {} messages (after adding tool result)",
                        messages.len()
                    );
                }

                info!(
                    message_count = messages.len(),
                    total_recorded = recorded_tool_calls.len(),
                    "🔄 All tools executed. Looping back to LLM with updated context ({} messages, {} recorded tool calls)",
                    messages.len(), recorded_tool_calls.len()
                );
            }
        }
    }

    warn!(
        max_iterations = config.max_iterations,
        total_recorded = recorded_tool_calls.len(),
        "⚠️  Reached max iterations ({}) without final text response",
        config.max_iterations
    );
    Ok(LoopOutcome::MaxIterations {
        tool_calls: recorded_tool_calls,
    })
}

// ============================================================================
// Process User Input (Session + Memory aware)
// ============================================================================
// This is the orchestration layer that ties Session + Memory + Agentic Loop.
//
// Maps to: `src/agent/thread_ops.rs` → `process_user_input()`
//
// Flow:
//   1. Get or create the active thread
//   2. Start a new Turn in the thread
//   3. Rebuild messages from Turn history (Thread::messages())
//   4. Prepend system prompt (with MEMORY.md content injected!)
//   5. Run the agentic loop
//   6. Record tool calls and response into the Turn
//   7. Auto-extract memories from the completed turn
//   8. Complete or fail the Turn
// ============================================================================

/// Build the system prompt with long-term memory injected.
///
/// Maps to: `Workspace::system_prompt()` in IronClaw.
///
/// This is THE KEY INTEGRATION POINT between short-term (Session) and
/// long-term (Workspace) memory. The MEMORY.md content is injected into
/// the system prompt so the LLM has access to persistent facts.
///
/// ## What's new (compared to mini-agent-memory):
///
/// The memory content is now pre-truncated by the caller (via
/// `MemoryStore::memory_content_truncated()`), preventing unbounded
/// prompt token growth.
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
    info!(
        turn_number = turn_number,
        thread_id = %&thread_id.to_string()[..8],
        "📝 Turn #{} started", turn_number
    );
    println!(
        "  📝 Turn #{} started in thread {}",
        turn_number,
        &thread_id.to_string()[..8]
    );

    // 3. Check if context compaction is needed BEFORE rebuilding messages.
    //    Maps to: IronClaw's ContextMonitor + ContextCompactor
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
                turn_count = before_turns,
                keep_recent = loop_config.compaction_keep_recent,
                "✂️  Context approaching limit — compacting (Truncate strategy)"
            );
            println!(
                "  ✂️  Context compaction: {} turns → keeping {} most recent (est. {} tokens, {:.0}% of limit)",
                before_turns, loop_config.compaction_keep_recent, est_tokens, usage_pct
            );

            // Write removed turns to daily log before truncating
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
            info!(
                turns_after = thread.turns.len(),
                "✂️  Compaction complete: {} turns remaining",
                thread.turns.len()
            );
        } else {
            debug!(
                est_tokens = est_tokens,
                usage_pct = format!("{:.1}%", usage_pct),
                "Context within limits ({} tokens, {:.0}% of {})",
                est_tokens, usage_pct, loop_config.context_token_limit
            );
        }
    }

    // 4. Rebuild messages from Turn history (after potential compaction)
    let mut messages = vec![system_prompt.clone()];
    let history_messages = thread.messages();
    let history_count = history_messages.len();
    messages.extend(history_messages);

    let est_total_tokens = estimate_tokens(&messages);
    info!(
        total_messages = messages.len(),
        history_messages = history_count,
        turn_count = thread.turns.len(),
        est_tokens = est_total_tokens,
        "📨 Context rebuilt: {} messages (1 system + {} from {} turns, ~{} tokens)",
        messages.len(), history_count, thread.turns.len(), est_total_tokens
    );
    debug!(
        messages_roles = %messages.iter().map(|m| {
            match m.role {
                crate::llm::Role::System => "SYS",
                crate::llm::Role::User => "USR",
                crate::llm::Role::Assistant => "AST",
                crate::llm::Role::Tool => "TOL",
            }
        }).collect::<Vec<_>>().join(" → "),
        "Message role sequence"
    );
    println!(
        "  📨 Context: {} messages (from {} turns, ~{} tokens)",
        messages.len(),
        thread.turns.len(),
        est_total_tokens
    );

    // 5. Run the agentic loop
    info!("🔄 Launching agentic loop...");
    let loop_start = std::time::Instant::now();
    let outcome = run_agentic_loop(llm, registry, &mut messages, loop_config).await;
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
                "✅ Turn completed with response ({} chars, {} tool calls)",
                text.len(), tool_calls.len()
            );
            if let Some(turn) = thread.last_turn_mut() {
                record_tool_calls_to_turn(turn, &tool_calls);
            }
            thread.complete_turn(&text);

            // 7. Auto-extract memories from the completed turn
            //    This is a complement to the LLM's own memory_write calls.
            //    Maps to: IronClaw's passive profile building (AGENTS.md guidelines)
            //
            //    IMPROVED: Now skips extraction if LLM already called memory_write
            //    during this turn (avoids double-writing the same fact).
            let llm_already_wrote_memory = tool_calls.iter().any(|tc| tc.name == "memory_write" && tc.result.is_ok());
            let memories = if llm_already_wrote_memory {
                info!("🧠 Skipping auto-extraction: LLM already called memory_write this turn");
                Vec::new()
            } else {
                memory::extract_memories_from_turn(user_input, &text)
            };
            if !memories.is_empty() {
                info!(
                    memory_count = memories.len(),
                    "🧠 Auto-extracted {} memory/memories from turn",
                    memories.len()
                );
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
                // Also log to daily log
                let summary = format!(
                    "Turn #{}: User asked '{}' → {} tool call(s), {} auto-memory(s)",
                    turn_number,
                    truncate_str(user_input, 40),
                    tool_calls.len(),
                    memories.len()
                );
                let _ = store.append_daily_log(&summary);
            }

            debug!(
                turn_count = thread.turns.len(),
                thread_id = %&thread_id.to_string()[..8],
                "Turn recorded and completed in thread"
            );
            Ok(text)
        }
        Ok(LoopOutcome::MaxIterations { tool_calls }) => {
            warn!(
                tool_call_count = tool_calls.len(),
                "⚠️  Turn failed: reached max iterations ({} tool calls made)",
                tool_calls.len()
            );
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
