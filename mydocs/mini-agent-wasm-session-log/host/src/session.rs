//! # Session / Thread / Turn Model
//!
//! THIS IS THE KEY ADDITION compared to mini-agent-wasm.
//!
//! Maps to IronClaw's src/agent/session.rs:
//!   Session → Thread → Turn hierarchy
//!   Thread::messages() rebuilds ChatMessage list from Turn history
//!   TurnToolCall records each tool invocation within a turn
//!
//! The key insight: the LLM needs the FULL conversation history as context.
//! Instead of building messages from scratch each time (mini-agent-wasm),
//! we store structured Turns and rebuild messages from them.
//!
//! ```text
//! Session (per user)
//! └── Thread (per conversation — can have many)
//!     └── Turn (per request/response pair)
//!         ├── user_input: String
//!         ├── response: Option<String>
//!         ├── tool_calls: Vec<TurnToolCall>
//!         └── state: TurnState
//! ```

use chrono::{DateTime, Utc};
use std::collections::HashMap;
use uuid::Uuid;

use tracing::{debug, info, trace};

use crate::llm::{ChatMessage, ToolCall};
use crate::utils::truncate_str;

// ============================================================================
// Turn — A single request/response pair
// ============================================================================

/// State of a turn.
///
/// Maps to: `src/agent/session.rs` → `TurnState`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnState {
    /// Turn is being processed by the agent.
    Processing,
    /// Turn completed successfully with a response.
    Completed,
    /// Turn failed with an error.
    Failed,
}

/// A tool call recorded within a turn.
///
/// Maps to: `src/agent/session.rs` → `TurnToolCall`
#[derive(Debug, Clone)]
pub struct TurnToolCall {
    /// Tool name.
    pub name: String,
    /// Parameters passed to the tool (JSON).
    pub parameters: serde_json::Value,
    /// Result from the tool (JSON), if completed.
    pub result: Option<serde_json::Value>,
    /// Error message, if the tool failed.
    pub error: Option<String>,
}

/// A single turn (request/response pair) in a thread.
///
/// Maps to: `src/agent/session.rs` → `Turn`
///
/// A turn represents one user input and the agent's complete response,
/// including any tool calls made along the way.
#[derive(Debug, Clone)]
pub struct Turn {
    /// Turn number (0-indexed).
    pub turn_number: usize,
    /// User input that started this turn.
    pub user_input: String,
    /// Agent response (if completed).
    pub response: Option<String>,
    /// Tool calls made during this turn.
    pub tool_calls: Vec<TurnToolCall>,
    /// Turn state.
    pub state: TurnState,
    /// When the turn started.
    pub started_at: DateTime<Utc>,
    /// When the turn completed.
    pub completed_at: Option<DateTime<Utc>>,
    /// Error message (if failed).
    pub error: Option<String>,
}

impl Turn {
    /// Create a new turn.
    pub fn new(turn_number: usize, user_input: impl Into<String>) -> Self {
        let input = user_input.into();
        debug!(
            turn_number = turn_number,
            user_input = %truncate_str(&input, 80),
            "Creating new Turn #{}",
            turn_number
        );
        Self {
            turn_number,
            user_input: input,
            response: None,
            tool_calls: Vec::new(),
            state: TurnState::Processing,
            started_at: Utc::now(),
            completed_at: None,
            error: None,
        }
    }

    /// Record a tool call in this turn.
    pub fn record_tool_call(&mut self, name: &str, parameters: serde_json::Value) {
        debug!(
            turn = self.turn_number,
            tool_name = %name,
            "Recording tool call '{}' in Turn #{}",
            name, self.turn_number
        );
        self.tool_calls.push(TurnToolCall {
            name: name.to_string(),
            parameters,
            result: None,
            error: None,
        });
    }

    /// Record a tool result for the last tool call.
    pub fn record_tool_result(&mut self, result: serde_json::Value) {
        if let Some(tc) = self.tool_calls.last_mut() {
            debug!(
                turn = self.turn_number,
                tool_name = %tc.name,
                "Recording tool result for '{}' in Turn #{}",
                tc.name, self.turn_number
            );
            tc.result = Some(result);
        }
    }

