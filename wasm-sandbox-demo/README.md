# WASM Sandbox Demo

A minimal, self-contained project demonstrating how WASM sandboxing works,
inspired by IronClaw's architecture.

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│                    Host (Rust binary)                     │
│                                                           │
│  1. Compile WIT → wasmtime bindgen types                 │
│  2. Load .wasm component                                 │
│  3. Implement host functions (log, now_millis)           │
│  4. Create Store + Linker + Instance                     │
│  5. Call guest's execute() / schema() / description()    │
│                                                           │
│  ┌─────────────────────────────────────────────────────┐ │
│  │              WASM Sandbox (wasmtime)                 │ │
│  │                                                      │ │
│  │  Guest (compiled to .wasm):                         │ │
│  │  - Can ONLY call host-provided functions             │ │
│  │  - Cannot access filesystem                          │ │
│  │  - Cannot access network                             │ │
│  │  - Cannot access memory outside its sandbox          │ │
│  │  - Fuel-metered (CPU limit)                          │ │
│  │  - Memory-limited                                    │ │
│  │                                                      │ │
│  │  Exports: execute(), schema(), description()         │ │
│  │  Imports: host::log(), host::now_millis()            │ │
│  └─────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────┘
```

## Project Structure

```
wasm-sandbox-demo/
├── wit/
│   └── tool.wit              # WIT interface definition (the contract)
├── guest/                    # WASM guest tool (compiles to .wasm)
│   ├── Cargo.toml
│   └── src/lib.rs
├── host/                     # Host runtime (loads and runs .wasm)
│   ├── Cargo.toml
│   └── src/main.rs
└── README.md
```

## How to Build & Run

### Prerequisites

```bash
rustup target add wasm32-wasip2
```

### Step 1: Build the guest (WASM tool)

```bash
cd guest
cargo build --target wasm32-wasip2 --release
```

### Step 2: Run the host

```bash
cd host
cargo run --release
```

## Key Concepts

| Concept | Where |
|---------|-------|
| WIT interface (contract) | `wit/tool.wit` |
| Guest uses `wit_bindgen` | `guest/src/lib.rs` |
| Host uses `wasmtime::component::bindgen!` | `host/src/main.rs` |
| Host implements imported functions | `impl Host for StoreData` |
| Guest exports functions for host to call | `impl Guest for CalculatorTool` |
| Fuel metering (CPU limit) | Host sets fuel on Store |
