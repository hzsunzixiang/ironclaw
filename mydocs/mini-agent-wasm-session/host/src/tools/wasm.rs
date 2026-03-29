//! # WASM Sandbox Infrastructure
//!
//! Corresponds to: `src/wasm/sandbox.rs` in IronClaw.
//!
//! Contains the Wasmtime-based sandbox that executes guest .wasm tools
//! in complete isolation. The guest can ONLY call host-provided functions
//! (log, now_millis) — no filesystem, no network, no env vars.

use async_trait::async_trait;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use wasmtime::component::Linker;
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiView};

use super::{Tool, ToolOutput};

// ════════════════════════════════════════════════════════════════════
// Step 1: Generate host-side bindings from the WIT file.
// ════════════════════════════════════════════════════════════════════

wasmtime::component::bindgen!({
    path: "../wit/tool.wit",
    world: "sandboxed-tool",
    async: false,
    with: {},
});

// ════════════════════════════════════════════════════════════════════
// Step 2: Per-execution state.
// ════════════════════════════════════════════════════════════════════

struct StoreData {
    wasi: WasiCtx,
    table: ResourceTable,
    /// Collected log messages from the guest
    logs: Vec<(String, String)>,
}

impl WasiView for StoreData {
    fn ctx(&mut self) -> &mut WasiCtx {
        &mut self.wasi
    }
    fn table(&mut self) -> &mut ResourceTable {
        &mut self.table
    }
}

// ════════════════════════════════════════════════════════════════════
// Step 3: Implement the Host trait — THE CORE OF THE SANDBOX.
// ════════════════════════════════════════════════════════════════════

impl demo::sandbox::host::Host for StoreData {
    fn log(&mut self, level: demo::sandbox::host::LogLevel, message: String) {
        let level_str = match level {
            demo::sandbox::host::LogLevel::Info => "INFO",
            demo::sandbox::host::LogLevel::Warn => "WARN",
            demo::sandbox::host::LogLevel::Error => "ERROR",
        };
        println!("    📋 [WASM LOG] [{level_str}] {message}");
        self.logs.push((level_str.to_string(), message));
    }

    fn now_millis(&mut self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }
}

// ════════════════════════════════════════════════════════════════════
// Step 4: Shared WASM engine + compiled component.
// ════════════════════════════════════════════════════════════════════

pub struct WasmToolEngine {
    engine: Engine,
    component: wasmtime::component::Component,
    /// Tool metadata (cached from first instantiation)
    pub tool_name: String,
    pub tool_description: String,
    pub tool_schema: serde_json::Value,
    /// Fuel limit per execution
    pub fuel_limit: u64,
}

impl WasmToolEngine {
    /// Create a new WASM tool engine from a .wasm file.
    ///
    /// This:
    /// 1. Creates a Wasmtime engine with component model + fuel metering
    /// 2. Compiles the WASM bytes to native code
    /// 3. Instantiates once to read tool metadata (name, description, schema)
    pub fn new(wasm_path: &str, fuel_limit: u64) -> Result<Self, String> {
        // Create engine with component model and fuel metering
        let mut config = Config::new();
        config.wasm_component_model(true);
        config.consume_fuel(true);
        let engine =
            Engine::new(&config).map_err(|e| format!("Failed to create WASM engine: {}", e))?;

        // Load and compile the WASM component
        let wasm_bytes = std::fs::read(wasm_path)
            .map_err(|e| format!("Failed to read WASM file '{}': {}", wasm_path, e))?;
        let component = wasmtime::component::Component::new(&engine, &wasm_bytes)
            .map_err(|e| format!("Failed to compile WASM component: {}", e))?;

        // Instantiate once to read metadata
        let (mut store, linker) = Self::create_store_and_linker(&engine, fuel_limit)?;

        let instance = SandboxedTool::instantiate(&mut store, &component, &linker)
            .map_err(|e| format!("Failed to instantiate WASM tool: {}", e))?;

        // Read tool metadata from the guest itself (name comes from guest, not caller)
        let tool_name = instance
            .demo_sandbox_tool()
            .call_name(&mut store)
            .map_err(|e| format!("Failed to call name(): {}", e))?;
        let description = instance
            .demo_sandbox_tool()
            .call_description(&mut store)
            .map_err(|e| format!("Failed to call description(): {}", e))?;
        let schema_str = instance
            .demo_sandbox_tool()
            .call_schema(&mut store)
            .map_err(|e| format!("Failed to call schema(): {}", e))?;
        let schema: serde_json::Value = serde_json::from_str(&schema_str)
            .map_err(|e| format!("Failed to parse schema JSON: {}", e))?;

        Ok(Self {
            engine,
            component,
            tool_name,
            tool_description: description,
            tool_schema: schema,
            fuel_limit,
        })
    }