    /// Record a tool error for the last tool call.
    pub fn record_tool_error(&mut self, error: String) {
        if let Some(tc) = self.tool_calls.last_mut() {
            debug!(
                turn = self.turn_number,
                tool_name = %tc.name,
                error = %error,
                "Recording tool error for '{}' in Turn #{}",
                tc.name, self.turn_number
            );
            tc.error = Some(error);
        }
    }

    /// Complete the turn with a response.
    pub fn complete(&mut self, response: impl Into<String>) {
        let resp = response.into();
        info!(
            turn = self.turn_number,
            response_len = resp.len(),
            tool_call_count = self.tool_calls.len(),
            "✅ Turn #{} completed ({} chars response, {} tool calls)",
            self.turn_number, resp.len(), self.tool_calls.len()
        );
        self.response = Some(resp);
        self.state = TurnState::Completed;
        self.completed_at = Some(Utc::now());
    }

    /// Fail the turn with an error.
    pub fn fail(&mut self, error: impl Into<String>) {
        let err = error.into();
        info!(
            turn = self.turn_number,
            error = %err,
            "❌ Turn #{} failed: {}",
            self.turn_number, err
        );
        self.error = Some(err);
        self.state = TurnState::Failed;
        self.completed_at = Some(Utc::now());
    }
}

// ============================================================================
// Thread — A conversation containing turns
// ============================================================================

/// State of a thread.
///
/// Maps to: `src/agent/session.rs` → `ThreadState`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadState {
    /// Thread is idle, waiting for input.
    Idle,
    /// Thread is processing a turn.
    Processing,
}

/// A conversation thread containing turns.
///
/// Maps to: `src/agent/session.rs` → `Thread`
///
/// The thread is the core unit of conversation. It maintains an ordered
/// list of turns and can rebuild the full ChatMessage history from them.
#[derive(Debug, Clone)]
pub struct Thread {
    /// Unique thread ID.
    pub id: Uuid,
    /// Parent session ID.
    #[allow(dead_code)]
    pub session_id: Uuid,
    /// Current state.
    pub state: ThreadState,
    /// Turns in this thread.
    pub turns: Vec<Turn>,
    /// When the thread was created.
    pub created_at: DateTime<Utc>,
    /// When the thread was last updated.
    pub updated_at: DateTime<Utc>,
}

impl Thread {
    /// Create a new thread.
    pub fn new(session_id: Uuid) -> Self {
        let now = Utc::now();
        let thread = Self {
            id: Uuid::new_v4(),
            session_id,
            state: ThreadState::Idle,
            turns: Vec::new(),
            created_at: now,
            updated_at: now,
        };
        info!(
            thread_id = %&thread.id.to_string()[..8],
            session_id = %&session_id.to_string()[..8],
            "🧵 New thread created"
        );
        thread
    }

    /// Start a new turn with user input.
    pub fn start_turn(&mut self, user_input: impl Into<String>) -> &mut Turn {
        let turn_number = self.turns.len();
        let input = user_input.into();
        info!(
            thread_id = %&self.id.to_string()[..8],
            turn_number = turn_number,
            user_input = %truncate_str(&input, 80),
            "📝 Starting Turn #{} in thread",
            turn_number
        );
        let turn = Turn::new(turn_number, input);
        self.turns.push(turn);
        self.state = ThreadState::Processing;
        self.updated_at = Utc::now();
        &mut self.turns[turn_number]
    }

    /// Complete the current turn with a response.
    pub fn complete_turn(&mut self, response: impl Into<String>) {
        let resp = response.into();
        if let Some(turn) = self.turns.last_mut() {
            info!(
                thread_id = %&self.id.to_string()[..8],
                turn = turn.turn_number,
                "Completing Turn #{} in thread",
                turn.turn_number
            );
            turn.complete(resp);
        }
        self.state = ThreadState::Idle;
        self.updated_at = Utc::now();
    }

