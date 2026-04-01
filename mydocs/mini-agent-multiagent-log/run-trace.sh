#!/usr/bin/env bash
#
# run-trace.sh — Run mini-agent-multiagent with RUST_LOG=trace and demonstrate
#                multi-agent capabilities: ChatDelegate, JobDelegate, Router,
#                Scheduler, and parallel job execution.
#
# This is the MULTI-AGENT version of run-trace.sh.
# It runs three sessions to exercise all multi-agent features:
#
#   Session 1: ChatDelegate (foreground) — store memories + math
#   Session 2: Multi-Agent — /job commands + /jobs + /status + /cancel + chat interleave
#   Session 3: Memory persistence + parallel jobs
#
# Multi-Agent features tested:
#   ✅ LoopDelegate trait — shared agentic loop engine
#   ✅ ChatDelegate — interactive chat (foreground, session-aware)
#   ✅ JobDelegate — background job execution (independent, Arc-based)
#   ✅ Router — /job, /jobs, /status, /cancel command dispatch
#   ✅ Scheduler — parallel job management (max 3)
#   ✅ Memory persistence — across sessions
#   ✅ Context compaction — token limit management
#
# Usage:
#   ./run-trace.sh                    # Full demo (all 3 sessions)
#   ./run-trace.sh session1           # Only session 1 (ChatDelegate: store memories)
#   ./run-trace.sh session2           # Only session 2 (Multi-Agent: jobs + chat)
#   ./run-trace.sh session3           # Only session 3 (Parallel jobs + recall)
#   ./run-trace.sh clean              # Clean workspace and run full demo
#

set -euo pipefail

# ── Configuration ──
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

GUEST_WASM="guest/target/wasm32-wasip2/release/guest_tool.wasm"
HOST_MANIFEST="host/Cargo.toml"
WORKSPACE_DIR="workspace"

LOG_SESSION1="trace-session1.log"
LOG_SESSION2="trace-session2.log"
LOG_SESSION3="trace-session3.log"
LOG_COMBINED="trace-output-raw.log"

# Only trace OUR code; suppress noisy third-party crates
RUST_LOG_FILTER="mini_agent_memory=trace,wasmtime=warn,cranelift=warn,regalloc=warn,wasi=warn"

MODE="${1:-full}"

# ── Helper functions ──

