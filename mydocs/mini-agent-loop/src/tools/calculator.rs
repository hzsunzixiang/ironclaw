
//! # Calculator Tool — Example Builtin Tool
//!
//! In IronClaw, tools can be:
//!   - Builtin (Rust native): shell, http, file_read, file_write, memory, etc.
//!   - WASM (sandboxed): third-party tools running in Wasmtime sandbox
//!   - MCP (external): tools accessed via Model Context Protocol
//!
//! The Calculator here is a simple builtin tool for demonstration.

use async_trait::async_trait;

use super::{Tool, ToolOutput};

pub struct CalculatorTool;

#[async_trait]
impl Tool for CalculatorTool {
    fn name(&self) -> &str { "calculator" }

    fn description(&self) -> &str {
        "Perform basic arithmetic operations. Supports: add, sub, mul, div."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["add", "sub", "mul", "div"],
                    "description": "The arithmetic operation to perform"
                },
                "a": { "type": "number", "description": "First operand" },
                "b": { "type": "number", "description": "Second operand" }
            },
            "required": ["operation", "a", "b"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<ToolOutput, String> {
        let op = params["operation"].as_str().ok_or("Missing 'operation'")?;
        let a = params["a"].as_f64().ok_or("Missing 'a'")?;
        let b = params["b"].as_f64().ok_or("Missing 'b'")?;

        let (result, expression) = match op {
            "add" => (a + b, format!("{} + {} = {}", a, b, a + b)),
            "sub" => (a - b, format!("{} - {} = {}", a, b, a - b)),
            "mul" => (a * b, format!("{} × {} = {}", a, b, a * b)),
            "div" => {
                if b == 0.0 {
                    return Err("Division by zero".to_string());
                }
                (a / b, format!("{} ÷ {} = {}", a, b, a / b))
            }
            _ => return Err(format!("Unknown operation: '{}'", op)),
        };

        Ok(ToolOutput {
            result: serde_json::json!({
                "result": result,
                "expression": expression,
            }),
        })
    }
}
