#!/usr/bin/env bash
#
# run-session-test.sh — Multi-turn Session test with trace logging
#
# This script feeds a sequence of user inputs (multi-turn conversation + session
# commands) through stdin pipe, collects stdout and trace logs separately, then
# produces a combined analysis-friendly log.
#
# Usage:
#   ./run-session-test.sh                    # Run default test scenario
#   ./run-session-test.sh my-scenario.txt    # Run custom scenario from file
#   ./run-session-test.sh --list             # List built-in scenarios
#   ./run-session-test.sh --scenario <name>  # Run a named built-in scenario
#
# How it works:
#   stdin.read_line() in Rust is blocking — it reads one line at a time from
#   the pipe buffer. Even though all lines are available immediately, the
#   program processes them sequentially (each line triggers a full LLM round-trip
#   before the next read_line()). So piping multi-line input works correctly.
#
# Output:
#   session-test.log — Single merged log (report header + stdout + trace interleaved)
#

set -euo pipefail

# ── cd to script directory ──
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

GUEST_WASM="guest/target/wasm32-wasip2/release/guest_tool.wasm"

# ── Log file name ──
LOG_FILE="session-test.log"

# ── RUST_LOG filter: trace our code, suppress noisy third-party crates ──
RUST_LOG_FILTER="mini_agent_wasm_session=trace,wasmtime=warn,cranelift=warn,regalloc=warn,wasi=warn"

# ═══════════════════════════════════════════════════════════════════
# Built-in test scenarios
# ═══════════════════════════════════════════════════════════════════

# Scenario: basic — Multi-turn memory test
scenario_basic() {
cat << 'SCENARIO'
What is 42 + 58?
Now multiply that result by 3
What was my first question and what was the answer?
/history
/session
quit
SCENARIO
}

# Scenario: threads — Multi-thread isolation test
scenario_threads() {
cat << 'SCENARIO'
What is 100 + 200?
Now divide that by 3
/new
What is 7 * 8?
/threads
/switch 1
What was the last result we calculated?
/history
/session
quit
SCENARIO
}

# Scenario: full — Comprehensive session test (memory + threads + commands)
scenario_full() {
cat << 'SCENARIO'
What is 42 + 58?
Now multiply that result by 3
What was my first question?
/history
/new
What is 7 * 8?
Now add 100 to that
/threads
/switch 1
Do you remember what we calculated? What was the final answer?
/history
/switch 2
/history
/session
quit
SCENARIO
}

# Scenario: stress — Many turns in one thread
scenario_stress() {
cat << 'SCENARIO'
What is 1 + 1?
Add 10 to that
Multiply the result by 5
Subtract 20
Divide by 2
What is the final result? Walk me through all the steps.
/history
/session
quit
SCENARIO
}

# ═══════════════════════════════════════════════════════════════════
# Helper functions
# ═══════════════════════════════════════════════════════════════════

list_scenarios() {
    echo "Built-in test scenarios:"
    echo ""
    echo "  basic    — Multi-turn memory test (3 questions + history)"
    echo "  threads  — Multi-thread isolation test (2 threads, switch between them)"
    echo "  full     — Comprehensive test (memory + threads + all commands)"
    echo "  stress   — Many sequential turns in one thread (chain of calculations)"
    echo ""
    echo "Usage:"
    echo "  ./run-session-test.sh --scenario basic"
    echo "  ./run-session-test.sh --scenario full"
    echo "  ./run-session-test.sh my-custom-inputs.txt"
}

get_scenario_input() {
    local name="$1"
    case "$name" in
        basic)   scenario_basic ;;
        threads) scenario_threads ;;
        full)    scenario_full ;;
        stress)  scenario_stress ;;
        *)
            echo "❌ Unknown scenario: $name" >&2
            echo "   Run with --list to see available scenarios." >&2
            exit 1
            ;;
    esac
}

# ═══════════════════════════════════════════════════════════════════
# Parse arguments
# ═══════════════════════════════════════════════════════════════════

INPUT_SOURCE=""
SCENARIO_NAME="basic"  # default

