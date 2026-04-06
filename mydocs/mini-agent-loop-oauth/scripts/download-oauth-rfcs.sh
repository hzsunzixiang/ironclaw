#!/usr/bin/env bash
# ============================================================
# Download OAuth-related RFCs
# ============================================================
# Downloads the core OAuth 2.0 / 2.1 RFCs and related security
# specifications as plain text from the IETF datatracker.
#
# Usage:
#   chmod +x scripts/download-oauth-rfcs.sh
#   ./scripts/download-oauth-rfcs.sh
# ============================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
RFC_DIR="$PROJECT_DIR/rfcs"

mkdir -p "$RFC_DIR"

# Core OAuth RFCs
declare -A RFCS=(
    # OAuth 2.0 core framework
    ["rfc6749"]="The OAuth 2.0 Authorization Framework"
    # OAuth 2.0 Bearer Token Usage
    ["rfc6750"]="The OAuth 2.0 Authorization Framework: Bearer Token Usage"
    # PKCE - Proof Key for Code Exchange
    ["rfc7636"]="Proof Key for Code Exchange by OAuth Public Clients"
    # OAuth 2.0 Token Revocation
    ["rfc7009"]="OAuth 2.0 Token Revocation"
    # OAuth 2.0 Token Introspection
    ["rfc7662"]="OAuth 2.0 Token Introspection"
    # OAuth 2.0 Dynamic Client Registration
    ["rfc7591"]="OAuth 2.0 Dynamic Client Registration Protocol"
    # OAuth 2.0 Authorization Server Metadata
    ["rfc8414"]="OAuth 2.0 Authorization Server Metadata"
    # OAuth 2.0 Security Best Current Practice
    ["rfc9700"]="OAuth 2.0 Security Best Current Practice"
    # OAuth 2.1 (draft became RFC 9728 area, but the main spec)
    ["rfc6819"]="OAuth 2.0 Threat Model and Security Considerations"
    # Resource Indicators for OAuth 2.0 (RFC 8707)
    ["rfc8707"]="Resource Indicators for OAuth 2.0"
)

echo "╔══════════════════════════════════════════════════════════════╗"
echo "║           OAuth RFC Downloader                              ║"
echo "╠══════════════════════════════════════════════════════════════╣"
echo "║  Downloading ${#RFCS[@]} OAuth-related RFCs to: rfcs/              ║"
echo "╚══════════════════════════════════════════════════════════════╝"
echo ""

DOWNLOADED=0
SKIPPED=0
FAILED=0

for rfc in $(echo "${!RFCS[@]}" | tr ' ' '\n' | sort); do
    desc="${RFCS[$rfc]}"
    outfile="$RFC_DIR/${rfc}.txt"

    if [ -f "$outfile" ]; then
        echo "  ⏭  $rfc — already exists (skipped)"
        SKIPPED=$((SKIPPED + 1))
        continue
    fi

    echo -n "  ⬇  $rfc — $desc ... "

    # IETF datatracker provides plain text at this URL pattern
    url="https://www.rfc-editor.org/rfc/${rfc}.txt"

    if curl -sSfL --max-time 30 -o "$outfile" "$url" 2>/dev/null; then
        echo "✅"
        DOWNLOADED=$((DOWNLOADED + 1))
    else
        echo "❌ (failed)"
        rm -f "$outfile"
        FAILED=$((FAILED + 1))
    fi

    # Be polite to the RFC editor server
    sleep 0.5
done

echo ""
echo "Done: $DOWNLOADED downloaded, $SKIPPED skipped, $FAILED failed"
echo "RFC files are in: $RFC_DIR/"
