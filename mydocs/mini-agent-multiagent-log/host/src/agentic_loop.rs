
//! # Unified Agentic Loop Engine
//!
//! Maps to: `src/agent/agentic_loop.rs` in IronClaw.
//!
//! Provides a single implementation of the core LLM call → tool execution →
//! result processing → context update → repeat cycle. Two consumers
//! (chat delegate, job worker) customize behavior via the `LoopDelegate` trait.
//!
//! ## Design
//!
//! The shared loop calls delegate methods at well-defined points:
//!
//! ```text
//! run_agentic_loop(delegate, messages, config)
//!   1. Check signals (stop/cancel) via delegate.check_signals()
//!   2. Pre-LLM hook via delegate.before_llm_call()
//!   3. LLM call via delegate.call_llm()
//!   4. If text response → delegate.handle_text_response() → Continue or Return
//!   5. If tool calls → delegate.execute_tool_calls() → Continue or Return
//!   6. Post-iteration hook via delegate.after_iteration()
//!   7. Repeat until LoopOutcome returned or max_iterations reached
//! ```

use async_trait::async_trait;
use tracing::{debug, info, trace, warn};

use crate::llm::{ChatMessage, ToolDefinition};

// ============================================================================
// Loop Signals and Outcomes
// ============================================================================

/// Signal from the delegate indicating how the loop should proceed.
///
/// Maps to: `src/agent/agentic_loop.rs` → `LoopSignal`
pub enum LoopSignal {
    /// Continue normally.
    Continue,
    /// Stop the loop gracefully.
    Stop,
    /// Inject a user message into context and continue.
    #[allow(dead_code)]
    InjectMessage(String),
}

/// Outcome of a text response from the LLM.
///
/// Maps to: `src/agent/agentic_loop.rs` → `TextAction`
pub enum TextAction {
    /// Return this as the final loop result.
    Return(LoopOutcome),
    /// Continue the loop (text was handled but loop should proceed).
    #[allow(dead_code)]
    Continue,
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

/// Final outcome of the agentic loop.
///
/// Maps to: `src/agent/agentic_loop.rs` → `LoopOutcome`
pub enum LoopOutcome {
    /// LLM returned a final text response.
    Response {
        text: String,
        /// Tool calls made during this turn.
        tool_calls: Vec<RecordedToolCall>,
    },
    /// Loop was stopped by a signal.
    Stopped {
        tool_calls: Vec<RecordedToolCall>,
    },
    /// Reached max iterations without a final response.
    MaxIterations {
        tool_calls: Vec<RecordedToolCall>,
    },
}

/// Configuration for the agentic loop.
///
/// Maps to: `src/agent/agentic_loop.rs` → `AgenticLoopConfig`
pub struct AgenticLoopConfig {
    pub max_iterations: usize,
    /// Maximum words from MEMORY.md to inject into system prompt.
    pub max_memory_words: usize,
    /// Maximum estimated tokens before triggering context compaction.
    pub context_token_limit: usize,
    /// Compaction threshold as a fraction of context_token_limit.
    pub compaction_threshold: f64,
    /// Number of recent turns to keep after compaction.
    pub compaction_keep_recent: usize,
}

impl Default for AgenticLoopConfig {
    fn default() -> Self {
        Self {
            max_iterations: 10,
            max_memory_words: 500,
            context_token_limit: 8_000,
            compaction_threshold: 0.8,
            compaction_keep_recent: 5,
        }
    }
}

// ============================================================================
// LoopDelegate Trait — The Strategy Pattern
// ============================================================================

/// Strategy trait — each consumer implements this to customize I/O and lifecycle.
///
/// Maps to: `src/agent/agentic_loop.rs` → `trait LoopDelegate`
///
/// The shared loop calls these methods at well-defined points. Consumers
/// implement only the behavior that differs between chat and job contexts.
/// The loop itself handles the common logic: iteration counting, tool
/// definition refresh, and the respond → execute → process cycle.
///
/// # `Send + Sync` requirement
///
/// This trait requires `Send + Sync` because the loop accepts `&dyn LoopDelegate`.
/// Delegates using borrowed references must ensure all borrowed fields are
/// `Send + Sync`. If a delegate needs to be spawned into a detached task,
/// it must use `Arc`-based ownership instead of borrows (as `JobDelegate` does).
#[async_trait]
pub trait LoopDelegate: Send + Sync {
    /// Called at the start of each iteration. Check for external signals
    /// (cancellation, user messages, stop requests).
    async fn check_signals(&self) -> LoopSignal;

    /// Called before the LLM call. Allows the delegate to refresh tool
    /// definitions, enforce cost guards, or inject messages.
    /// Return `Some(outcome)` to break the loop early.
    async fn before_llm_call(
        &self,
        messages: &mut Vec<ChatMessage>,
        iteration: usize,
    ) -> Option<LoopOutcome>;

    /// Call the LLM with the current messages and tool definitions.
    /// Delegates own the LLM call to handle consumer-specific concerns
    /// (rate limiting, auto-compaction, cost tracking).
    async fn call_llm(
        &self,
        messages: &mut Vec<ChatMessage>,
        tool_defs: &[ToolDefinition],
    ) -> Result<crate::llm::LlmResponse, String>;

    /// Handle a text-only response from the LLM.
    /// Return `TextAction::Return` to exit the loop, `TextAction::Continue` to proceed.
    async fn handle_text_response(
        &self,
        text: &str,
        recorded_tool_calls: &[RecordedToolCall],
    ) -> TextAction;

