#!/usr/bin/env bash
#
# run-trace.sh — Run mini-agent-wasm-session with RUST_LOG=trace and save logs
#
# Usage:
#   ./run-trace.sh                          # Use default question
#   ./run-trace.sh "What is 1+1?"           # Custom question
#   ./run-trace.sh "question" my-log.log    # Custom question + custom log file
#

set -euo pipefail

# Defaults
DEFAULT_QUESTION='What is 42 + 58*99 +(88*12/3)?'
DEFAULT_LOG_FILE="trace-output-raw.log"

QUESTION="${1:-$DEFAULT_QUESTION}"
LOG_FILE="${2:-$DEFAULT_LOG_FILE}"

# cd to script directory (where Makefile lives)
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

GUEST_WASM="guest/target/wasm32-wasip2/release/guest_tool.wasm"

echo "╔════════════════════════════════════════════════════════╗"
echo "║   Mini Agent + WASM + Session — Trace Runner           ║"
echo "╠════════════════════════════════════════════════════════╣"
echo "║  Question : $QUESTION"
echo "║  Log file : $SCRIPT_DIR/$LOG_FILE"
echo "║  Log level: trace (our code only, third-party=warn)"
echo "╚════════════════════════════════════════════════════════╝"
echo ""

# Step 1: Build guest WASM if needed
if [ ! -f "$GUEST_WASM" ]; then
    echo "⏳ Building guest WASM tool..."
    cd guest && cargo build --target wasm32-wasip2 --release && cd ..
    echo "✅ Guest WASM built"
fi

# Step 2: Run with trace logging
echo "⏳ Running with trace logging..."

# Only trace OUR code (mini_agent_wasm_session); suppress noisy third-party crates
# wasmtime internals (type_registry, cranelift, etc.) produce thousands of
# unreadable TRACE lines — keep them at warn level.
RUST_LOG_FILTER="mini_agent_wasm_session=trace,wasmtime=warn,cranelift=warn,regalloc=warn,wasi=warn"

printf '%s\nquit\n' "$QUESTION" | \
  RUST_LOG="$RUST_LOG_FILTER" cargo run --manifest-path host/Cargo.toml -- "$GUEST_WASM" >"$LOG_FILE" 2>&1

LINE_COUNT=$(wc -l < "$LOG_FILE")
FILE_SIZE=$(du -h "$LOG_FILE" | cut -f1)

echo ""
echo "✅ Done! Logs saved to: $SCRIPT_DIR/$LOG_FILE"
echo "   Lines: $LINE_COUNT"
echo "   Size : $FILE_SIZE"
echo ""
echo "📖 View logs:"
echo "   cat $LOG_FILE"
echo "   less $LOG_FILE"
echo ""
echo "🔍 Filter examples:"
echo "   grep 'TRACE' $LOG_FILE   # Maximum detail"
echo "   grep 'DEBUG' $LOG_FILE   # Detailed info"
echo "   grep 'INFO'  $LOG_FILE   # Key events only"
echo "   grep 'WASM'  $LOG_FILE   # WASM sandbox events"
echo "   grep 'Turn'  $LOG_FILE   # Turn lifecycle events"
echo "   grep 'Thread' $LOG_FILE  # Thread management events"
echo "   grep 'Session' $LOG_FILE # Session events"
echo "   grep 'request_body\|response_body' $LOG_FILE  # HTTP request/response bodies"
