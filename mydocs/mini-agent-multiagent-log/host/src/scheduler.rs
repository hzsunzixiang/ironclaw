
//! # Job Scheduler for Parallel Execution
//!
//! Maps to: `src/agent/scheduler.rs` in IronClaw.
//!
//! Manages background jobs that run independently of the interactive chat session.
//! Each job gets its own agentic loop with a `JobDelegate` that implements
//! `LoopDelegate`.
//!
//! ## Architecture
//!
//! ```text
//! Scheduler
//! ├── jobs: HashMap<Uuid, ScheduledJob>
//! │   ├── Job #1 (Worker + mpsc channel)
//! │   ├── Job #2 (Worker + mpsc channel)
//! │   └── ...
//! └── dispatch_job() → spawns Worker → runs agentic loop
//! ```
//!
//! ## IronClaw Mapping
//!
//! | IronClaw                       | Mini-Agent                              |
//! |-------------------------------|------------------------------------------|
//! | `Scheduler` + `ContextManager` | `Scheduler` (simplified, in-memory)     |
//! | `Worker` + `JobDelegate`       | `Worker` + `JobDelegate`                |
//! | `WorkerMessage` (Start/Stop)   | `WorkerMessage` (Start/Stop)            |
//! | `ScheduledJob` (handle + tx)   | `ScheduledJob` (handle + tx + metadata) |
//! | Database-backed job state      | In-memory `JobInfo`                     |

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tokio::sync::{mpsc, RwLock};
use tokio::task::JoinHandle;
use uuid::Uuid;

use tracing::{debug, error, info, warn};

use crate::agentic_loop::{
    run_agentic_loop, AgenticLoopConfig, LoopDelegate, LoopOutcome, LoopSignal, RecordedToolCall,
    TextAction,
};
use crate::llm::{ChatMessage, LlmProvider, ToolDefinition};
use crate::memory::MemoryStore;
use crate::tools::{execute_tool_with_safety, ToolRegistry};

// ============================================================================
// Job State and Metadata
// ============================================================================

/// State of a background job.
///
/// Maps to: `src/context/mod.rs` → `JobState`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobState {
    /// Job is queued but not yet started.
    Pending,
    /// Job is actively running.
    InProgress,
    /// Job completed successfully.
    Completed,
    /// Job failed with an error.
    Failed,
    /// Job was cancelled by the user.
    Cancelled,
}

impl std::fmt::Display for JobState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JobState::Pending => write!(f, "⏳ Pending"),
            JobState::InProgress => write!(f, "🔄 In Progress"),
            JobState::Completed => write!(f, "✅ Completed"),
            JobState::Failed => write!(f, "❌ Failed"),
            JobState::Cancelled => write!(f, "🛑 Cancelled"),
        }
    }
}

/// Information about a job (visible to the user).
///
/// In IronClaw, this is stored in the database via `ContextManager`.
/// Here we keep it in memory for simplicity.
#[derive(Debug, Clone)]
pub struct JobInfo {
    pub id: Uuid,
    pub short_id: String,
    pub description: String,
    pub state: JobState,
    pub result: Option<String>,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub tool_calls_count: usize,
}

// ============================================================================
// Worker Messages
// ============================================================================

/// Message to send to a worker.
///
/// Maps to: `src/agent/scheduler.rs` → `WorkerMessage`
#[derive(Debug)]
pub enum WorkerMessage {
    /// Start working on the job.
    Start,
    /// Stop the job.
    Stop,
}

// ============================================================================
// Scheduled Job
// ============================================================================

/// A running job with its task handle and communication channel.
///
/// Maps to: `src/agent/scheduler.rs` → `ScheduledJob`
struct ScheduledJob {
    #[allow(dead_code)]
    handle: JoinHandle<()>,
    tx: mpsc::Sender<WorkerMessage>,
}

// ============================================================================
// JobDelegate — LoopDelegate for background jobs
// ============================================================================

