//! WASM Host Runtime — loads and executes WASM tools in a sandbox.
//!
//! This is the "other side" of the WASM sandbox. It:
//! 1. Creates a Wasmtime engine (the WASM virtual machine)
//! 2. Loads the guest .wasm component
//! 3. Implements host functions that the guest can call
//! 4. Calls the guest's exported functions
//!
//! The key insight: the guest runs in a completely isolated sandbox.
//! It can ONLY do what the host explicitly allows through the WIT interface.

use std::time::{SystemTime, UNIX_EPOCH};

use wasmtime::component::Linker;
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiView};

// ═══════════════════════════════════════════════════════════════════
// Step 1: Generate host-side bindings from the SAME WIT file
//
// This macro reads the WIT and generates:
//   - `demo::sandbox::host::Host` trait (we must implement)
//   - `demo::sandbox::host::add_to_linker()` function
//   - `SandboxedTool` struct with `instantiate()` to create guest instances
//   - `exports::demo::sandbox::tool::*` types for calling guest functions
//
// The guest uses `wit_bindgen::generate!` (guest-side bindings)
// The host uses `wasmtime::component::bindgen!` (host-side bindings)
// Both read the SAME WIT file — that's the contract!
// ═══════════════════════════════════════════════════════════════════
wasmtime::component::bindgen!({
    path: "../wit/tool.wit",
    world: "sandboxed-tool",
    async: false,
    with: {},
});

// ═══════════════════════════════════════════════════════════════════
// Step 2: Define the Store data (host state)
//
// The Store holds all per-execution state. Each WASM execution gets
// a fresh Store — this is the "fresh instance per execution" pattern
// from NEAR blockchain, ensuring complete isolation between runs.
// ═══════════════════════════════════════════════════════════════════
struct StoreData {
    wasi: WasiCtx,
    table: ResourceTable,
    /// Collected log messages from the guest
    logs: Vec<(String, String)>, // (level, message)
}

// WasiView is required by wasmtime-wasi for WASI support
impl WasiView for StoreData {
    fn ctx(&mut self) -> &mut WasiCtx {
        &mut self.wasi
    }
    fn table(&mut self) -> &mut ResourceTable {
        &mut self.table
    }
}

// ═══════════════════════════════════════════════════════════════════
// Step 3: Implement the Host trait
//
// THIS IS THE CORE OF THE SANDBOX.
//
// When the guest calls `host::log(...)` or `host::now_millis()`,
// execution crosses the WASM boundary and arrives HERE.
//
// The host has FULL CONTROL over what these functions do:
// - log() could write to a file, send to a server, or just collect in memory
// - now_millis() could return the real time, or a fake time for testing
// - In IronClaw, http_request() checks an allowlist before making the request
//
// The guest has NO WAY to bypass this — it's enforced by the WASM VM.
// ═══════════════════════════════════════════════════════════════════
impl demo::sandbox::host::Host for StoreData {
    fn log(&mut self, level: demo::sandbox::host::LogLevel, message: String) {
        let level_str = match level {
            demo::sandbox::host::LogLevel::Info => "INFO",
            demo::sandbox::host::LogLevel::Warn => "WARN",
            demo::sandbox::host::LogLevel::Error => "ERROR",
        };
        // We CHOOSE to print it. We could also silently drop it,
        // or send it to a logging service, or rate-limit it.
        println!("  📋 [GUEST LOG] [{level_str}] {message}");
        self.logs.push((level_str.to_string(), message));
    }

    fn now_millis(&mut self) -> u64 {
        // We CHOOSE to return the real time. We could return 0,
        // or a fixed timestamp for deterministic testing.
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        println!("  ⏰ [HOST] Guest requested current time → {now}");
        now
    }
}