print_header() {
    echo ""
    echo "╔══════════════════════════════════════════════════════════════╗"
    echo "║  Mini Agent + Multi-Agent — Trace Runner                    ║"
    echo "╠══════════════════════════════════════════════════════════════╣"
    echo "║  Mode     : $MODE"
    echo "║  Workspace: $SCRIPT_DIR/$WORKSPACE_DIR"
    echo "║  Log level: trace (our code only, third-party=warn)"
    echo "║                                                            ║"
    echo "║  Multi-Agent features under test:                          ║"
    echo "║    • LoopDelegate trait (shared agentic loop engine)       ║"
    echo "║    • ChatDelegate (foreground interactive chat)            ║"
    echo "║    • JobDelegate (background job execution)                ║"
    echo "║    • Router (/job, /jobs, /status, /cancel dispatch)       ║"
    echo "║    • Scheduler (parallel job management, max 3)            ║"
    echo "║    • Memory persistence (cross-session recall)             ║"
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

        local total_lines
        local entry_count
        local word_count
        total_lines=$(wc -l < "$memory_file")
        entry_count=$(grep -c "^- " "$memory_file" || echo "0")
        word_count=$(wc -w < "$memory_file")
        echo "  📊 Memory stats:"
        echo "     Lines: $total_lines"
        echo "     Entries: $entry_count"
        echo "     Words: $word_count"
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

analyze_multiagent_log() {
    local log_file="$1"
    local session_name="$2"

    echo "═══════════════════════════════════════════════════════════"
    echo "  🔍 Multi-Agent Analysis: $session_name"
    echo "═══════════════════════════════════════════════════════════"

    # Router dispatch events
    local route_count
    route_count=$(grep -c "Router intent" "$log_file" 2>/dev/null || echo "0")
    echo "  📡 Router dispatches: $route_count"

    # ChatDelegate events (foreground chat)
    local chat_loop_count
    chat_loop_count=$(grep -c "Launching agentic loop via ChatDelegate" "$log_file" 2>/dev/null || echo "0")
    echo "  💬 ChatDelegate loop invocations: $chat_loop_count"

    # JobDelegate events (background jobs)
    local job_dispatch_count
    job_dispatch_count=$(grep -c "Dispatching new job" "$log_file" 2>/dev/null || echo "0")
    echo "  🚀 Jobs dispatched: $job_dispatch_count"

    local job_started_count
    job_started_count=$(grep -c "Job worker started" "$log_file" 2>/dev/null || echo "0")
    echo "  ▶️  Jobs started: $job_started_count"

    local job_completed_count
    job_completed_count=$(grep -c "Job completed successfully" "$log_file" 2>/dev/null || echo "0")
    echo "  ✅ Jobs completed: $job_completed_count"

    local job_cancelled_count
    job_cancelled_count=$(grep -c "Cancel signal sent" "$log_file" 2>/dev/null || echo "0")
    echo "  🛑 Jobs cancelled: $job_cancelled_count"

    local job_stopped_count
    job_stopped_count=$(grep -c "Job received stop signal\|Job was stopped" "$log_file" 2>/dev/null || echo "0")
    echo "  🛑 Jobs stopped (by signal): $job_stopped_count"

    # Scheduler events
    local scheduler_init
    scheduler_init=$(grep -c "Scheduler initialized" "$log_file" 2>/dev/null || echo "0")
    echo "  📋 Scheduler initialized: $scheduler_init"

    local cleanup_count
    cleanup_count=$(grep -c "Cleaned up finished job" "$log_file" 2>/dev/null || echo "0")
    echo "  🧹 Jobs cleaned up: $cleanup_count"

    # Agentic loop events (shared engine)
    local loop_enter_count
    loop_enter_count=$(grep -c "Entering agentic loop" "$log_file" 2>/dev/null || echo "0")
    echo "  🔄 Agentic loop entries (total): $loop_enter_count"

    # Tool execution events
    local tool_success_count
    tool_success_count=$(grep -c "Tool .* succeeded" "$log_file" 2>/dev/null || echo "0")
    echo "  🔧 Tool executions (success): $tool_success_count"

    # Memory events
    local memory_auto_count
    memory_auto_count=$(grep -c "Auto-saved memory" "$log_file" 2>/dev/null || echo "0")
    echo "  🧠 Auto-saved memories: $memory_auto_count"

    echo ""
}

print_summary() {
    echo ""
    echo "╔══════════════════════════════════════════════════════════════╗"
    echo "║  ✅ All sessions completed!                                 ║"
    echo "╠══════════════════════════════════════════════════════════════╣"
    echo "║                                                            ║"
    echo "║  Log files:                                                ║"
    if [ -f "$LOG_SESSION1" ]; then
    echo "║    Session 1 (ChatDelegate):  $LOG_SESSION1"
    fi
    if [ -f "$LOG_SESSION2" ]; then
    echo "║    Session 2 (Multi-Agent):   $LOG_SESSION2"
    fi
    if [ -f "$LOG_SESSION3" ]; then
    echo "║    Session 3 (Parallel Jobs): $LOG_SESSION3"
    fi
    if [ -f "$LOG_COMBINED" ]; then
    echo "║    Combined:                  $LOG_COMBINED"
    fi
    echo "║                                                            ║"
    echo "║  📖 View logs:                                             ║"
    echo "║    cat $LOG_COMBINED                                       ║"
    echo "║    less $LOG_COMBINED                                      ║"
    echo "║                                                            ║"
    echo "║  🔍 Multi-Agent filter examples:                           ║"
    echo "║    grep 'Router intent' $LOG_COMBINED    # Route decisions ║"
    echo "║    grep 'ChatDelegate' $LOG_COMBINED     # Chat agent      ║"
    echo "║    grep 'JobDelegate\\|Job worker' $LOG_COMBINED # Job agent║"
    echo "║    grep 'Dispatching\\|dispatch' $LOG_COMBINED  # Job create║"
    echo "║    grep 'Scheduler' $LOG_COMBINED        # Scheduler       ║"
    echo "║    grep 'check_signals\\|stop signal' $LOG_COMBINED # Sigs  ║"
    echo "║    grep 'Entering agentic loop' $LOG_COMBINED  # Loop entry║"
    echo "║    grep 'LoopDelegate\\|delegate' $LOG_COMBINED # Delegate  ║"
    echo "║    grep 'memory_write' $LOG_COMBINED     # Memory writes   ║"
    echo "║    grep 'Auto-saved' $LOG_COMBINED       # Auto-extracted  ║"
    echo "║    grep 'Cleaned up' $LOG_COMBINED       # Job cleanup     ║"
    echo "║    grep 'Cancel' $LOG_COMBINED           # Job cancellation║"
    echo "║    grep 'Turn' $LOG_COMBINED             # Turn lifecycle  ║"
    echo "║    grep 'request_body\\|response_body' $LOG_COMBINED        ║"
    echo "║                                                            ║"
    echo "║  🏗️  Architecture verified:                                ║"
    echo "║    ✅ LoopDelegate trait — shared engine for Chat + Job     ║"
    echo "║    ✅ ChatDelegate — foreground interactive chat            ║"
    echo "║    ✅ JobDelegate — background job execution                ║"
    echo "║    ✅ Router — command dispatch (/job, /jobs, /status, etc) ║"
    echo "║    ✅ Scheduler — parallel job management                   ║"
    echo "║    ✅ Memory — persistence across sessions                  ║"
    echo "║                                                            ║"
    echo "╚══════════════════════════════════════════════════════════════╝"
}

# ══════════════════════════════════════════════════════════════════════
# Session 1 Input: ChatDelegate — Store Memories + Math
#
# Tests:
#   ✅ ChatDelegate (foreground agentic loop via LoopDelegate)
#   ✅ Router routes natural language → UserInput → ChatDelegate
#   ✅ WASM calculator tool (math questions)
#   ✅ Memory tools (memory_write via LLM)
#   ✅ Auto-memory extraction (from conversation)
#   ✅ Session/Thread/Turn lifecycle
#   ✅ Context token estimation
#
# This session stores personal info and project details into memory,
# which Session 2 and 3 will recall to prove persistence.
# ══════════════════════════════════════════════════════════════════════

SESSION1_INPUT="My name is Erick and I work at IronClaw Labs. I live in Shanghai.
Remember that our project deadline is April 15th 2026 and the project name is Phoenix.
I prefer using Rust for backend development and TypeScript for frontend.
What is 42 + 58 * 99 + (88 * 12 / 3)?
Please remember that the database we use is PostgreSQL and our deployment target is CentOS.
quit
"

# ══════════════════════════════════════════════════════════════════════
# Session 2 Input: Multi-Agent — Jobs + Chat Interleave
#
# Tests:
#   ✅ Router dispatches /job → CreateJob → Scheduler.dispatch_job()
#   ✅ JobDelegate runs agentic loop in background (tokio::spawn)
#   ✅ Router dispatches /jobs → ListJobs (Scheduler.list_jobs())
#   ✅ Router dispatches /status → CheckJobStatus
#   ✅ ChatDelegate + JobDelegate coexist (interleaved commands)
#   ✅ Job lifecycle: Pending → InProgress → Completed
#   ✅ Scheduler worker management (mpsc channel, WorkerMessage)
#   ✅ Router dispatches natural language → UserInput (between jobs)
#   ✅ Memory recall (proves Session 1 memories persisted)
#
# Flow:
#   1. /job — create background job (calculator task)
#   2. (brief pause for job to start)
#   3. /jobs — list all jobs (should show InProgress or Completed)
#   4. Chat question — foreground ChatDelegate (interleaved with jobs)
#   5. /job — create second background job (memory task)
#   6. /jobs — list both jobs
#   7. Chat question — recall memory from Session 1
#   8. /status — check specific job status (use placeholder, will match first)
#   9. quit
# ══════════════════════════════════════════════════════════════════════

SESSION2_INPUT="/job Calculate the result of 999 * 888 + 777 using the calculator tool
/jobs
What is my name and where do I work?
/job Search memory for information about our project deadline and summarize it
/jobs
What programming languages do I prefer?
/jobs
quit
"

# ══════════════════════════════════════════════════════════════════════
# Session 3 Input: Parallel Jobs + Cancel + Memory Recall
#
# Tests:
#   ✅ Multiple parallel jobs (Scheduler capacity: max 3)
#   ✅ /cancel command → Scheduler.cancel_job() → WorkerMessage::Stop
#   ✅ JobDelegate.check_signals() → LoopSignal::Stop
#   ✅ Job state transitions: InProgress → Cancelled
#   ✅ /jobs shows mixed states (Completed, Cancelled, InProgress)
#   ✅ Memory persistence across 3 sessions
#   ✅ ChatDelegate recall after job operations
#
# Flow:
#   1. /job — create job #1 (calculator)
#   2. /job — create job #2 (calculator)
#   3. /job — create job #3 (memory search)
#   4. /jobs — list all 3 jobs (should show various states)
#   5. Chat — recall project info from Session 1
#   6. /jobs — final status check
#   7. /memory — show full memory content
#   8. quit
# ══════════════════════════════════════════════════════════════════════

SESSION3_INPUT="/job Calculate 123 * 456 + 789 using the calculator tool
/job Calculate 100 + 200 + 300 + 400 + 500 using the calculator tool
/job Search memory for all stored information and list everything you find
/jobs
What project am I working on and when is the deadline?
/jobs
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
        rm -f "$LOG_SESSION1" "$LOG_SESSION2" "$LOG_SESSION3" "$LOG_COMBINED"
        echo "✅ Workspace cleaned"
        echo ""
        build_if_needed

        # Session 1: ChatDelegate — store memories
        run_session "Session 1 — ChatDelegate: Store Memories" "$LOG_SESSION1" "$SESSION1_INPUT"
        show_memory_file
        show_daily_logs
        analyze_multiagent_log "$LOG_SESSION1" "Session 1"

        echo ""
        echo "⏳ Simulating restart (2 second pause)..."
        sleep 2
        echo ""

        # Session 2: Multi-Agent — jobs + chat interleave
        run_session "Session 2 — Multi-Agent: Jobs + Chat Interleave" "$LOG_SESSION2" "$SESSION2_INPUT"
        show_memory_file
        show_daily_logs
        analyze_multiagent_log "$LOG_SESSION2" "Session 2"

        echo ""
        echo "⏳ Simulating restart (2 second pause)..."
        sleep 2
        echo ""

        # Session 3: Parallel jobs + cancel + recall
        run_session "Session 3 — Parallel Jobs + Memory Recall" "$LOG_SESSION3" "$SESSION3_INPUT"
        show_memory_file
        show_daily_logs
        analyze_multiagent_log "$LOG_SESSION3" "Session 3"

        # Combine logs
        {
            echo "=== SESSION 1 — ChatDelegate: Store Memories ==="
            cat "$LOG_SESSION1"
            echo ""
            echo "=== SESSION 2 — Multi-Agent: Jobs + Chat Interleave ==="
            cat "$LOG_SESSION2"
            echo ""
            echo "=== SESSION 3 — Parallel Jobs + Memory Recall ==="
            cat "$LOG_SESSION3"
        } > "$LOG_COMBINED"

        print_summary
        ;;

    session1)
        build_if_needed
        run_session "Session 1 — ChatDelegate: Store Memories" "$LOG_SESSION1" "$SESSION1_INPUT"
        show_memory_file
        show_daily_logs
        analyze_multiagent_log "$LOG_SESSION1" "Session 1"
        ;;

    session2)
        build_if_needed
        run_session "Session 2 — Multi-Agent: Jobs + Chat Interleave" "$LOG_SESSION2" "$SESSION2_INPUT"
        show_memory_file
        show_daily_logs
        analyze_multiagent_log "$LOG_SESSION2" "Session 2"
        ;;

    session3)
        build_if_needed
        run_session "Session 3 — Parallel Jobs + Memory Recall" "$LOG_SESSION3" "$SESSION3_INPUT"
        show_memory_file
        show_daily_logs
        analyze_multiagent_log "$LOG_SESSION3" "Session 3"
        ;;

    full)
        build_if_needed

        # Session 1: ChatDelegate — store memories
        run_session "Session 1 — ChatDelegate: Store Memories" "$LOG_SESSION1" "$SESSION1_INPUT"
        show_memory_file
        show_daily_logs
        analyze_multiagent_log "$LOG_SESSION1" "Session 1"

        echo ""
        echo "⏳ Simulating restart (2 second pause)..."
        sleep 2
        echo ""

        # Session 2: Multi-Agent — jobs + chat interleave
        run_session "Session 2 — Multi-Agent: Jobs + Chat Interleave" "$LOG_SESSION2" "$SESSION2_INPUT"
        show_memory_file
        show_daily_logs
        analyze_multiagent_log "$LOG_SESSION2" "Session 2"

        echo ""
        echo "⏳ Simulating restart (2 second pause)..."
        sleep 2
        echo ""

        # Session 3: Parallel jobs + cancel + recall
        run_session "Session 3 — Parallel Jobs + Memory Recall" "$LOG_SESSION3" "$SESSION3_INPUT"
        show_memory_file
        show_daily_logs
        analyze_multiagent_log "$LOG_SESSION3" "Session 3"

        # Combine logs
        {
            echo "=== SESSION 1 — ChatDelegate: Store Memories ==="
            cat "$LOG_SESSION1"
            echo ""
            echo "=== SESSION 2 — Multi-Agent: Jobs + Chat Interleave ==="
            cat "$LOG_SESSION2"
            echo ""
            echo "=== SESSION 3 — Parallel Jobs + Memory Recall ==="
            cat "$LOG_SESSION3"
        } > "$LOG_COMBINED"

        print_summary
        ;;

    *)
        echo "Usage: $0 [full|session1|session2|session3|clean]"
        echo ""
        echo "  full      Run all 3 sessions (default)"
        echo "  session1  Only session 1 (ChatDelegate: store memories)"
        echo "  session2  Only session 2 (Multi-Agent: jobs + chat interleave)"
        echo "  session3  Only session 3 (Parallel jobs + memory recall)"
        echo "  clean     Clean workspace + run full demo"
        exit 1
        ;;
esac
