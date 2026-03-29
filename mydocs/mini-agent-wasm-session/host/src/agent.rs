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
async fn run_agentic_loop(
    llm: &dyn LlmProvider,
    registry: &ToolRegistry,
    messages: &mut Vec<ChatMessage>,
    config: &AgenticLoopConfig,
) -> Result<LoopOutcome, String> {
    let tool_defs = registry.definitions();
    let mut recorded_tool_calls: Vec<RecordedToolCall> = Vec::new();

    for iteration in 1..=config.max_iterations {
        println!(
            "\n--- Iteration {}/{} ---",
            iteration, config.max_iterations
        );

        let response = llm.chat(messages, &tool_defs).await?;

        match response.result {
            LlmOutput::Text(text) => {
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
                println!("  🔧 LLM wants to call {} tool(s)", tool_calls.len());

                if response.finish_reason == FinishReason::Length {
                    println!("  ⚠️  Response was truncated, discarding tool calls");
                    if let Some(text) = content {
                        messages.push(ChatMessage::assistant(text));
                    }
                    messages.push(ChatMessage::user(
                        "Your previous response was truncated. Please try a simpler approach.",
                    ));
                    continue;
                }

                messages.push(ChatMessage::assistant_with_tool_calls(
                    content,
                    tool_calls.clone(),
                ));

                for tc in &tool_calls {
                    let result =
                        execute_tool_with_safety(registry, &tc.name, tc.arguments.clone()).await;

                    let result_msg = process_tool_result(&tc.name, &tc.id, &result);

                    match &result {
                        Ok(output) => println!(
                            "  ✅ Tool '{}' succeeded: {}",
                            tc.name,
                            truncate_str(output, 80)
                        ),
                        Err(e) => println!("  ❌ Tool '{}' failed: {}", tc.name, e),
                    }

                    // Record for Turn
                    recorded_tool_calls.push(RecordedToolCall {
                        name: tc.name.clone(),
                        params: tc.arguments.clone(),
                        result,
                    });

                    messages.push(result_msg);
                }
            }
        }
    }

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

    // 2. Start a new Turn
    thread.start_turn(user_input);
    println!(
        "  📝 Turn #{} started in thread {}",
        thread.turns.len(),
        &thread_id.to_string()[..8]
    );

    // 3. Rebuild messages from Turn history
    //    This is THE KEY: instead of starting fresh, we get the full
    //    conversation history from all previous turns.
    let mut messages = vec![system_prompt.clone()];
    messages.extend(thread.messages());

    println!(
        "  📨 Context: {} messages (from {} turns)",
        messages.len(),
        thread.turns.len()
    );

    // 4. Run the agentic loop
    let outcome = run_agentic_loop(llm, registry, &mut messages, loop_config).await;

    // 5. Record results into the Turn
    let thread = session.thread_mut(thread_id).unwrap();

    match outcome {
        Ok(LoopOutcome::Response { text, tool_calls }) => {
            if let Some(turn) = thread.last_turn_mut() {
                record_tool_calls_to_turn(turn, &tool_calls);
            }
            thread.complete_turn(&text);
            Ok(text)
        }
        Ok(LoopOutcome::MaxIterations { tool_calls }) => {
            if let Some(turn) = thread.last_turn_mut() {
                record_tool_calls_to_turn(turn, &tool_calls);
            }
            thread.fail_turn("Reached maximum iterations");
            Err("Reached maximum iterations without a final response.".to_string())
        }
        Err(e) => {
            thread.fail_turn(&e);
            Err(e)
        }
    }
}
