//! # Agentic Loop — The Core Engine (with Turn integration)
//!
//! Corresponds to: `src/agent/agentic_loop.rs` in IronClaw.
//!
//! This is the HEART of the system. The loop:
//!   1. Calls LLM with conversation context + available tools
//!   2. If LLM returns text → done (or continue if delegate says so)
//!   3. If LLM returns tool calls → execute tools → add results to context → goto 1
//!   4. Repeat until text response or max iterations
//!
//! Key difference from mini-agent-wasm: we track tool calls made during
//! the loop so they can be recorded into the Turn afterwards.

use crate::llm::{ChatMessage, FinishReason, LlmOutput, LlmProvider};
use crate::session::{Session, Turn};
use crate::tools::{execute_tool_with_safety, process_tool_result, ToolRegistry};
use crate::utils::truncate_str;
use tracing::{debug, info, trace, warn, error, instrument};

/// Configuration for the agentic loop.
/// Maps to: `src/agent/agentic_loop.rs` → `struct AgenticLoopConfig`
pub struct AgenticLoopConfig {
    pub max_iterations: usize,
}

impl Default for AgenticLoopConfig {
    fn default() -> Self {
        Self { max_iterations: 10 }
    }
}

/// A tool call recorded during the agentic loop, for later storage into a Turn.
///
/// Replaces the raw `(String, serde_json::Value, Result<String, String>)` tuple
/// for better readability and self-documentation.
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
///
/// Extracted to eliminate duplication between the Response and MaxIterations
/// branches of process_user_input().
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
///
/// This is the single most important function in the entire system.
/// Key difference from mini-agent-wasm: we track tool calls made during
/// the loop so they can be recorded into the Turn afterwards.
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
// Process User Input (Session-aware)
// ============================================================================
// This is the orchestration layer that ties Session + Agentic Loop together.
//
// Maps to: `src/agent/thread_ops.rs` → `process_user_input()`
//
// Flow:
//   1. Get or create the active thread
//   2. Start a new Turn in the thread
//   3. Rebuild messages from Turn history (Thread::messages())
//   4. Prepend system prompt
//   5. Run the agentic loop
//   6. Record tool calls and response into the Turn
//   7. Complete or fail the Turn
// ============================================================================

#[instrument(skip(session, llm, registry, system_prompt, loop_config), fields(user_input = %user_input))]
pub async fn process_user_input(
    session: &mut Session,
    llm: &dyn LlmProvider,
    registry: &ToolRegistry,
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

    // 3. Rebuild messages from Turn history
    let mut messages = vec![system_prompt.clone()];
    let history_messages = thread.messages();
    let history_count = history_messages.len();
    messages.extend(history_messages);

    info!(
        total_messages = messages.len(),
        history_messages = history_count,
        turn_count = thread.turns.len(),
        "📨 Context rebuilt: {} messages (1 system + {} from {} turns)",
        messages.len(), history_count, thread.turns.len()
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
        "  📨 Context: {} messages (from {} turns)",
        messages.len(),
        thread.turns.len()
    );

    // 4. Run the agentic loop
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

    // 5. Record results into the Turn
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
