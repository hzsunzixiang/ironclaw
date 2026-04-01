
# ═══════════════════════════════════════════════════════════════════
# Mini Agent Loop + WASM Sandbox + Session + Memory — Makefile
#
# Project structure:
#   guest/     → Calculator tool compiled to WASM (runs inside sandbox)
#   host/      → Agent loop + WASM runtime + Session + Memory management
#   wit/       → WIT interface definition (contract between guest & host)
#   workspace/ → Persistent memory files (created at runtime)
# ═══════════════════════════════════════════════════════════════════

GUEST_DIR     := guest
HOST_DIR      := host
GUEST_WASM    := $(GUEST_DIR)/target/wasm32-wasip2/release/guest_tool.wasm
HOST_BIN      := $(HOST_DIR)/target/release/mini-agent-memory

GREEN  := \033[0;32m
CYAN   := \033[0;36m
YELLOW := \033[0;33m
RESET  := \033[0m

.PHONY: all build build-guest build-host run run-debug run-trace debug clean help check-deps test

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
	@echo "$(YELLOW)═══ Running Mini Agent + WASM Sandbox + Session + Memory ═══$(RESET)"
	cd $(HOST_DIR) && cargo run --release -- ../$(GUEST_WASM)

run-debug: debug
	@echo ""
	@echo "$(YELLOW)═══ Running (debug mode) ═══$(RESET)"
	cd $(HOST_DIR) && cargo run -- ../$(GUEST_DIR)/target/wasm32-wasip2/debug/guest_tool.wasm

run-trace: build
	@echo ""
	@echo "$(YELLOW)═══ Running with RUST_LOG=trace (full tracing) ═══$(RESET)"
	cd $(HOST_DIR) && RUST_LOG=mini_agent_memory=trace cargo run --release -- ../$(GUEST_WASM)

debug:
	@echo "$(CYAN)━━━ Building Guest (WASM, debug) ━━━$(RESET)"
	cd $(GUEST_DIR) && cargo build --target wasm32-wasip2
	@echo "$(CYAN)━━━ Building Host (Native, debug) ━━━$(RESET)"
	cd $(HOST_DIR) && cargo build

test:
	@echo "$(CYAN)━━━ Running Tests ━━━$(RESET)"
	cd $(HOST_DIR) && cargo test
	@echo "$(GREEN)✅ All tests passed!$(RESET)"

clean:
	cd $(GUEST_DIR) && cargo clean
	cd $(HOST_DIR) && cargo clean
	@echo "$(GREEN)✅ All clean!$(RESET)"

clean-workspace:
	rm -rf workspace/
	@echo "$(GREEN)✅ Workspace cleaned (memory reset)!$(RESET)"

check-deps:
	@echo "$(CYAN)━━━ Checking Dependencies ━━━$(RESET)"
	@echo -n "  rustc:          " && rustc --version || echo "❌ NOT FOUND"
	@echo -n "  cargo:          " && cargo --version || echo "❌ NOT FOUND"
	@echo -n "  wasm32-wasip2:  " && (rustup target list --installed | grep -q wasm32-wasip2 && echo "✅ installed" || echo "❌ NOT FOUND — run: rustup target add wasm32-wasip2")

help:
	@echo "$(CYAN)Mini Agent + WASM Sandbox + Session + Memory — Available targets:$(RESET)"
	@echo ""
	@echo "  $(GREEN)build$(RESET)            Build guest (WASM) + host (native)"
	@echo "  $(GREEN)build-guest$(RESET)      Build guest WASM only"
	@echo "  $(GREEN)build-host$(RESET)       Build host binary only"
	@echo "  $(GREEN)run$(RESET)              Build and run (release mode)"
	@echo "  $(GREEN)run-debug$(RESET)        Build and run (debug mode)"
	@echo "  $(GREEN)run-trace$(RESET)        Build and run with RUST_LOG=trace"
	@echo "  $(GREEN)test$(RESET)             Run unit tests"
	@echo "  $(GREEN)clean$(RESET)            Remove all build artifacts"
	@echo "  $(GREEN)clean-workspace$(RESET)  Delete workspace/ (reset all memory)"
	@echo "  $(GREEN)check-deps$(RESET)       Check required tools"
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
	@echo ""
	@echo "$(YELLOW)Memory commands (in-app):$(RESET)"
	@echo "  /memory           Show MEMORY.md content"
	@echo "  /memory-search <q> Search workspace memory"
	@echo "  /memory-tree      Show workspace file tree"
