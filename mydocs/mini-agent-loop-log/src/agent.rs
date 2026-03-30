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
use tracing::{debug, info, trace, warn, error, instrument};

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
#[instrument(skip(llm, registry, messages, config), fields(max_iter = config.max_iterations))]
pub async fn run_agentic_loop(
    llm: &dyn LlmProvider,
    registry: &ToolRegistry,
    messages: &mut Vec<ChatMessage>,
    config: &AgenticLoopConfig,
) -> Result<LoopOutcome, String> {
    let tool_defs = registry.definitions();
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
            // ── Step 2a: Text Response ──
            LlmOutput::Text(text) => {
                info!(
                    text_len = text.len(),
                    "💬 LLM returned TEXT response ({} chars)", text.len()
                );
                debug!(text = %text, "Full text response from LLM");
                println!("  💬 LLM returned text: {}", truncate_str(&text, 100));
                return Ok(LoopOutcome::Response(text));
            }

            // ── Step 2b: Tool Calls ──
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

                // Handle truncated responses
                if response.finish_reason == FinishReason::Length {
                    warn!(
                        "⚠️  Response was truncated (finish_reason=Length), discarding tool calls"
                    );
                    println!("  ⚠️  Response was truncated, discarding tool calls");
                    if let Some(text) = content {
                        debug!(text = %text, "Adding truncated assistant text to context");
                        messages.push(ChatMessage::assistant(&text));
                    }
                    messages.push(ChatMessage::user(
                        "Your previous response was truncated. Please try a simpler approach.",
                    ));
                    info!("Injected truncation recovery message, continuing to next iteration");
                    continue;
                }

                // Add assistant message with tool calls to context
                // (OpenAI protocol requires this before tool results)
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

                // Execute each tool and add results to context
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
                    "🔄 All tools executed. Looping back to LLM with updated context ({} messages)",
                    messages.len()
                );
                // Loop continues — LLM will see tool results in next iteration
            }
        }
    }

    warn!(
        max_iterations = config.max_iterations,
        "⚠️  Reached max iterations ({}) without final text response",
        config.max_iterations
    );
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
