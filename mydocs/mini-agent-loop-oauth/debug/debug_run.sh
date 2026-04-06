#!/bin/bash
# ============================================================================
# debug_run.sh — One-click debug launcher for mini-agent-loop
# ============================================================================
#
# Uses lldbinit for automated breakpoint tracing (pure LLDB, no Python).
#
# Usage:
#   ./debug_run.sh "What is 42 + 58?"     # Auto mode: run in background, tail log
#   ./debug_run.sh --bg "What is 42 + 58?" # Pure background, no foreground output
#   ./debug_run.sh --input <file>          # Batch with stdin from file
#   ./debug_run.sh --interactive           # Interactive lldb session
#   ./debug_run.sh --log                   # Interactive + logging
#   ./debug_run.sh --batch                 # Batch mode (auto-run, no stdin)
#
# Examples:
#   ./debug_run.sh "What is 42 + 58?"
#   ./debug_run.sh "Hello, who are you?"
#   ./debug_run.sh --bg "Calculate 100 * 3.14"
#   echo -e "What is 1+1?\nquit" | ./debug_run.sh --stdin
#
# ============================================================================

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
TARGET="$PROJECT_DIR/target/debug/mini-agent-loop"
LLDBINIT="$SCRIPT_DIR/lldbinit"
TIMESTAMP="$(date +%Y%m%d_%H%M%S)"
LOG_FILE="$SCRIPT_DIR/logs/debug_${TIMESTAMP}.log"
INPUT_FILE="$SCRIPT_DIR/.debug_input_${TIMESTAMP}.txt"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

# ── Build if needed ──
build_if_needed() {
    if [ ! -f "$TARGET" ]; then
        echo -e "${YELLOW}⚙️  Binary not found, building...${NC}"
        (cd "$PROJECT_DIR" && cargo build)
        echo -e "${GREEN}✅ Build complete${NC}"
    fi
}

# ── Ensure log directory exists ──
mkdir -p "$SCRIPT_DIR/logs"

# ── Find lldb ──
find_lldb() {
    if command -v rust-lldb &> /dev/null; then
        LLDB_CMD="rust-lldb"
    else
        LLDB_CMD="lldb"
    fi
}

# ── Create input file from a question string ──
create_input_file() {
    local question="$1"
    printf '%s\nquit\n' "$question" > "$INPUT_FILE"
}

# ── Cleanup temp files on exit ──
# Note: For background mode, cleanup is handled inside run_batch
#       to avoid deleting the input file before lldb reads it.
SKIP_CLEANUP=false
cleanup() {
    if [ "$SKIP_CLEANUP" = "false" ]; then
        rm -f "$INPUT_FILE" 2>/dev/null || true
    fi
}
trap cleanup EXIT

# ── Print banner ──
print_banner() {
    echo -e "${CYAN}╔══════════════════════════════════════════════════════════════╗${NC}"
    echo -e "${CYAN}║       mini-agent-loop — Debug Launcher (lldbinit)           ║${NC}"
    echo -e "${CYAN}╚══════════════════════════════════════════════════════════════╝${NC}"
    echo ""
}

# ── Run lldb in batch mode, output to log ──
run_batch() {
    local input="$1"
    local bg_mode="$2"

    # Run lldb in a subshell that cleans up the input file after lldb finishes
    (
        cd "$PROJECT_DIR" && $LLDB_CMD -b \
            -o "command source $LLDBINIT" \
            -o "process launch -i $input" \
            "$TARGET"
        # Cleanup temp input file after lldb finishes
        rm -f "$input" 2>/dev/null || true
    ) > "$LOG_FILE" 2>&1 &
    local PID=$!

    if [ "$bg_mode" = "true" ]; then
        # Skip cleanup in trap since the subshell handles it
        SKIP_CLEANUP=true
        echo -e "${GREEN}✅ Running in background (PID: $PID)${NC}"
        echo -e "${GREEN}📄 Log: $LOG_FILE${NC}"
        echo -e "${YELLOW}   View: tail -f $LOG_FILE${NC}"
        echo -e "${YELLOW}   Stop: kill $PID${NC}"
    else
        echo -e "${GREEN}✅ Running (PID: $PID), streaming output...${NC}"
        echo -e "${GREEN}📄 Log: $LOG_FILE${NC}"
        echo -e "${YELLOW}   Press Ctrl+C to stop watching (process continues in background)${NC}"
        echo ""

        # Wait a moment for the log file to be created
        sleep 0.5

        # Tail the log, but exit when the process finishes
        tail -f "$LOG_FILE" &
        local TAIL_PID=$!

        # Wait for lldb to finish, then kill tail
        wait $PID 2>/dev/null || true
        kill $TAIL_PID 2>/dev/null || true

        echo ""
        echo -e "${GREEN}✅ Debug session complete.${NC}"
        echo -e "${GREEN}📄 Full log: $LOG_FILE${NC}"
    fi
}

