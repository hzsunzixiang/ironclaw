//! # Agentic Loop — The Core Engine
//!
//! Corresponds to: `src/agent/agentic_loop.rs` in IronClaw.
//!
//! This is the HEART of the system. The loop:
//!   1. Calls LLM with conversation context + available tools
//!   2. If LLM returns text → done (or continue if delegate says so)
//!   3. If LLM returns tool calls → execute tools → add results to context → goto 1
//!   4. Repeat until text response or max iterations
//!
//! In IronClaw, the loop is generic via LoopDelegate trait, serving:
//!   - ChatDelegate (interactive chat)
//!   - JobDelegate (background jobs)
//!   - ContainerDelegate (Docker containers)
//!
//! Here we inline a simplified version without the delegate pattern.

use crate::llm::{ChatMessage, FinishReason, LlmOutput, LlmProvider};
use crate::tools::{execute_tool_with_safety, process_tool_result, ToolRegistry};

/// Configuration for the agentic loop.
/// Maps to: `src/agent/agentic_loop.rs` → `struct AgenticLoopConfig`
pub struct AgenticLoopConfig {
    pub max_iterations: usize,
}

impl Default for AgenticLoopConfig {
    fn default() -> Self {
        Self { max_iterations: 10 } // IronClaw default is 50
    }
}

/// Outcome of the agentic loop.
/// Maps to: `src/agent/agentic_loop.rs` → `enum LoopOutcome`
///
/// In IronClaw, this also has:
///   - Stopped (external signal)
///   - NeedApproval (tool requires user confirmation)
pub enum LoopOutcome {
    Response(String),
    MaxIterations,
}

/// Run the agentic loop.
/// Maps to: `src/agent/agentic_loop.rs` → `run_agentic_loop()`
///
/// This is the single most important function in the entire system.
///
/// In IronClaw, this function:
///   1. check_signals() — check for cancellation/stop/inject
///   2. before_llm_call() — refresh tools, cost guard, inject context
///   3. call_llm() — call the LLM provider
///   4. Handle text → delegate.handle_text_response()
///   5. Handle tool calls → delegate.execute_tool_calls()
///   6. Tool intent nudge — if LLM says "let me search" without calling a tool
///   7. Truncation handling — if response was cut off (finish_reason=Length)
///   8. after_iteration() — post-iteration hook
///
/// We keep: call LLM → handle text/tools → loop
pub async fn run_agentic_loop(
    llm: &dyn LlmProvider,
    registry: &ToolRegistry,
    messages: &mut Vec<ChatMessage>,
    config: &AgenticLoopConfig,
) -> Result<LoopOutcome, String> {
    let tool_defs = registry.definitions();

    for iteration in 1..=config.max_iterations {
        println!(
            "\n--- Iteration {}/{} ---",
            iteration, config.max_iterations
        );

        // ── Step 1: Call LLM ──
        let response = llm.chat(messages, &tool_defs).await?;

        match response.result {
            // ── Step 2a: Text Response ──
            LlmOutput::Text(text) => {
                println!("  💬 LLM returned text: {}", truncate_str(&text, 100));
                return Ok(LoopOutcome::Response(text));
            }

            // ── Step 2b: Tool Calls ──
            LlmOutput::ToolCalls {
                tool_calls,
                content,
            } => {
                println!("  🔧 LLM wants to call {} tool(s)", tool_calls.len());

                // Handle truncated responses
                if response.finish_reason == FinishReason::Length {
                    println!("  ⚠️  Response was truncated, discarding tool calls");
                    if let Some(text) = content {
                        messages.push(ChatMessage::assistant(&text));
                    }
                    messages.push(ChatMessage::user(
                        "Your previous response was truncated. Please try a simpler approach.",
                    ));
                    continue;
                }

                // Add assistant message with tool calls to context
                // (OpenAI protocol requires this before tool results)
                messages.push(ChatMessage::assistant_with_tool_calls(
                    content,
                    tool_calls.clone(),
                ));

                // Execute each tool and add results to context
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

                    messages.push(result_msg);
                }

                // Loop continues — LLM will see tool results in next iteration
            }
        }
    }

    Ok(LoopOutcome::MaxIterations)
}

/// Safely truncate a string to at most `max_bytes` bytes without splitting
/// a multi-byte UTF-8 character. Returns the truncated slice with "[...]" appended
/// if truncation occurred.
fn truncate_str(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    // Walk backwards from max_bytes to find a valid char boundary
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}[...]", &s[..end])
}
