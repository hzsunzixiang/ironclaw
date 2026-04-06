# ============================================================
# Mini Agent Loop + OAuth 2.0 — Makefile
# ============================================================
# Minimal agentic loop demo with OAuth 2.0 support.
# Demonstrates: stdin → LLM → Tool Execute → Response → stdout
#             + OAuth 2.0 Authorization Code with PKCE
# ============================================================

CARGO   := cargo
BINARY  := mini-agent-loop
CONFIG  := HAI_WOA.json
TEMPLATE:= HAI_WOA.template.json

# Default target
.DEFAULT_GOAL := help

# ---- Build ----

.PHONY: build
build: ## Build the project (debug mode)
	$(CARGO) build

.PHONY: release
release: ## Build the project (release mode, optimized)
	$(CARGO) build --release

# ---- Run ----

.PHONY: run
run: ## Run the agent loop (interactive mode)
	$(CARGO) run

.PHONY: run-release
run-release: ## Run the agent loop (release build)
	$(CARGO) run --release

.PHONY: demo
demo: ## Run a quick demo: ask "What is 42 + 58?"
	@echo "=== Quick Demo ==="
	@echo -e "What is 42 + 58?\nquit" | $(CARGO) run 2>&1

# ---- Test & Check ----

.PHONY: check
check: ## Run cargo check (fast compilation check)
	$(CARGO) check

.PHONY: test
test: ## Run all tests
	$(CARGO) test

.PHONY: clippy
clippy: ## Run clippy linter
	$(CARGO) clippy -- -D warnings

.PHONY: fmt
fmt: ## Format code with rustfmt
	$(CARGO) fmt

.PHONY: fmt-check
fmt-check: ## Check code formatting (CI-friendly)
	$(CARGO) fmt -- --check

# ---- Config ----

.PHONY: config
config: $(CONFIG) ## Show current config file
	@echo "=== Current Config ($(CONFIG)) ==="
	@cat $(CONFIG)

.PHONY: config-init
config-init: ## Create config from template (won't overwrite existing)
	@if [ -f $(CONFIG) ]; then \
		echo "⚠️  $(CONFIG) already exists. Remove it first if you want to re-init."; \
	else \
		cp $(TEMPLATE) $(CONFIG); \
		echo "✅ Created $(CONFIG) from template."; \
		echo "👉 Edit $(CONFIG) and fill in your OPENAI_API_KEY."; \
	fi

# ---- OAuth ----

.PHONY: download-rfcs
download-rfcs: ## Download OAuth-related RFCs
	@bash scripts/download-oauth-rfcs.sh

# ---- Clean ----

.PHONY: clean
clean: ## Clean build artifacts
	$(CARGO) clean

# ---- Tree ----

.PHONY: tree
tree: ## Show project source tree
	@echo "=== Project Structure ==="
	@find src -type f -name '*.rs' | sort | sed 's|^|  |'
	@echo ""
	@echo "  Cargo.toml"
	@echo "  Makefile"
	@echo "  $(TEMPLATE)"

# ---- Help ----

.PHONY: help
help: ## Show this help message
	@echo ""
	@echo "Mini Agent Loop + OAuth 2.0 — Available Commands"
	@echo "================================================="
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | \
		awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-15s\033[0m %s\n", $$1, $$2}'
	@echo ""
	@echo "Quick Start:"
	@echo "  1. make config-init    # Create config from template"
	@echo "  2. Edit HAI_WOA.json   # Fill in your API key"
	@echo "  3. make run            # Start interactive agent"
	@echo ""
	@echo "OAuth Quick Start:"
	@echo "  1. make download-rfcs  # Download OAuth RFCs for reference"
	@echo "  2. make run            # Start agent"
	@echo "  3. /auth github <id>   # Authenticate with GitHub"
	@echo "  4. /tokens             # View stored tokens"
	@echo ""