/// Delegate for background job execution.
///
/// Maps to: `src/worker/job.rs` → `JobDelegate`
///
/// Unlike `ChatDelegate` which runs in the interactive session context,
/// `JobDelegate` runs independently in a spawned task. It uses `Arc`-based
/// ownership instead of borrows because it must be `'static` for `tokio::spawn`.
struct JobDelegate {
    job_id: Uuid,
    llm: Arc<dyn LlmProvider>,
    registry: Arc<ToolRegistry>,
    stop_rx: Arc<tokio::sync::Mutex<mpsc::Receiver<WorkerMessage>>>,
    stopped: Arc<tokio::sync::Mutex<bool>>,
}

#[async_trait]
impl LoopDelegate for JobDelegate {
    async fn check_signals(&self) -> LoopSignal {
        // Non-blocking check for stop messages
        let mut rx = self.stop_rx.lock().await;
        match rx.try_recv() {
            Ok(WorkerMessage::Stop) => {
                info!(job_id = %&self.job_id.to_string()[..8], "🛑 Job received stop signal");
                *self.stopped.lock().await = true;
                LoopSignal::Stop
            }
            Ok(WorkerMessage::Start) => {
                debug!(job_id = %&self.job_id.to_string()[..8], "Job received start signal (already running)");
                LoopSignal::Continue
            }
            Err(_) => LoopSignal::Continue,
        }
    }

    async fn before_llm_call(
        &self,
        _messages: &mut Vec<ChatMessage>,
        _iteration: usize,
    ) -> Option<LoopOutcome> {
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
        recorded_tool_calls: &[RecordedToolCall],
    ) -> TextAction {
        info!(
            job_id = %&self.job_id.to_string()[..8],
            text_len = text.len(),
            tool_calls = recorded_tool_calls.len(),
            "📋 Job completed with text response"
        );
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
        execute_tool_with_safety(&self.registry, tool_name, arguments).await
    }

    fn tool_definitions(&self) -> Vec<ToolDefinition> {
        self.registry.definitions()
    }

    async fn after_iteration(&self, iteration: usize) {
        debug!(
            job_id = %&self.job_id.to_string()[..8],
            iteration,
            "Job iteration {} completed",
            iteration
        );
    }
}

// ============================================================================
// Scheduler
// ============================================================================

/// Job scheduler for parallel execution.
///
/// Maps to: `src/agent/scheduler.rs` → `Scheduler`
///
/// Maintains a map of running jobs, each with a `Worker` and an `mpsc`
/// channel for `WorkerMessage` (Start, Stop).
pub struct Scheduler {
    llm: Arc<dyn LlmProvider>,
    registry: Arc<ToolRegistry>,
    memory_store: Arc<RwLock<MemoryStore>>,
    model_name: String,
    /// Running jobs.
    jobs: Arc<RwLock<HashMap<Uuid, ScheduledJob>>>,
    /// Job info (visible to user, persists after completion).
    job_info: Arc<RwLock<HashMap<Uuid, JobInfo>>>,
    /// Max parallel jobs.
    max_parallel_jobs: usize,
    /// Agentic loop config for jobs.
    loop_config: AgenticLoopConfig,
}

impl Scheduler {
    /// Create a new scheduler.
    pub fn new(
        llm: Arc<dyn LlmProvider>,
        registry: Arc<ToolRegistry>,
        memory_store: Arc<RwLock<MemoryStore>>,
        model_name: String,
    ) -> Self {
        info!("📋 Scheduler initialized (max_parallel_jobs=3)");
        Self {
            llm,
            registry,
            memory_store,
            model_name,
            jobs: Arc::new(RwLock::new(HashMap::new())),
            job_info: Arc::new(RwLock::new(HashMap::new())),
            max_parallel_jobs: 3,
            loop_config: AgenticLoopConfig {
                max_iterations: 15, // Jobs get more iterations than chat
                ..Default::default()
            },
        }
    }