// ═══════════════════════════════════════════════════════════════════
// Step 4: Main — put it all together
// ═══════════════════════════════════════════════════════════════════
fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("╔══════════════════════════════════════════════════════════╗");
    println!("║         WASM Sandbox Demo — Understanding WASM          ║");
    println!("╚══════════════════════════════════════════════════════════╝");
    println!();

    // --- Engine: the WASM virtual machine ---
    let mut config = Config::new();
    config.wasm_component_model(true); // Enable Component Model
    config.consume_fuel(true);          // Enable fuel metering (CPU limit)
    let engine = Engine::new(&config)?;

    println!("✅ Wasmtime engine created (with fuel metering enabled)");

    // --- Load the guest WASM component ---
    let wasm_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "../guest/target/wasm32-wasip2/release/guest_tool.wasm".to_string());

    println!("📦 Loading WASM component from: {wasm_path}");
    let wasm_bytes = std::fs::read(&wasm_path)?;
    println!("   Size: {} bytes ({:.1} KB)", wasm_bytes.len(), wasm_bytes.len() as f64 / 1024.0);

    // Compile the WASM bytes into a Component
    // This validates the WASM and compiles it to native code
    let component = wasmtime::component::Component::new(&engine, &wasm_bytes)?;
    println!("✅ WASM component compiled successfully");

    // --- Set up the Linker ---
    // The Linker connects host functions to the WASM guest.
    // It's like a "phone book" — the guest looks up "host::log" and
    // the linker tells it where to find the host's implementation.
    let mut linker: Linker<StoreData> = Linker::new(&engine);

    // Register WASI functions (required by wasm32-wasip2 target)
    wasmtime_wasi::add_to_linker_sync(&mut linker)?;

    // Register OUR host functions (log, now_millis)
    // This uses the generated `add_to_linker` from the bindgen! macro
    demo::sandbox::host::add_to_linker(&mut linker, |state| state)?;

    println!("✅ Host functions registered (log, now_millis)");

    // --- Create a Store (per-execution state) ---
    let mut store = Store::new(
        &engine,
        StoreData {
            wasi: WasiCtxBuilder::new().build(),
            table: ResourceTable::new(),
            logs: Vec::new(),
        },
    );

    // Set fuel limit — the guest can only use this many "fuel units"
    // Each WASM instruction costs some fuel. If it runs out, execution traps.
    // This prevents infinite loops and CPU exhaustion.
    store.set_fuel(1_000_000)?; // 1 million fuel units
    println!("⛽ Fuel limit set: 1,000,000 units");

    // --- Instantiate the guest ---
    let tool = SandboxedTool::instantiate(&mut store, &component, &linker)?;
    println!("✅ Guest tool instantiated in sandbox");
    println!();

    // ═══════════════════════════════════════════════════════════════
    // Now let's call the guest's exported functions!
    // ═══════════════════════════════════════════════════════════════

    // --- Call description() ---
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("📝 Calling guest.description():");
    let desc = tool.demo_sandbox_tool().call_description(&mut store)?;
    println!("   → {desc}");
    println!();

    // --- Call schema() ---
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("📐 Calling guest.schema():");
    let schema: serde_json::Value = serde_json::from_str(&tool.demo_sandbox_tool().call_schema(&mut store)?)?;
    println!("   → {}", serde_json::to_string_pretty(&schema)?);
    println!();

    // --- Call execute() with different inputs ---
    let test_cases = vec![
        r#"{"operation": "add", "a": 42, "b": 58}"#,
        r#"{"operation": "mul", "a": 3.14, "b": 2}"#,
        r#"{"operation": "div", "a": 100, "b": 3}"#,
        r#"{"operation": "div", "a": 1, "b": 0}"#,
        r#"{"operation": "sqrt", "a": 9, "b": 0}"#,
    ];

    for (i, params) in test_cases.iter().enumerate() {
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        println!("🔧 Test {}: execute({})", i + 1, params);

        let request = exports::demo::sandbox::tool::Request {
            params: params.to_string(),
        };

        let response = tool.demo_sandbox_tool().call_execute(&mut store, &request)?;

        if let Some(output) = &response.output {
            let parsed: serde_json::Value = serde_json::from_str(output)?;
            println!("   ✅ Output: {}", serde_json::to_string_pretty(&parsed)?);
        }
        if let Some(error) = &response.error {
            println!("   ❌ Error: {error}");
        }
        println!();
    }

    // --- Show fuel consumption ---
    let remaining_fuel = store.get_fuel()?;
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("⛽ Fuel remaining: {} / 1,000,000", remaining_fuel);
    println!("   Consumed: {} units", 1_000_000 - remaining_fuel);
    println!();

    // --- Show collected logs ---
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("📋 Total guest log entries collected by host: {}", store.data().logs.len());
    println!();

    println!("╔══════════════════════════════════════════════════════════╗");
    println!("║                    Demo Complete! 🎉                     ║");
    println!("║                                                          ║");
    println!("║  The guest ran in a WASM sandbox. It could ONLY:         ║");
    println!("║  • Call host::log() to emit messages                     ║");
    println!("║  • Call host::now_millis() to get the time               ║");
    println!("║  • Do pure computation (math, string ops)                ║");
    println!("║                                                          ║");
    println!("║  It could NOT:                                           ║");
    println!("║  • Access the filesystem                                 ║");
    println!("║  • Make network requests                                 ║");
    println!("║  • Read environment variables                            ║");
    println!("║  • Access host memory                                    ║");
    println!("║  • Run forever (fuel limit enforced)                     ║");
    println!("╚══════════════════════════════════════════════════════════╝");

    Ok(())
}
