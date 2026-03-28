#!/usr/bin/env bash
#
# sync.sh - Rsync sync script between local and remote ironclaw repo
#
# Usage:
#   ./sync.sh push       Push local changes to remote server
#   ./sync.sh pull       Pull remote changes to local
#   ./sync.sh push -n    Dry-run push (preview only)
#   ./sync.sh pull -n    Dry-run pull (preview only)
#

set -euo pipefail

# ============ Configuration ============
LOCAL_DIR="/Users/ericksun/workspace/ironclaw/"
REMOTE_HOST="ericksun@172.16.117.160"
REMOTE_DIR="/home/ericksun/ironclaw/"

# Rsync common options:
#   -a          archive mode (preserves permissions, timestamps, etc.)
#   -v          verbose
#   -z          compress during transfer
#   --progress  show transfer progress
#   --delete    delete files on destination that don't exist on source
RSYNC_OPTS="-avz --progress --delete"

# Exclude patterns (files/dirs that should NOT be synced)
EXCLUDES=(
    ".git/"
    ".DS_Store"
    ".vscode/"
    ".env"
    ".env.local"
    ".env.*"
    ".claude/"
    ".sidecar/"
    ".todos/"
    "target/"
    "__pycache__/"
    "*.pyc"
    "*.pyo"
    "*.pyd"
    "bench-results/"
    "coverage/"
    "*.wasm"
    "trace_*.json"
    ".worktrees/"
    "ironclaw-plugin/"
)

# ============ Functions ============

usage() {
    echo "Usage: $0 {push|pull} [-n]"
    echo ""
    echo "Commands:"
    echo "  push    Sync local -> remote (upload)"
    echo "  pull    Sync remote -> local (download)"
    echo ""
    echo "Options:"
    echo "  -n      Dry-run mode (preview changes without applying)"
    echo ""
    echo "Examples:"
    echo "  $0 push        # Upload local changes to server"
    echo "  $0 pull        # Download server changes to local"
    echo "  $0 push -n     # Preview what would be uploaded"
    echo "  $0 pull -n     # Preview what would be downloaded"
    exit 1
}

build_exclude_args() {
    local args=""
    for pattern in "${EXCLUDES[@]}"; do
        args="$args --exclude=$pattern"
    done
    echo "$args"
}

do_sync() {
    local direction="$1"
    local dry_run="${2:-}"
    local extra_opts=""

    if [[ "$dry_run" == "-n" ]]; then
        extra_opts="--dry-run"
        echo "🔍 DRY-RUN mode: no files will be modified"
        echo ""
    fi

    local exclude_args
    exclude_args=$(build_exclude_args)

    if [[ "$direction" == "push" ]]; then
        echo "⬆️  Pushing: ${LOCAL_DIR} -> ${REMOTE_HOST}:${REMOTE_DIR}"
        echo "---------------------------------------------------"
        # shellcheck disable=SC2086
        rsync $RSYNC_OPTS $extra_opts $exclude_args \
            "$LOCAL_DIR" "${REMOTE_HOST}:${REMOTE_DIR}"
    elif [[ "$direction" == "pull" ]]; then
        echo "⬇️  Pulling: ${REMOTE_HOST}:${REMOTE_DIR} -> ${LOCAL_DIR}"
        echo "---------------------------------------------------"
        # shellcheck disable=SC2086
        rsync $RSYNC_OPTS $extra_opts $exclude_args \
            "${REMOTE_HOST}:${REMOTE_DIR}" "$LOCAL_DIR"
    fi

    echo ""
    if [[ "$dry_run" == "-n" ]]; then
        echo "✅ Dry-run complete. No files were changed."
    else
        echo "✅ Sync complete!"
    fi
}

# ============ Main ============

if [[ $# -lt 1 ]]; then
    usage
fi

COMMAND="$1"
DRY_RUN="${2:-}"

case "$COMMAND" in
    push|pull)
        do_sync "$COMMAND" "$DRY_RUN"
        ;;
    *)
        echo "❌ Unknown command: $COMMAND"
        echo ""
        usage
        ;;
esac
