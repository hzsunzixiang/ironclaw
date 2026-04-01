#!/usr/bin/env bash
#
# run-trace.sh — Run mini-agent-memory with RUST_LOG=trace and demonstrate
#                long-term memory persistence across sessions.
#
# This script runs TWO sessions:
#   Session 1: Tell the agent personal info & preferences → memories saved
#   Session 2: Restart → ask the agent to recall → proves memory persists
#
# Usage:
#   ./run-trace.sh                    # Full demo (both sessions)
#   ./run-trace.sh session1           # Only session 1 (store memories)
#   ./run-trace.sh session2           # Only session 2 (recall memories)
#   ./run-trace.sh clean              # Clean workspace and run full demo
#

set -euo pipefail

# ── Configuration ──
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

GUEST_WASM="guest/target/wasm32-wasip2/release/guest_tool.wasm"
HOST_MANIFEST="host/Cargo.toml"
# Note: cargo run --manifest-path runs with cwd = SCRIPT_DIR (project root)
# So workspace/ is created under project root, and HAI_WOA.json is found there too.
WORKSPACE_DIR="workspace"

LOG_SESSION1="trace-session1.log"
LOG_SESSION2="trace-session2.log"
LOG_COMBINED="trace-output-raw.log"

# Only trace OUR code; suppress noisy third-party crates
RUST_LOG_FILTER="mini_agent_memory=trace,wasmtime=warn,cranelift=warn,regalloc=warn,wasi=warn"

MODE="${1:-full}"

# ── Helper functions ──

print_header() {
    echo ""
    echo "╔══════════════════════════════════════════════════════════════╗"
    echo "║  Mini Agent + WASM + Session + Memory — Trace Runner        ║"
    echo "╠══════════════════════════════════════════════════════════════╣"
    echo "║  Mode     : $MODE"
    echo "║  Workspace: $SCRIPT_DIR/$WORKSPACE_DIR"
    echo "║  Log level: trace (our code only, third-party=warn)"
    echo "╚══════════════════════════════════════════════════════════════╝"
    echo ""
}

build_if_needed() {
    if [ ! -f "$GUEST_WASM" ]; then
        echo "⏳ Building guest WASM tool..."
        cd guest && cargo build --target wasm32-wasip2 --release && cd ..
        echo "✅ Guest WASM built"
    else
        echo "✅ Guest WASM already built: $GUEST_WASM"
    fi

    echo "⏳ Building host (release)..."
    cd host && cargo build --release && cd ..
    echo "✅ Host binary built"
    echo ""
}

run_session() {
    local session_name="$1"
    local log_file="$2"
    local input_data="$3"

    echo "═══════════════════════════════════════════════════════════"
    echo "  🚀 Running $session_name"
    echo "  📄 Log: $SCRIPT_DIR/$log_file"
    echo "═══════════════════════════════════════════════════════════"
    echo ""
    echo "  Input commands:"
    echo "$input_data" | sed 's/^/    │ /'
    echo ""

    printf '%s' "$input_data" | \
        RUST_LOG="$RUST_LOG_FILTER" cargo run --manifest-path "$HOST_MANIFEST" --release \
        -- "$GUEST_WASM" > "$log_file" 2>&1

    local line_count
    local file_size
    line_count=$(wc -l < "$log_file")
    file_size=$(du -h "$log_file" | cut -f1)

    echo "  ✅ $session_name completed"
    echo "     Lines: $line_count"
    echo "     Size : $file_size"
    echo ""
}

show_memory_file() {
    local memory_file="$WORKSPACE_DIR/MEMORY.md"
    if [ -f "$memory_file" ]; then
        echo "═══════════════════════════════════════════════════════════"
        echo "  📝 MEMORY.md content after session:"
        echo "═══════════════════════════════════════════════════════════"
        cat "$memory_file" | sed 's/^/    │ /'
        echo ""
    else
        echo "  ⚠️  MEMORY.md not found at $memory_file"
    fi
}

