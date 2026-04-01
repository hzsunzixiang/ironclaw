
//! # Message Router
//!
//! Maps to: `src/agent/router.rs` in IronClaw.
//!
//! Routes explicit `/commands` to appropriate handlers. Natural language
//! messages bypass the router entirely — they go directly to the ChatDelegate.
//!
//! ## Supported Commands
//!
//! | Command                | Intent                |
//! |------------------------|-----------------------|
//! | `/job <description>`   | CreateJob             |
//! | `/jobs`                | ListJobs              |
//! | `/status <id>`         | CheckJobStatus        |
//! | `/cancel <id>`         | CancelJob             |

use tracing::{debug, info};

/// Intent extracted from a message.
///
/// Maps to: `src/agent/router.rs` → `MessageIntent`
#[derive(Debug, Clone)]
pub enum MessageIntent {
    /// Create a new background job.
    CreateJob {
        description: String,
    },
    /// Check status of a job.
    CheckJobStatus {
        job_id: String,
    },
    /// Cancel a running job.
    CancelJob {
        job_id: String,
    },
    /// List all jobs.
    ListJobs,
    /// General conversation/question — goes to ChatDelegate.
    UserInput {
        content: String,
    },
}

/// Message router — parses `/commands` into `MessageIntent`.
///
/// Maps to: `src/agent/router.rs` → `Router`
pub struct Router;

impl Router {
    /// Route a user input string to the appropriate intent.
    ///
    /// Commands starting with `/job`, `/jobs`, `/status`, `/cancel` are
    /// parsed into their respective intents. Everything else is treated
    /// as a `UserInput` for the ChatDelegate.
    pub fn route(input: &str) -> MessageIntent {
        let trimmed = input.trim();

        // /job <description> — create a background job
        if let Some(desc) = trimmed.strip_prefix("/job ") {
            let desc = desc.trim();
            if desc.is_empty() {
                info!("Empty /job description, treating as user input");
                return MessageIntent::UserInput {
                    content: trimmed.to_string(),
                };
            }
            info!(description = %desc, "🚀 Routing to CreateJob");
            return MessageIntent::CreateJob {
                description: desc.to_string(),
            };
        }

        // /jobs — list all jobs
        if trimmed == "/jobs" {
            info!("📋 Routing to ListJobs");
            return MessageIntent::ListJobs;
        }

        // /status <id> — check job status
        if let Some(id) = trimmed.strip_prefix("/status ") {
            let id = id.trim();
            info!(job_id = %id, "📊 Routing to CheckJobStatus");
            return MessageIntent::CheckJobStatus {
                job_id: id.to_string(),
            };
        }

        // /cancel <id> — cancel a job
        if let Some(id) = trimmed.strip_prefix("/cancel ") {
            let id = id.trim();
            info!(job_id = %id, "🛑 Routing to CancelJob");
            return MessageIntent::CancelJob {
                job_id: id.to_string(),
            };
        }

        // Everything else is user input for the ChatDelegate
        debug!(input = %trimmed, "Routing to UserInput (no command match)");
        MessageIntent::UserInput {
            content: trimmed.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_route_create_job() {
        match Router::route("/job Analyze the project structure") {
            MessageIntent::CreateJob { description } => {
                assert_eq!(description, "Analyze the project structure");
            }
            other => panic!("Expected CreateJob, got {:?}", other),
        }
    }

    #[test]
    fn test_route_list_jobs() {
        assert!(matches!(Router::route("/jobs"), MessageIntent::ListJobs));
    }

    #[test]
    fn test_route_check_status() {
        match Router::route("/status abc123") {
            MessageIntent::CheckJobStatus { job_id } => {
                assert_eq!(job_id, "abc123");
            }
            other => panic!("Expected CheckJobStatus, got {:?}", other),
        }
    }

    #[test]
    fn test_route_cancel_job() {
        match Router::route("/cancel abc123") {
            MessageIntent::CancelJob { job_id } => {
                assert_eq!(job_id, "abc123");
            }
            other => panic!("Expected CancelJob, got {:?}", other),
        }
    }

    #[test]
    fn test_route_user_input() {
        match Router::route("What is 42 + 58?") {
            MessageIntent::UserInput { content } => {
                assert_eq!(content, "What is 42 + 58?");
            }
            other => panic!("Expected UserInput, got {:?}", other),
        }
    }

    #[test]
    fn test_route_empty_job_description() {
        // Empty /job description should be treated as user input
        match Router::route("/job ") {
            MessageIntent::UserInput { .. } => {}
            other => panic!("Expected UserInput for empty /job, got {:?}", other),
        }
    }
}
