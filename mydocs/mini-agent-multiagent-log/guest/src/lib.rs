//! WASM Guest Tool — Calculator running inside the sandbox.
//!
//! This code compiles to a .wasm component. It can ONLY:
//! 1. Call host-provided functions (log, now_millis)
//! 2. Do pure computation (math, string manipulation, etc.)
//!
//! It CANNOT:
//! - Access the filesystem
//! - Access the network
//! - Read environment variables
//! - Spawn threads
//! - Do anything the host doesn't explicitly allow

use serde::{Deserialize, Serialize};

// Generate Rust bindings from the WIT file.
// Creates:
//   - `exports::demo::sandbox::tool::Guest` trait (we must implement)
//   - `exports::demo::sandbox::tool::{Request, Response}` types
//   - `demo::sandbox::host` module (functions we can call on the host)
wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../wit/tool.wit",
});

use exports::demo::sandbox::tool::{Guest, Request, Response};
use demo::sandbox::host;

// ── Input / Output types ────────────────────────────────────────

#[derive(Deserialize)]
struct CalculatorInput {
    /// Mathematical operation: "add", "sub", "mul", "div"
    operation: String,
    /// First operand
    a: f64,
    /// Second operand
    b: f64,
}

#[derive(Serialize)]
struct CalculatorOutput {
    /// The expression that was evaluated
    expression: String,
    /// The result of the calculation
    result: f64,
    /// Timestamp when the calculation was performed (from host)
    timestamp_ms: u64,
}

// ── Tool implementation ─────────────────────────────────────────

struct CalculatorTool;

impl Guest for CalculatorTool {
    fn execute(req: Request) -> Response {
        // Parse input JSON
        let input: CalculatorInput = match serde_json::from_str(&req.params) {
            Ok(i) => i,
            Err(e) => {
                return Response {
                    output: None,
                    error: Some(format!("Invalid input: {}", e)),
                }
            }
        };

        // Call host function: log (this is the ONLY way we can produce side effects)
        host::log(
            host::LogLevel::Info,
            &format!("Calculating: {} {} {}", input.a, input.operation, input.b),
        );

        // Pure computation — this is what WASM is great at
        let (result, expression) = match input.operation.as_str() {
            "add" => (input.a + input.b, format!("{} + {} = {}", input.a, input.b, input.a + input.b)),
            "sub" => (input.a - input.b, format!("{} - {} = {}", input.a, input.b, input.a - input.b)),
            "mul" => (input.a * input.b, format!("{} × {} = {}", input.a, input.b, input.a * input.b)),
            "div" => {
                if input.b == 0.0 {
                    host::log(host::LogLevel::Error, "Division by zero!");
                    return Response {
                        output: None,
                        error: Some("Division by zero".to_string()),
                    };
                }
                (input.a / input.b, format!("{} ÷ {} = {}", input.a, input.b, input.a / input.b))
            }
            _ => {
                return Response {
                    output: None,
                    error: Some(format!("Unknown operation: '{}'. Use: add, sub, mul, div", input.operation)),
                }
            }
        };

        // Call host function: get current time
        // The guest has NO other way to know the time — it's fully isolated!
        let now = host::now_millis();

        host::log(
            host::LogLevel::Info,
            &format!("Result: {} (at timestamp {})", result, now),
        );

        // Build output
        let output = CalculatorOutput {
            expression,
            result,
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
                "operation": {
                    "type": "string",
                    "description": "Math operation: add, sub, mul, div",
                    "enum": ["add", "sub", "mul", "div"]
                },
                "a": {
                    "type": "number",
                    "description": "First operand"
                },
                "b": {
                    "type": "number",
                    "description": "Second operand"
                }
            },
            "required": ["operation", "a", "b"]
        })
        .to_string()
    }

    fn name() -> String {
        "calculator".to_string()
    }

    fn description() -> String {
        "A sandboxed calculator tool. Performs basic math operations (add, sub, mul, div) inside a WASM sandbox.".to_string()
    }
}

// Register this struct as the implementation of the `tool` interface
export!(CalculatorTool);