if [ $# -eq 0 ]; then
    # No args: use default scenario
    INPUT_SOURCE="scenario"
elif [ "$1" = "--list" ]; then
    list_scenarios
    exit 0
elif [ "$1" = "--scenario" ]; then
    if [ $# -lt 2 ]; then
        echo "❌ --scenario requires a name. Run with --list to see options." >&2
        exit 1
    fi
    SCENARIO_NAME="$2"
    INPUT_SOURCE="scenario"
else
    # Treat as a file path
    if [ ! -f "$1" ]; then
        echo "❌ File not found: $1" >&2
        exit 1
    fi
    INPUT_SOURCE="file"
    INPUT_FILE="$1"
fi

# ═══════════════════════════════════════════════════════════════════
# Build if needed
# ═══════════════════════════════════════════════════════════════════

if [ ! -f "$GUEST_WASM" ]; then
    echo "⏳ Building guest WASM tool..."
    cd guest && cargo build --target wasm32-wasip2 --release && cd ..
    echo "✅ Guest WASM built"
fi

# ═══════════════════════════════════════════════════════════════════
# Prepare input
# ═══════════════════════════════════════════════════════════════════

if [ "$INPUT_SOURCE" = "scenario" ]; then
    INPUT_CONTENT=$(get_scenario_input "$SCENARIO_NAME")
    DISPLAY_NAME="scenario: $SCENARIO_NAME"
else
    INPUT_CONTENT=$(cat "$INPUT_FILE")
    DISPLAY_NAME="file: $INPUT_FILE"
fi

# Count input lines (= number of turns/commands)
INPUT_LINE_COUNT=$(echo "$INPUT_CONTENT" | wc -l | tr -d ' ')

# ═══════════════════════════════════════════════════════════════════
# Display banner
# ═══════════════════════════════════════════════════════════════════

echo "╔════════════════════════════════════════════════════════════╗"
echo "║   Mini Agent + WASM + Session — Multi-Turn Test Runner    ║"
echo "╠════════════════════════════════════════════════════════════╣"
echo "║  Source   : $DISPLAY_NAME"
echo "║  Inputs   : $INPUT_LINE_COUNT lines"
echo "║  Log file : $SCRIPT_DIR/$LOG_FILE"
echo "║  Log level: trace (our code only, third-party=warn)"
echo "╚════════════════════════════════════════════════════════════╝"
echo ""

# Show the input that will be sent
echo "📋 Input sequence:"
echo "$INPUT_CONTENT" | nl -ba -w3 -s'  ' | sed 's/^/   /'
echo ""

# ═══════════════════════════════════════════════════════════════════
# Run the agent with piped input
# ═══════════════════════════════════════════════════════════════════

echo "⏳ Running multi-turn session test..."
echo ""

# Write report header, then run agent with stdout+stderr merged into one file
{
    echo "═══════════════════════════════════════════════════════════════"
    echo "  Session Test Report"
    echo "  Generated: $(date '+%Y-%m-%d %H:%M:%S')"
    echo "  Source: $DISPLAY_NAME"
    echo "═══════════════════════════════════════════════════════════════"
    echo ""
    echo "━━━ INPUT SEQUENCE ━━━"
    echo "$INPUT_CONTENT" | nl -ba -w3 -s'  '
    echo ""
    echo "━━━ EXECUTION LOG (stdout + trace interleaved) ━━━"
    echo ""
} > "$LOG_FILE"

# Run: stdout and stderr both append to the same log file
# This gives a natural chronological interleaving of agent output and trace logs
echo "$INPUT_CONTENT" | \
    RUST_LOG="$RUST_LOG_FILTER" \
    cargo run --manifest-path host/Cargo.toml -- "$GUEST_WASM" \
    >>"$LOG_FILE" 2>&1

EXIT_CODE=$?

# Append analysis summary to the log
{
    echo ""
    echo "━━━ ANALYSIS SUMMARY ━━━"
    echo ""
    echo "Exit code: $EXIT_CODE"
    echo ""
    echo "Turn count (user inputs processed):"
    grep -c 'User input received' "$LOG_FILE" 2>/dev/null || echo "  (no turns found)"
    echo ""
    echo "Tool calls executed:"
    grep -c 'Executing tool' "$LOG_FILE" 2>/dev/null || echo "  (no tool calls found)"
    echo ""
    echo "Threads created:"
    grep -c 'Creating new thread' "$LOG_FILE" 2>/dev/null || echo "  (count not available)"
    echo ""
    echo "Errors:"
    grep -i 'error\|failed\|❌' "$LOG_FILE" 2>/dev/null || echo "  (none)"
    echo ""
    echo "Session commands executed:"
    grep 'User requested' "$LOG_FILE" 2>/dev/null || echo "  (none)"
} >> "$LOG_FILE"

# ═══════════════════════════════════════════════════════════════════
# Summary
# ═══════════════════════════════════════════════════════════════════

LOG_LINES=$(wc -l < "$LOG_FILE" | tr -d ' ')
LOG_SIZE=$(du -h "$LOG_FILE" | cut -f1)

echo ""
echo "✅ Done! (exit code: $EXIT_CODE)"
echo ""
echo "📁 Output:"
echo "   $LOG_FILE — $LOG_LINES lines, $LOG_SIZE"
echo ""
echo "📖 Quick view:"
echo "   cat $LOG_FILE                              # Full report"
echo "   less $LOG_FILE                             # Paged view"
echo ""
echo "🔍 Analysis commands:"
echo "   grep 'INFO'  $LOG_FILE                     # Key events"
echo "   grep 'TRACE' $LOG_FILE                     # Maximum detail"
echo "   grep 'Turn'  $LOG_FILE                     # Turn lifecycle"
echo "   grep 'Thread' $LOG_FILE                    # Thread operations"
echo "   grep 'Session' $LOG_FILE                   # Session events"
echo "   grep 'request_body' $LOG_FILE              # LLM requests"
echo "   grep 'response_body' $LOG_FILE             # LLM responses"
echo "   grep 'tool_calls\|Executing' $LOG_FILE     # Tool call chain"
echo "   grep '🤖' $LOG_FILE                        # Agent responses only"
echo ""
echo "🧪 Run other scenarios:"
echo "   ./run-session-test.sh --list              # See all scenarios"
echo "   ./run-session-test.sh --scenario threads   # Test thread isolation"
echo "   ./run-session-test.sh --scenario full      # Comprehensive test"
echo "   ./run-session-test.sh my-inputs.txt        # Custom input file"
