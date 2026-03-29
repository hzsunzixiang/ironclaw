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
//! Same as mini-agent-loop — the loop doesn't change.
//! It calls tools through the Tool trait, unaware of WASM underneath.

use crate::llm::{ChatMessage, FinishReason, LlmOutput, LlmProvider, Role};
use crate::tools::{execute_tool_with_safety, process_tool_result, ToolRegistry};

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

/// Outcome of the agentic loop.
/// Maps to: `src/agent/agentic_loop.rs` → `enum LoopOutcome`
pub enum LoopOutcome {
    Response(String),
    MaxIterations,
}

/// Run the agentic loop.
/// Maps to: `src/agent/agentic_loop.rs` → `run_agentic_loop()`
///
/// This is the single most important function in the entire system.
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

        let response = llm.chat(messages, &tool_defs).await?;

        match response.result {
            LlmOutput::Text(text) => {
                println!(
                    "  💬 LLM returned text: {}",
                    &text[..text.len().min(100)]
                );
                return Ok(LoopOutcome::Response(text));
            }

            LlmOutput::ToolCalls {
                tool_calls,
                content,
            } => {
                println!("  🔧 LLM wants to call {} tool(s)", tool_calls.len());

                if response.finish_reason == FinishReason::Length {
                    println!("  ⚠️  Response was truncated, discarding tool calls");
                    if let Some(text) = content {
                        messages.push(ChatMessage {
                            role: Role::Assistant,
                            content: text,
                            tool_call_id: None,
                            name: None,
                            tool_calls: None,
                        });
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
                            &output[..output.len().min(80)]
                        ),
                        Err(e) => println!("  ❌ Tool '{}' failed: {}", tc.name, e),
                    }

                    messages.push(result_msg);
                }
            }
        }
    }

    Ok(LoopOutcome::MaxIterations)
}