    /// Fail the current turn with an error.
    pub fn fail_turn(&mut self, error: impl Into<String>) {
        let err = error.into();
        if let Some(turn) = self.turns.last_mut() {
            info!(
                thread_id = %&self.id.to_string()[..8],
                turn = turn.turn_number,
                error = %err,
                "Failing Turn #{} in thread",
                turn.turn_number
            );
            turn.fail(err);
        }
        self.state = ThreadState::Idle;
        self.updated_at = Utc::now();
    }

    /// Get the last turn.
    #[allow(dead_code)]
    pub fn last_turn(&self) -> Option<&Turn> {
        self.turns.last()
    }

    /// Get the last turn mutably.
    pub fn last_turn_mut(&mut self) -> Option<&mut Turn> {
        self.turns.last_mut()
    }

    /// Rebuild ChatMessage history from turns.
    ///
    /// This is THE KEY METHOD — it converts the structured Turn history
    /// into the flat ChatMessage list that the LLM expects.
    ///
    /// Maps to: `src/agent/session.rs` → `Thread::messages()`
    ///
    /// For each turn, it produces:
    ///   1. User message (the input)
    ///   2. If tool calls: assistant message declaring tool calls + tool result messages
    ///   3. If response: assistant message with the final text
    pub fn messages(&self) -> Vec<ChatMessage> {
        let mut messages = Vec::new();

        debug!(
            thread_id = %&self.id.to_string()[..8],
            turn_count = self.turns.len(),
            "🔄 Rebuilding ChatMessage history from {} turns",
            self.turns.len()
        );

        for (turn_idx, turn) in self.turns.iter().enumerate() {
            // 1. User message
            messages.push(ChatMessage::user(&turn.user_input));
            trace!(
                turn = turn_idx,
                "Added user message for Turn #{}",
                turn_idx
            );

            // 2. Tool calls (if any)
            if !turn.tool_calls.is_empty() {
                debug!(
                    turn = turn_idx,
                    tool_call_count = turn.tool_calls.len(),
                    "Turn #{} has {} tool call(s) to replay",
                    turn_idx, turn.tool_calls.len()
                );

                // Generate synthetic tool call IDs (deterministic from turn/tool indices)
                let tool_calls_with_ids: Vec<(String, &TurnToolCall)> = turn
                    .tool_calls
                    .iter()
                    .enumerate()
                    .map(|(tc_idx, tc)| (format!("call_{turn_idx}_{tc_idx}"), tc))
                    .collect();

                // Assistant message declaring the tool calls
                let tool_calls: Vec<ToolCall> = tool_calls_with_ids
                    .iter()
                    .map(|(call_id, tc)| ToolCall {
                        id: call_id.clone(),
                        name: tc.name.clone(),
                        arguments: tc.parameters.clone(),
                    })
                    .collect();
                messages.push(ChatMessage::assistant_with_tool_calls(None, tool_calls));

                // Tool result messages
                for (call_id, tc) in &tool_calls_with_ids {
                    let content = if let Some(ref err) = tc.error {
                        format!("Error: {}", err)
                    } else if let Some(ref res) = tc.result {
                        match res {
                            serde_json::Value::String(s) => s.clone(),
                            other => other.to_string(),
                        }
                    } else {
                        "OK".to_string()
                    };
                    messages.push(ChatMessage::tool_result(call_id, &tc.name, content));
                }
            }

            // 3. Assistant response (if completed)
            if let Some(ref response) = turn.response {
                messages.push(ChatMessage::assistant(response));
                trace!(
                    turn = turn_idx,
                    "Added assistant response for Turn #{}",
                    turn_idx
                );
            }
        }

        info!(
            thread_id = %&self.id.to_string()[..8],
            total_messages = messages.len(),
            turn_count = self.turns.len(),
            "📨 Rebuilt {} messages from {} turns",
            messages.len(), self.turns.len()
        );

        messages
    }
}

// ============================================================================
// Session — Contains one or more threads
// ============================================================================

