# ═══════════════════════════════════════════════════════════════════
# WASM Sandbox Demo — Makefile
#
# Project structure:
#   guest/  → Rust code compiled to WASM (runs inside sandbox)
#   host/   → Rust code compiled to native (the sandbox runtime)
#   wit/    → WIT interface definition (the contract between guest & host)
#
# Build flow:
#   1. guest → compile to wasm32-wasip2 → produces .wasm file
#   2. host  → compile to native binary  → loads and runs the .wasm
# ═══════════════════════════════════════════════════════════════════

# Paths
GUEST_DIR     := guest
HOST_DIR      := host
GUEST_WASM    := $(GUEST_DIR)/target/wasm32-wasip2/release/guest_tool.wasm
HOST_BIN      := $(HOST_DIR)/target/release/wasm-host

# Colors for pretty output
GREEN  := \033[0;32m
CYAN   := \033[0;36m
YELLOW := \033[0;33m
RESET  := \033[0m

# ─── Default target ──────────────────────────────────────────────
.PHONY: all
all: build  ## Build everything (guest + host)

# ─── Build targets ───────────────────────────────────────────────
.PHONY: build build-guest build-host

build: build-guest build-host  ## Build both guest (WASM) and host (native)
	@echo "$(GREEN)✅ Build complete!$(RESET)"
	@echo "   Guest WASM: $(GUEST_WASM)"
	@echo "   Host binary: $(HOST_BIN)"

build-guest:  ## Compile guest Rust code → WASM component (wasm32-wasip2)
	@echo "$(CYAN)━━━ Building Guest (WASM) ━━━$(RESET)"
	@echo "  Target: wasm32-wasip2"
	@echo "  This compiles Rust into WASM bytecode that runs inside the sandbox."
	@echo ""
	cd $(GUEST_DIR) && cargo build --target wasm32-wasip2 --release
	@echo ""
	@echo "$(GREEN)✅ Guest WASM built: $(GUEST_WASM)$(RESET)"
	@ls -lh $(GUEST_WASM) | awk '{print "   Size: " $$5}'

build-host:  ## Compile host Rust code → native binary (the sandbox runtime)
	@echo "$(CYAN)━━━ Building Host (Native) ━━━$(RESET)"
	@echo "  Target: native ($(shell rustc -vV | grep host | awk '{print $$2}'))"
	@echo "  This is the Wasmtime-based runtime that loads and executes the WASM."
	@echo ""
	cd $(HOST_DIR) && cargo build --release
	@echo ""
	@echo "$(GREEN)✅ Host binary built: $(HOST_BIN)$(RESET)"

# ─── Run target ──────────────────────────────────────────────────
.PHONY: run

run: build  ## Build everything and run the demo
	@echo ""
	@echo "$(YELLOW)═══ Running WASM Sandbox Demo ═══$(RESET)"
	@echo "  Host loads guest WASM → creates sandbox → calls guest functions"
	@echo ""
	cd $(HOST_DIR) && cargo run --release -- ../$(GUEST_WASM)

# ─── Debug build (faster compile, with debug info) ───────────────
.PHONY: debug debug-guest debug-host run-debug

debug: debug-guest debug-host  ## Build both in debug mode (faster compile)

debug-guest:  ## Build guest WASM in debug mode
	@echo "$(CYAN)━━━ Building Guest (WASM, debug) ━━━$(RESET)"
	cd $(GUEST_DIR) && cargo build --target wasm32-wasip2

debug-host:  ## Build host in debug mode
	@echo "$(CYAN)━━━ Building Host (Native, debug) ━━━$(RESET)"
	cd $(HOST_DIR) && cargo build

run-debug: debug  ## Build in debug mode and run
	@echo ""
	@echo "$(YELLOW)═══ Running WASM Sandbox Demo (debug) ═══$(RESET)"
	cd $(HOST_DIR) && cargo run -- ../$(GUEST_DIR)/target/wasm32-wasip2/debug/guest_tool.wasm

# ─── Inspect targets ─────────────────────────────────────────────
.PHONY: inspect size

inspect:  ## Show WASM component info (requires wasm-tools)
	@echo "$(CYAN)━━━ WASM Component Info ━━━$(RESET)"
	@if command -v wasm-tools >/dev/null 2>&1; then \
		wasm-tools component wit $(GUEST_WASM); \
	else \
		echo "⚠️  wasm-tools not installed. Install with: cargo install wasm-tools"; \
	fi

size: build-guest  ## Show WASM binary size breakdown
	@echo "$(CYAN)━━━ WASM Binary Size ━━━$(RESET)"
	@ls -lh $(GUEST_WASM)
	@echo ""
	@echo "For detailed size analysis, install twiggy: cargo install twiggy"
	@echo "  twiggy top $(GUEST_WASM)"

# ─── Clean targets ───────────────────────────────────────────────
.PHONY: clean clean-guest clean-host

clean: clean-guest clean-host  ## Remove all build artifacts
	@echo "$(GREEN)✅ All clean!$(RESET)"

clean-guest:  ## Remove guest build artifacts
	@echo "Cleaning guest..."
	cd $(GUEST_DIR) && cargo clean

clean-host:  ## Remove host build artifacts
	@echo "Cleaning host..."
	cd $(HOST_DIR) && cargo clean

# ─── Prerequisites check ─────────────────────────────────────────
.PHONY: check-deps

check-deps:  ## Check that all required tools are installed
	@echo "$(CYAN)━━━ Checking Dependencies ━━━$(RESET)"
	@echo -n "  rustc:          " && rustc --version || echo "❌ NOT FOUND"
	@echo -n "  cargo:          " && cargo --version || echo "❌ NOT FOUND"
	@echo -n "  wasm32-wasip2:  " && (rustup target list --installed | grep -q wasm32-wasip2 && echo "✅ installed" || echo "❌ NOT FOUND — run: rustup target add wasm32-wasip2")
	@echo -n "  wasm-tools:     " && (wasm-tools --version 2>/dev/null || echo "⚠️  optional, install with: cargo install wasm-tools")
	@echo ""

# ─── Help ─────────────────────────────────────────────────────────
.PHONY: help

help:  ## Show this help message
	@echo "$(CYAN)WASM Sandbox Demo — Available targets:$(RESET)"
	@echo ""
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | \
		awk 'BEGIN {FS = ":.*?## "}; {printf "  $(GREEN)%-16s$(RESET) %s\n", $$1, $$2}'
	@echo ""
	@echo "$(YELLOW)Typical workflow:$(RESET)"
	@echo "  make check-deps   # Verify tools are installed"
	@echo "  make build        # Compile guest (WASM) + host (native)"
	@echo "  make run          # Build and run the demo"
	@echo "  make inspect      # Inspect the WASM component"
	@echo "  make clean        # Remove all build artifacts"
