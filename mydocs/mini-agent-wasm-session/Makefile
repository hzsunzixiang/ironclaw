# ═══════════════════════════════════════════════════════════════════
# Mini Agent Loop + WASM Sandbox + Session — Makefile
#
# Project structure:
#   guest/  → Calculator tool compiled to WASM (runs inside sandbox)
#   host/   → Agent loop + WASM runtime + Session management
#   wit/    → WIT interface definition (contract between guest & host)
# ═══════════════════════════════════════════════════════════════════

GUEST_DIR     := guest
HOST_DIR      := host
GUEST_WASM    := $(GUEST_DIR)/target/wasm32-wasip2/release/guest_tool.wasm
HOST_BIN      := $(HOST_DIR)/target/release/mini-agent-wasm-session

GREEN  := \033[0;32m
CYAN   := \033[0;36m
YELLOW := \033[0;33m
RESET  := \033[0m

.PHONY: all build build-guest build-host run run-debug debug clean help check-deps

all: build

build: build-guest build-host
	@echo "$(GREEN)✅ Build complete!$(RESET)"
	@echo "   Guest WASM: $(GUEST_WASM)"
	@echo "   Host binary: $(HOST_BIN)"

build-guest:
	@echo "$(CYAN)━━━ Building Guest (WASM) ━━━$(RESET)"
	cd $(GUEST_DIR) && cargo build --target wasm32-wasip2 --release
	@echo "$(GREEN)✅ Guest WASM built$(RESET)"
	@ls -lh $(GUEST_WASM) | awk '{print "   Size: " $$5}'

build-host:
	@echo "$(CYAN)━━━ Building Host (Native) ━━━$(RESET)"
	cd $(HOST_DIR) && cargo build --release
	@echo "$(GREEN)✅ Host binary built$(RESET)"

run: build
	@echo ""
	@echo "$(YELLOW)═══ Running Mini Agent + WASM Sandbox + Session ═══$(RESET)"
	cd $(HOST_DIR) && cargo run --release -- ../$(GUEST_WASM)

run-debug: debug
	@echo ""
	@echo "$(YELLOW)═══ Running (debug mode) ═══$(RESET)"
	cd $(HOST_DIR) && cargo run -- ../$(GUEST_DIR)/target/wasm32-wasip2/debug/guest_tool.wasm

debug:
	@echo "$(CYAN)━━━ Building Guest (WASM, debug) ━━━$(RESET)"
	cd $(GUEST_DIR) && cargo build --target wasm32-wasip2
	@echo "$(CYAN)━━━ Building Host (Native, debug) ━━━$(RESET)"
	cd $(HOST_DIR) && cargo build

clean:
	cd $(GUEST_DIR) && cargo clean
	cd $(HOST_DIR) && cargo clean
	@echo "$(GREEN)✅ All clean!$(RESET)"

check-deps:
	@echo "$(CYAN)━━━ Checking Dependencies ━━━$(RESET)"
	@echo -n "  rustc:          " && rustc --version || echo "❌ NOT FOUND"
	@echo -n "  cargo:          " && cargo --version || echo "❌ NOT FOUND"
	@echo -n "  wasm32-wasip2:  " && (rustup target list --installed | grep -q wasm32-wasip2 && echo "✅ installed" || echo "❌ NOT FOUND — run: rustup target add wasm32-wasip2")

help:
	@echo "$(CYAN)Mini Agent + WASM Sandbox + Session — Available targets:$(RESET)"
	@echo ""
	@echo "  $(GREEN)build$(RESET)        Build guest (WASM) + host (native)"
	@echo "  $(GREEN)build-guest$(RESET)  Build guest WASM only"
	@echo "  $(GREEN)build-host$(RESET)   Build host binary only"
	@echo "  $(GREEN)run$(RESET)          Build and run (release mode)"
	@echo "  $(GREEN)run-debug$(RESET)    Build and run (debug mode)"
	@echo "  $(GREEN)clean$(RESET)        Remove all build artifacts"
	@echo "  $(GREEN)check-deps$(RESET)   Check required tools"
	@echo ""
	@echo "$(YELLOW)Quick start:$(RESET)"
	@echo "  make check-deps   # Verify tools"
	@echo "  make run           # Build everything and run"
	@echo ""
	@echo "$(YELLOW)Session commands (in-app):$(RESET)"
	@echo "  /new              Create a new conversation thread"
	@echo "  /threads          List all threads"
	@echo "  /switch <n>       Switch to thread #n"
	@echo "  /history          Show turn history for current thread"
	@echo "  /session          Show session info"
