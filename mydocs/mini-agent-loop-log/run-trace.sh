#!/usr/bin/env bash
#
# run-trace.sh — Run mini-agent-loop with RUST_LOG=trace and save logs
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

# cd to script directory (where Cargo.toml lives)
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

echo "╔════════════════════════════════════════════════╗"
echo "║        Mini Agent Loop — Trace Runner          ║"
echo "╠════════════════════════════════════════════════╣"
echo "║  Question : $QUESTION"
echo "║  Log file : $SCRIPT_DIR/$LOG_FILE"
echo "║  Log level: trace (maximum detail)"
echo "╚════════════════════════════════════════════════╝"
echo ""
echo "⏳ Running..."

# Run with trace logging, capture all output (stdout + stderr) into log file
printf '%s\nquit\n' "$QUESTION" | \
  RUST_LOG=trace cargo run >"$LOG_FILE" 2>&1

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
echo "   grep 'request_body\|response_body' $LOG_FILE  # HTTP request/response bodies"