# ── Main ──
print_banner
build_if_needed
find_lldb

echo -e "${GREEN}✅ Binary: $TARGET${NC}"
echo -e "${GREEN}✅ Using:  $LLDB_CMD${NC}"
echo -e "${GREEN}✅ Init:   $LLDBINIT${NC}"
echo ""

case "${1:-}" in
    # ── Pure background mode ──
    --bg)
        QUESTION="${2:?Usage: $0 --bg \"Your question here\"}"
        create_input_file "$QUESTION"
        echo -e "${CYAN}🚀 BACKGROUND mode — question: ${BOLD}$QUESTION${NC}"
        echo ""
        run_batch "$INPUT_FILE" "true"
        ;;

    # ── Batch with input file ──
    --input)
        INPUT="${2:?Usage: $0 --input <file>}"
        if [ ! -f "$INPUT" ]; then
            echo -e "${RED}❌ Input file not found: $INPUT${NC}"
            exit 1
        fi
        echo -e "${CYAN}🚀 BATCH mode with stdin from: ${BOLD}$INPUT${NC}"
        echo ""
        run_batch "$INPUT" "false"
        ;;

    # ── Read from stdin pipe ──
    --stdin)
        cat > "$INPUT_FILE"
        echo -e "${CYAN}🚀 BATCH mode with piped stdin${NC}"
        echo ""
        run_batch "$INPUT_FILE" "false"
        ;;

    # ── Batch mode (no stdin) ──
    --batch)
        echo -e "${CYAN}🚀 BATCH mode — auto-run${NC}"
        echo ""
        (cd "$PROJECT_DIR" && $LLDB_CMD -b \
            -o "command source $LLDBINIT" \
            -o "run" \
            "$TARGET") 2>&1 | tee "$LOG_FILE"
        echo -e "\n${GREEN}📄 Log: $LOG_FILE${NC}"
        ;;

    # ── Interactive + logging ──
    --log)
        echo -e "${CYAN}🚀 INTERACTIVE mode + logging to $LOG_FILE${NC}"
        echo -e "${YELLOW}   Type 'run' to start${NC}"
        echo ""
        (cd "$PROJECT_DIR" && $LLDB_CMD -o "command source $LLDBINIT" "$TARGET") 2>&1 | tee "$LOG_FILE"
        echo -e "\n${GREEN}📄 Log: $LOG_FILE${NC}"
        ;;

    # ── Interactive mode ──
    --interactive)
        echo -e "${CYAN}🚀 INTERACTIVE mode${NC}"
        echo -e "${YELLOW}   Type 'run' to start${NC}"
        echo ""
        (cd "$PROJECT_DIR" && $LLDB_CMD -o "command source $LLDBINIT" "$TARGET")
        ;;

    # ── Help ──
    --help|-h)
        echo -e "${BOLD}Usage:${NC}"
        echo -e "  ${GREEN}./debug_run.sh \"Your question\"${NC}        Auto: run + stream log"
        echo -e "  ${GREEN}./debug_run.sh --bg \"Your question\"${NC}   Pure background, view log later"
        echo -e "  ${GREEN}./debug_run.sh --input <file>${NC}         Batch with input file"
        echo -e "  ${GREEN}./debug_run.sh --stdin${NC}                Read input from pipe"
        echo -e "  ${GREEN}./debug_run.sh --batch${NC}                Batch mode (no stdin)"
        echo -e "  ${GREEN}./debug_run.sh --interactive${NC}          Interactive lldb session"
        echo -e "  ${GREEN}./debug_run.sh --log${NC}                  Interactive + logging"
        echo ""
        echo -e "${BOLD}Examples:${NC}"
        echo -e "  ./debug_run.sh \"What is 42 + 58?\""
        echo -e "  ./debug_run.sh --bg \"Calculate 100 * 3.14\""
        echo -e "  echo -e \"Hello\\nquit\" | ./debug_run.sh --stdin"
        ;;

    # ── Default: auto mode (question as argument) ──
    *)
        if [ -z "${1:-}" ]; then
            echo -e "${RED}❌ Please provide a question or use --help${NC}"
            echo -e "   Example: ${GREEN}./debug_run.sh \"What is 42 + 58?\"${NC}"
            exit 1
        fi
        QUESTION="$1"
        create_input_file "$QUESTION"
        echo -e "${CYAN}🚀 AUTO mode — question: ${BOLD}$QUESTION${NC}"
        echo ""
        run_batch "$INPUT_FILE" "false"
        ;;
esac