    /// Dispatch a new background job.
    ///
    /// Maps to: `Scheduler::dispatch_job()` in IronClaw.
    ///
    /// Creates the job context, persists metadata, and spawns a worker task.
    pub async fn dispatch_job(&self, description: &str) -> Result<Uuid, String> {
        let job_id = Uuid::new_v4();
        let short_id = job_id.to_string()[..8].to_string();

        // Check capacity
        {
            let jobs = self.jobs.read().await;
            if jobs.len() >= self.max_parallel_jobs {
                return Err(format!(
                    "Maximum parallel jobs ({}) exceeded. Use /jobs to check status or /cancel to free a slot.",
                    self.max_parallel_jobs
                ));
            }
        }

        info!(
            job_id = %short_id,
            description = %description,
            "🚀 Dispatching new job"
        );

        // Create job info
        let info = JobInfo {
            id: job_id,
            short_id: short_id.clone(),
            description: description.to_string(),
            state: JobState::Pending,
            result: None,
            error: None,
            created_at: Utc::now(),
            completed_at: None,
            tool_calls_count: 0,
        };

        {
            self.job_info.write().await.insert(job_id, info);
        }

        // Create worker channel
        let (tx, rx) = mpsc::channel(16);

        // Build system prompt for the job
        let memory_content = {
            let store = self.memory_store.read().await;
            store.memory_content_truncated(self.loop_config.max_memory_words)
        };
        let system_prompt = build_job_system_prompt(&self.model_name, &memory_content, description);

        // Clone Arcs for the spawned task
        let llm = Arc::clone(&self.llm);
        let registry = Arc::clone(&self.registry);
        let job_info = Arc::clone(&self.job_info);
        let loop_config_max_iter = self.loop_config.max_iterations;
        let loop_config = AgenticLoopConfig {
            max_iterations: loop_config_max_iter,
            ..Default::default()
        };

        // Spawn worker task
        let handle = tokio::spawn(async move {
            // Wait for Start message
            let stop_rx = Arc::new(tokio::sync::Mutex::new(rx));
            {
                let mut rx_guard = stop_rx.lock().await;
                match rx_guard.recv().await {
                    Some(WorkerMessage::Start) => {
                        info!(job_id = %short_id, "▶️  Job worker started");
                    }
                    Some(WorkerMessage::Stop) => {
                        info!(job_id = %short_id, "🛑 Job cancelled before start");
                        let mut infos = job_info.write().await;
                        if let Some(info) = infos.get_mut(&job_id) {
                            info.state = JobState::Cancelled;
                            info.completed_at = Some(Utc::now());
                        }
                        return;
                    }
                    None => {
                        error!(job_id = %short_id, "Job channel closed before start");
                        return;
                    }
                }
            }

            // Transition to InProgress
            {
                let mut infos = job_info.write().await;
                if let Some(info) = infos.get_mut(&job_id) {
                    info.state = JobState::InProgress;
                }
            }

            // Create delegate
            let delegate = JobDelegate {
                job_id,
                llm,
                registry,
                stop_rx,
                stopped: Arc::new(tokio::sync::Mutex::new(false)),
            };

            // Build initial messages
            let mut messages = vec![system_prompt, ChatMessage::user(&format!(
                "Please complete this task: {}", 
                job_info.read().await.get(&job_id).map(|i| i.description.as_str()).unwrap_or("unknown")
            ))];

            // Run agentic loop
            let outcome = run_agentic_loop(&delegate, &mut messages, &loop_config).await;

            // Update job info with result
            let mut infos = job_info.write().await;
            if let Some(info) = infos.get_mut(&job_id) {
                match outcome {
                    Ok(LoopOutcome::Response { text, tool_calls }) => {
                        info.state = JobState::Completed;
                        info.result = Some(text);
                        info.tool_calls_count = tool_calls.len();
                        info.completed_at = Some(Utc::now());
                        info!(
                            job_id = %info.short_id,
                            "✅ Job completed successfully"
                        );
                    }
                    Ok(LoopOutcome::Stopped { tool_calls }) => {
                        info.state = JobState::Cancelled;
                        info.tool_calls_count = tool_calls.len();
                        info.completed_at = Some(Utc::now());
                        info!(job_id = %info.short_id, "🛑 Job was stopped");
                    }
                    Ok(LoopOutcome::MaxIterations { tool_calls }) => {
                        info.state = JobState::Failed;
                        info.error = Some("Reached maximum iterations".to_string());
                        info.tool_calls_count = tool_calls.len();
                        info.completed_at = Some(Utc::now());
                        warn!(job_id = %info.short_id, "⚠️  Job reached max iterations");
                    }
                    Err(e) => {
                        info.state = JobState::Failed;
                        info.error = Some(e.clone());
                        info.completed_at = Some(Utc::now());
                        error!(job_id = %info.short_id, error = %e, "❌ Job failed");
                    }
                }
            }
        });

        // Send Start message
        tx.send(WorkerMessage::Start)
            .await
            .map_err(|_| "Worker died before receiving Start message".to_string())?;

        // Store scheduled job
        {
            self.jobs.write().await.insert(job_id, ScheduledJob { handle, tx });
        }

        // Spawn cleanup task
        let jobs = Arc::clone(&self.jobs);
        tokio::spawn(async move {
            loop {
                let finished = {
                    let jobs_read = jobs.read().await;
                    match jobs_read.get(&job_id) {
                        Some(scheduled) => scheduled.handle.is_finished(),
                        None => true,
                    }
                };

                if finished {
                    jobs.write().await.remove(&job_id);
                    debug!(job_id = %job_id.to_string()[..8], "🧹 Cleaned up finished job");
                    break;
                }

                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        });

        info!(job_id = %job_id.to_string()[..8], "📋 Job scheduled for execution");
        Ok(job_id)
    }

    /// Cancel a running job.
    pub async fn cancel_job(&self, job_id: Uuid) -> Result<(), String> {
        let tx = {
            let jobs = self.jobs.read().await;
            match jobs.get(&job_id) {
                Some(scheduled) => scheduled.tx.clone(),
                None => {
                    // Check if job exists but already finished
                    let infos = self.job_info.read().await;
                    if infos.contains_key(&job_id) {
                        return Err("Job has already finished".to_string());
                    }
                    return Err("Job not found".to_string());
                }
            }
        };

        tx.send(WorkerMessage::Stop)
            .await
            .map_err(|_| "Worker channel closed".to_string())?;

        info!(job_id = %job_id.to_string()[..8], "🛑 Cancel signal sent to job");
        Ok(())
    }

    /// Get info about a specific job.
    #[allow(dead_code)]
    pub async fn get_job_info(&self, job_id: Uuid) -> Option<JobInfo> {
        self.job_info.read().await.get(&job_id).cloned()
    }

    /// Find a job by short ID prefix.
    pub async fn find_job_by_short_id(&self, short_id: &str) -> Option<JobInfo> {
        let infos = self.job_info.read().await;
        infos
            .values()
            .find(|info| info.short_id.starts_with(short_id) || info.id.to_string().starts_with(short_id))
            .cloned()
    }

    /// List all jobs.
    pub async fn list_jobs(&self) -> Vec<JobInfo> {
        let infos = self.job_info.read().await;
        let mut jobs: Vec<_> = infos.values().cloned().collect();
        jobs.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        jobs
    }

    /// Get count of running jobs.
    pub async fn running_count(&self) -> usize {
        self.jobs.read().await.len()
    }
}

// ============================================================================
// Job System Prompt
// ============================================================================

/// Build the system prompt for a background job.
///
/// Jobs get a focused system prompt that includes the task description
/// and available memory, but is more task-oriented than the chat prompt.
fn build_job_system_prompt(model: &str, memory_content: &str, task_description: &str) -> ChatMessage {
    let memory_section = if memory_content.trim().is_empty()
        || memory_content.trim() == "# Memory"
    {
        String::new()
    } else {
        format!(
            "\n\n## Long-Term Memory\n\
             The following is persistent memory from previous sessions. \
             Use this information when relevant:\n\n\
             <memory>\n{}\n</memory>",
            memory_content.trim()
        )
    };

    ChatMessage::system(format!(
        "You are a background worker agent powered by the {} model. \
         You are executing a background job independently of the user's interactive session. \
         \n\n\
         ## Your Task\n\
         {}\n\n\
         ## Instructions\n\
         - Focus on completing the task efficiently.\n\
         - Use available tools (calculator, memory tools) as needed.\n\
         - When done, provide a clear summary of what you accomplished.\n\
         - If you encounter errors, explain what went wrong.\n\
         - Do NOT ask the user for clarification — work with what you have.{}",
        model, task_description, memory_section
    ))
}