    /// Execute a single tool call and return the result.
    /// The loop handles adding the result to messages and recording it.
    async fn execute_tool(
        &self,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<String, String>;

    /// Get the current tool definitions.
    fn tool_definitions(&self) -> Vec<ToolDefinition>;

    /// Called after each successful iteration (no error, no early return).
    async fn after_iteration(&self, _iteration: usize) {}
}

// ============================================================================
// Shared Agentic Loop Engine
// ============================================================================

/// Run the unified agentic loop.
///
/// This is the single implementation used by both consumers (chat, job).
/// The `delegate` provides consumer-specific behavior via the `LoopDelegate` trait.
///
/// Maps to: `src/agent/agentic_loop.rs` → `run_agentic_loop()`
pub async fn run_agentic_loop(
    delegate: &dyn LoopDelegate,
    messages: &mut Vec<ChatMessage>,
    config: &AgenticLoopConfig,
) -> Result<LoopOutcome, String> {
    let tool_defs = delegate.tool_definitions();
    let mut recorded_tool_calls: Vec<RecordedToolCall> = Vec::new();

    info!(
        tool_count = tool_defs.len(),
        tools = ?tool_defs.iter().map(|t| &t.name).collect::<Vec<_>>(),
        initial_message_count = messages.len(),
        "🔄 Entering agentic loop"
    );

    for iteration in 1..=config.max_iterations {
        // ── Step 0: Check signals ──
        match delegate.check_signals().await {
            LoopSignal::Continue => {}
            LoopSignal::Stop => {
                info!(iteration, "🛑 Loop stopped by signal");
                return Ok(LoopOutcome::Stopped {
                    tool_calls: recorded_tool_calls,
                });
            }
            LoopSignal::InjectMessage(msg) => {
                info!(iteration, msg_len = msg.len(), "💉 Injecting user message");
                messages.push(ChatMessage::user(&msg));
            }
        }

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

        // ── Step 1: Pre-LLM hook ──
        if let Some(outcome) = delegate.before_llm_call(messages, iteration).await {
            info!(iteration, "🔚 Loop ended by before_llm_call hook");
            return Ok(outcome);
        }

        // ── Step 2: Call LLM ──
        info!("📤 Step 2: Calling LLM...");
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
        let response = delegate.call_llm(messages, &tool_defs).await?;
        let llm_elapsed = llm_start.elapsed();

        info!(
            elapsed_ms = llm_elapsed.as_millis(),
            finish_reason = ?response.finish_reason,
            "📥 LLM responded in {}ms (finish_reason={:?})",
            llm_elapsed.as_millis(),
            response.finish_reason
        );

        match response.result {
            crate::llm::LlmOutput::Text(text) => {
                info!(
                    text_len = text.len(),
                    total_tool_calls = recorded_tool_calls.len(),
                    "💬 LLM returned TEXT response ({} chars, {} tool calls recorded)",
                    text.len(), recorded_tool_calls.len()
                );
                debug!(text = %text, "Full text response from LLM");
                println!("  💬 LLM returned text: {}", crate::utils::truncate_str(&text, 100));

                // ── Step 3: Let delegate decide what to do with text ──
                match delegate.handle_text_response(&text, &recorded_tool_calls).await {
                    TextAction::Return(outcome) => return Ok(outcome),
                    TextAction::Continue => {
                        debug!("Delegate chose to continue after text response");
                        messages.push(ChatMessage::assistant(&text));
                    }
                }
            }

            crate::llm::LlmOutput::ToolCalls {
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
                println!("  🔧 LLM wants to call {} tool(s)", tool_calls.len());

                // Handle truncated responses
                if response.finish_reason == crate::llm::FinishReason::Length {
                    warn!(
                        "⚠️  Response was truncated (finish_reason=Length), discarding tool calls"
                    );
                    println!("  ⚠️  Response was truncated, discarding tool calls");
                    if let Some(text) = content {
                        messages.push(ChatMessage::assistant(text));
                    }
                    messages.push(ChatMessage::user(
                        "Your previous response was truncated. Please try a simpler approach.",
                    ));
                    info!("Injected truncation recovery message, continuing to next iteration");
                    continue;
                }

                // Add assistant message with tool calls to context
                info!("📝 Adding assistant message with tool calls to conversation context");
                messages.push(ChatMessage::assistant_with_tool_calls(
                    content,
                    tool_calls.clone(),
                ));

                // Execute each tool call
                for (i, tc) in tool_calls.iter().enumerate() {
                    info!(
                        index = i,
                        tool = %tc.name,
                        "⚙️  Executing tool [{}/{}]: {}",
                        i + 1, tool_calls.len(), tc.name
                    );

                    let tool_start = std::time::Instant::now();
                    let result = delegate.execute_tool(&tc.name, tc.arguments.clone()).await;
                    let tool_elapsed = tool_start.elapsed();

                    let result_msg = crate::tools::process_tool_result(&tc.name, &tc.id, &result);

                    match &result {
                        Ok(output) => {
                            info!(
                                tool = %tc.name,
                                elapsed_ms = tool_elapsed.as_millis(),
                                output_len = output.len(),
                                "✅ Tool '{}' succeeded in {}ms ({} chars output)",
                                tc.name, tool_elapsed.as_millis(), output.len()
                            );
                            debug!(tool = %tc.name, output = %output, "Full tool output");
                            println!(
                                "  ✅ Tool '{}' succeeded: {}",
                                tc.name,
                                crate::utils::truncate_str(output, 80)
                            );
                        }
                        Err(e) => {
                            tracing::error!(
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
                    recorded_tool_calls.push(RecordedToolCall {
                        name: tc.name.clone(),
                        params: tc.arguments.clone(),
                        result,
                    });

                    messages.push(result_msg);
                }

                info!(
                    message_count = messages.len(),
                    total_recorded = recorded_tool_calls.len(),
                    "🔄 All tools executed. Looping back to LLM."
                );
            }
        }

        // ── Step 4: Post-iteration hook ──
        delegate.after_iteration(iteration).await;
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