    /// Create a fresh Store + Linker pair for sandbox execution.
    ///
    /// Extracted to eliminate duplication between `new()` and `execute_in_sandbox()`.
    /// Each call produces a completely isolated execution environment.
    fn create_store_and_linker(
        engine: &Engine,
        fuel_limit: u64,
    ) -> Result<(Store<StoreData>, Linker<StoreData>), String> {
        let mut linker: Linker<StoreData> = Linker::new(engine);
        wasmtime_wasi::add_to_linker_sync(&mut linker)
            .map_err(|e| format!("Failed to add WASI to linker: {}", e))?;
        demo::sandbox::host::add_to_linker(&mut linker, |state| state)
            .map_err(|e| format!("Failed to add host functions to linker: {}", e))?;

        let mut store = Store::new(
            engine,
            StoreData {
                wasi: WasiCtxBuilder::new().build(),
                table: ResourceTable::new(),
                logs: Vec::new(),
            },
        );
        store
            .set_fuel(fuel_limit)
            .map_err(|e| format!("Failed to set fuel: {}", e))?;

        Ok((store, linker))
    }

    /// Execute the tool in a fresh sandbox.
    fn execute_in_sandbox(&self, params: &str) -> Result<String, String> {
        let (mut store, linker) =
            Self::create_store_and_linker(&self.engine, self.fuel_limit)?;

        // Instantiate the guest in the sandbox
        let instance = SandboxedTool::instantiate(&mut store, &self.component, &linker)
            .map_err(|e| format!("Instantiation error: {}", e))?;

        // Call the guest's execute() function
        let request = exports::demo::sandbox::tool::Request {
            params: params.to_string(),
        };
        let response = instance
            .demo_sandbox_tool()
            .call_execute(&mut store, &request)
            .map_err(|e| format!("WASM execution error: {}", e))?;

        // Report fuel consumption
        let remaining = store.get_fuel().unwrap_or(0);
        let consumed = self.fuel_limit - remaining;
        println!(
            "    ⛽ Fuel consumed: {} / {} units",
            consumed, self.fuel_limit
        );

        // Check response
        if let Some(error) = response.error {
            return Err(format!("Tool error: {}", error));
        }
        response
            .output
            .ok_or_else(|| "Tool returned no output".to_string())
    }
}

// ════════════════════════════════════════════════════════════════════
// Step 5: WasmTool — implements the Tool trait using the WASM sandbox.
// ════════════════════════════════════════════════════════════════════

pub struct WasmTool {
    pub engine: Arc<WasmToolEngine>,
}

#[async_trait]
impl Tool for WasmTool {
    fn name(&self) -> &str {
        &self.engine.tool_name
    }

    fn description(&self) -> &str {
        &self.engine.tool_description
    }

    fn parameters_schema(&self) -> serde_json::Value {
        self.engine.tool_schema.clone()
    }

    async fn execute(&self, params: serde_json::Value) -> Result<ToolOutput, String> {
        let params_str = serde_json::to_string(&params)
            .map_err(|e| format!("Failed to serialize params: {}", e))?;

        // Execute in WASM sandbox (synchronous — WASM execution is sync)
        let engine = self.engine.clone();
        let result = tokio::task::spawn_blocking(move || engine.execute_in_sandbox(&params_str))
            .await
            .map_err(|e| format!("Task join error: {}", e))??;

        // Parse the JSON output from the guest
        let value: serde_json::Value = serde_json::from_str(&result)
            .map_err(|e| format!("Failed to parse tool output: {}", e))?;

        Ok(ToolOutput { result: value })
    }
}