/// A session containing one or more threads.
///
/// Maps to: `src/agent/session.rs` → `Session`
///
/// In IronClaw, a session is per-user. Here we have a single CLI user,
/// but the structure supports multiple threads within the session.
#[derive(Debug)]
pub struct Session {
    /// Unique session ID.
    pub id: Uuid,
    /// User ID that owns this session.
    pub user_id: String,
    /// Active thread ID.
    pub active_thread: Option<Uuid>,
    /// All threads in this session.
    pub threads: HashMap<Uuid, Thread>,
    /// When the session was created.
    pub created_at: DateTime<Utc>,
    /// When the session was last active.
    pub last_active_at: DateTime<Utc>,
}

impl Session {
    /// Create a new session.
    pub fn new(user_id: impl Into<String>) -> Self {
        let uid = user_id.into();
        let now = Utc::now();
        let session = Self {
            id: Uuid::new_v4(),
            user_id: uid.clone(),
            active_thread: None,
            threads: HashMap::new(),
            created_at: now,
            last_active_at: now,
        };
        info!(
            session_id = %&session.id.to_string()[..8],
            user_id = %uid,
            "🆕 New session created"
        );
        session
    }

    /// Create a new thread in this session.
    pub fn create_thread(&mut self) -> &mut Thread {
        let thread = Thread::new(self.id);
        let thread_id = thread.id;
        self.active_thread = Some(thread_id);
        self.last_active_at = Utc::now();
        info!(
            session_id = %&self.id.to_string()[..8],
            thread_id = %&thread_id.to_string()[..8],
            thread_count = self.threads.len() + 1,
            "🧵 Created new thread (now {} threads in session)",
            self.threads.len() + 1
        );
        self.threads.entry(thread_id).or_insert(thread)
    }

    /// Get the active thread.
    pub fn active_thread(&self) -> Option<&Thread> {
        self.active_thread.and_then(|id| self.threads.get(&id))
    }

    /// Get the active thread mutably.
    #[allow(dead_code)]
    pub fn active_thread_mut(&mut self) -> Option<&mut Thread> {
        self.active_thread.and_then(|id| self.threads.get_mut(&id))
    }

    /// Get a thread by ID mutably.
    ///
    /// Provides encapsulated access to threads without exposing the
    /// internal HashMap directly.
    pub fn thread_mut(&mut self, id: Uuid) -> Option<&mut Thread> {
        self.threads.get_mut(&id)
    }

    /// Get or create the active thread.
    pub fn get_or_create_thread(&mut self) -> &mut Thread {
        match self.active_thread {
            Some(id) if self.threads.contains_key(&id) => self.threads.get_mut(&id).unwrap(),
            _ => self.create_thread(),
        }
    }

    /// Switch to a different thread.
    pub fn switch_thread(&mut self, thread_id: Uuid) -> bool {
        if self.threads.contains_key(&thread_id) {
            let old_thread = self.active_thread;
            self.active_thread = Some(thread_id);
            self.last_active_at = Utc::now();
            info!(
                session_id = %&self.id.to_string()[..8],
                old_thread = %old_thread.map(|id| id.to_string()[..8].to_string()).unwrap_or_else(|| "none".to_string()),
                new_thread = %&thread_id.to_string()[..8],
                "🔀 Switched active thread"
            );
            true
        } else {
            debug!(
                thread_id = %&thread_id.to_string()[..8],
                "Thread not found in session"
            );
            false
        }
    }

    /// List all threads with summary info.
    pub fn list_threads(&self) -> Vec<ThreadSummary> {
        let mut summaries: Vec<_> = self
            .threads
            .values()
            .map(|t| {
                let first_input = t.turns.first().map(|turn| {
                    truncate_str(&turn.user_input, 40)
                });
                ThreadSummary {
                    id: t.id,
                    is_active: self.active_thread == Some(t.id),
                    turn_count: t.turns.len(),
                    created_at: t.created_at,
                    first_input,
                }
            })
            .collect();
        summaries.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        summaries
    }
}

/// Summary info for a thread (used in /threads listing).
pub struct ThreadSummary {
    pub id: Uuid,
    pub is_active: bool,
    pub turn_count: usize,
    pub created_at: DateTime<Utc>,
    pub first_input: Option<String>,
}