show_daily_logs() {
    local daily_dir="$WORKSPACE_DIR/daily"
    if [ -d "$daily_dir" ]; then
        echo "═══════════════════════════════════════════════════════════"
        echo "  📅 Daily logs:"
        echo "═══════════════════════════════════════════════════════════"
        for f in "$daily_dir"/*.md; do
            if [ -f "$f" ]; then
                echo "    ── $(basename "$f") ──"
                cat "$f" | sed 's/^/    │ /'
                echo ""
            fi
        done
    fi
}

print_summary() {
    echo ""
    echo "╔══════════════════════════════════════════════════════════════╗"
    echo "║  ✅ All sessions completed!                                 ║"
    echo "╠══════════════════════════════════════════════════════════════╣"
    echo "║                                                            ║"
    echo "║  Log files:                                                ║"
    if [ -f "$LOG_SESSION1" ]; then
    echo "║    Session 1: $LOG_SESSION1"
    fi
    if [ -f "$LOG_SESSION2" ]; then
    echo "║    Session 2: $LOG_SESSION2"
    fi
    if [ -f "$LOG_COMBINED" ]; then
    echo "║    Combined : $LOG_COMBINED"
    fi
    echo "║                                                            ║"
    echo "║  📖 View logs:                                             ║"
    echo "║    cat $LOG_COMBINED                                       ║"
    echo "║    less $LOG_COMBINED                                      ║"
    echo "║                                                            ║"
    echo "║  🔍 Filter examples:                                       ║"
    echo "║    grep 'TRACE' $LOG_COMBINED                              ║"
    echo "║    grep 'INFO'  $LOG_COMBINED                              ║"
    echo "║    grep 'memory' $LOG_COMBINED    # Memory operations      ║"
    echo "║    grep 'Auto-saved' $LOG_COMBINED # Auto-extracted mem    ║"
    echo "║    grep 'MEMORY.md' $LOG_COMBINED  # Memory injection      ║"
    echo "║    grep 'memory_write' $LOG_COMBINED # LLM memory writes   ║"
    echo "║    grep 'memory_search' $LOG_COMBINED # LLM memory search  ║"
    echo "║    grep 'Turn' $LOG_COMBINED       # Turn lifecycle        ║"
    echo "║    grep 'Thread' $LOG_COMBINED     # Thread management     ║"
    echo "║    grep 'request_body\\|response_body' $LOG_COMBINED        ║"
    echo "║                                                            ║"
    echo "╚══════════════════════════════════════════════════════════════╝"
}

# ══════════════════════════════════════════════════════════════════════
# Session 1 Input: Store memories
#
# This session:
#   1. Tells the agent personal info (name, job, location)
#   2. States preferences (favorite color, language)
#   3. Asks a math question (tests WASM calculator + memory coexistence)
#   4. Explicitly asks to remember a project detail
#   5. Quits → triggers session end + daily log
# ══════════════════════════════════════════════════════════════════════

SESSION1_INPUT="My name is Erick and I work at IronClaw Labs. I live in Shanghai.
Remember that our project deadline is April 15th 2026 and the project name is Phoenix.
I prefer using Rust for backend development and TypeScript for frontend.
My favorite color is blue.
What is 42 + 58 * 99 + (88 * 12 / 3)?
Please remember that the database we use is PostgreSQL and our deployment target is CentOS.
quit
"

# ══════════════════════════════════════════════════════════════════════
# Session 2 Input: Recall memories (proves persistence!)
#
# This session starts FRESH (new process, new Session object), but
# MEMORY.md persists on disk → injected into system prompt → LLM knows!
#
# We ask questions that can ONLY be answered if memory persists:
#   1. "What's my name?" → should recall "Erick"
#   2. "What project am I working on?" → should recall "Phoenix"
#   3. "What languages do I prefer?" → should recall "Rust + TypeScript"
#   4. "What's our project deadline?" → should recall "April 15th 2026"
#   5. Math question to verify tools still work
#   6. Check memory commands
# ══════════════════════════════════════════════════════════════════════

SESSION2_INPUT="What is my name and where do I work?
What project am I working on and when is the deadline?
What programming languages do I prefer?
What is 100 * 50 + 25?
/memory
quit
"

# ══════════════════════════════════════════════════════════════════════
# Main execution
# ══════════════════════════════════════════════════════════════════════

print_header

case "$MODE" in
    clean)
        echo "🧹 Cleaning workspace..."
        rm -rf "$WORKSPACE_DIR"
        rm -f "$LOG_SESSION1" "$LOG_SESSION2" "$LOG_COMBINED"
        echo "✅ Workspace cleaned"
        echo ""
        build_if_needed

        run_session "Session 1 — Store Memories" "$LOG_SESSION1" "$SESSION1_INPUT"
        show_memory_file
        show_daily_logs

        echo ""
        echo "⏳ Simulating restart (2 second pause)..."
        sleep 2
        echo ""

        run_session "Session 2 — Recall Memories (after restart)" "$LOG_SESSION2" "$SESSION2_INPUT"
        show_memory_file
        show_daily_logs

        # Combine logs
        echo "--- SESSION 1 ---" > "$LOG_COMBINED"
        cat "$LOG_SESSION1" >> "$LOG_COMBINED"
        echo "" >> "$LOG_COMBINED"
        echo "--- SESSION 2 (after restart) ---" >> "$LOG_COMBINED"
        cat "$LOG_SESSION2" >> "$LOG_COMBINED"

        print_summary
        ;;

    session1)
        build_if_needed
        run_session "Session 1 — Store Memories" "$LOG_SESSION1" "$SESSION1_INPUT"
        show_memory_file
        show_daily_logs
        ;;

    session2)
        build_if_needed
        run_session "Session 2 — Recall Memories" "$LOG_SESSION2" "$SESSION2_INPUT"
        show_memory_file
        show_daily_logs
        ;;

    full)
        build_if_needed

        # Session 1: Store memories
        run_session "Session 1 — Store Memories" "$LOG_SESSION1" "$SESSION1_INPUT"
        show_memory_file
        show_daily_logs

        echo ""
        echo "⏳ Simulating restart (2 second pause)..."
        sleep 2
        echo ""

        # Session 2: Recall memories (proves persistence!)
        run_session "Session 2 — Recall Memories (after restart)" "$LOG_SESSION2" "$SESSION2_INPUT"
        show_memory_file
        show_daily_logs

        # Combine logs
        echo "--- SESSION 1 ---" > "$LOG_COMBINED"
        cat "$LOG_SESSION1" >> "$LOG_COMBINED"
        echo "" >> "$LOG_COMBINED"
        echo "--- SESSION 2 (after restart) ---" >> "$LOG_COMBINED"
        cat "$LOG_SESSION2" >> "$LOG_COMBINED"

        print_summary
        ;;

    *)
        echo "Usage: $0 [full|session1|session2|clean]"
        echo ""
        echo "  full      Run both sessions (default)"
        echo "  session1  Only run session 1 (store memories)"
        echo "  session2  Only run session 2 (recall memories)"
        echo "  clean     Clean workspace + run full demo"
        exit 1
        ;;
esac
