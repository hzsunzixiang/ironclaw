//! Hello World WASM Tool for IronClaw.
//!
//! The simplest possible WASM tool — accepts a name and returns a greeting.
//! Use this as a starting point for building your own tools.
//!
//! # Example Usage
//!
//! ```json
//! {"name": "Alice"}
//! ```
//!
//! # Response
//!
//! ```json
//! {"greeting": "Hello, Alice! 👋 I'm a WASM tool running inside IronClaw's sandbox."}
//! ```

use serde::{Deserialize, Serialize};

// Generate bindings from the WIT interface.
wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",
});

use exports::near::agent::tool::{Guest, Request, Response};
use near::agent::host;

// ── Input / Output types ────────────────────────────────────────

#[derive(Deserialize)]
struct HelloInput {
    /// Name to greet (optional, defaults to "World")
    #[serde(default = "default_name")]
    name: String,
}

fn default_name() -> String {
    "World".to_string()
}

#[derive(Serialize)]
struct HelloOutput {
    greeting: String,
    timestamp_ms: u64,
}

// ── Tool implementation ─────────────────────────────────────────

struct HelloWorldTool;

impl Guest for HelloWorldTool {
    fn execute(req: Request) -> Response {
        // Parse input
        let input: HelloInput = match serde_json::from_str(&req.params) {
            Ok(i) => i,
            Err(e) => {
                return Response {
                    output: None,
                    error: Some(format!("Invalid input: {}", e)),
                }
            }
        };

        // Use host function: logging
        host::log(
            host::LogLevel::Info,
            &format!("Hello tool called with name: {}", input.name),
        );

        // Use host function: get current time
        let now = host::now_millis();

        // Build output
        let output = HelloOutput {
            greeting: format!(
                "Hello, {}! 👋 I'm a WASM tool running inside IronClaw's sandbox.",
                input.name
            ),
            timestamp_ms: now,
        };

        Response {
            output: Some(serde_json::to_string(&output).unwrap()),
            error: None,
        }
    }

    fn schema() -> String {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Name of the person to greet (defaults to 'World')"
                }
            },
            "required": ["name"]
        })
        .to_string()
    }

    fn description() -> String {
        "A simple hello-world tool that greets the given name. Use this to verify WASM tool loading works.".to_string()
    }
}

export!(HelloWorldTool);
