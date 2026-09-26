#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: Fails when a committed Boreal stylesheet (hosted page, web and mobile token blocks) is not what its generator emits
# ABOUTME: Also fails a hosted template or a client stylesheet that skips its generated sheet or declares a palette of its own

# WHY THIS EXISTS
# ---------------
# Every server-rendered hosted page (OAuth login and consent, the Sciotte and
# connect pages a chat link opens, the messaging link pages) embeds one
# stylesheet, crates/pierre-core/src/hosted_page.css, generated from the Boreal
# tokens in packages/shared-constants. The Rust crates cannot run the
# generator, so the file is committed — and a committed generated file goes
# stale the moment a token moves and nobody regenerates. Before this sheet
# existed, fourteen templates each carried their own copy of a palette the
# product had retired months earlier, because nothing tied them to the tokens.
#
# The two client stylesheets are the same shape of problem. The web's
# frontend/src/index.css and the phone's frontend-mobile/global.css each carried
# a hand copy of the token tree as --color-* variables, next to the TypeScript
# source in design-system.ts, and a parity test pinned ten of the forty-one
# values; the rest drifted freely. Both now import a block generated from the
# same tokens (packages/shared-constants/scripts/generate-client-css.ts), and
# the generator refuses to write one below the WCAG floors it measures.
#
# Four checks, all compile-free:
#   1. regenerate the hosted sheet into a temp file and diff it against the committed one;
#   2. regenerate both client blocks the same way and diff each;
#   3. every hosted template asks for the sheet and declares no styles of its own;
#   4. each client stylesheet imports its block and declares no custom property itself.
#
# It fails closed: a generator that exits non-zero, writes nothing, or writes a
# sheet without its dark scheme is a failure, never a pass over an empty diff.

set -uo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PROJECT_ROOT="$( cd "$SCRIPT_DIR/../.." && pwd )"
cd "$PROJECT_ROOT" || exit 1

COMMITTED="crates/pierre-core/src/hosted_page.css"
GENERATOR="packages/shared-constants/scripts/generate-hosted-css.ts"
REGENERATE="cd packages/shared-constants && bun run generate:hosted-css"
PLACEHOLDER='<style>{{HOSTED_PAGE_CSS}}</style>'

CLIENT_GENERATOR="packages/shared-constants/scripts/generate-client-css.ts"
CLIENT_REGENERATE="cd packages/shared-constants && bun run generate:client-css"
# Each client stylesheet, the generated block it imports, and the line that opens
# that block's dark scheme (so a generator emitting light only cannot pass).
CLIENT_SHEETS=(
    "frontend/src/index.css|frontend/src/boreal-tokens.generated.css|^html.dark {\$"
    "frontend-mobile/global.css|frontend-mobile/boreal-tokens.generated.css|^  :root.dark, .dark {\$"
)
CLIENT_IMPORT="@import './boreal-tokens.generated.css';"

echo -e "${BLUE}==== Boreal Generated Stylesheet Gate ====${NC}"

FAILED=false
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# ---------------------------------------------------------------------------
# Check 1: the committed sheet is the generator's output
# ---------------------------------------------------------------------------
if ! command -v bun >/dev/null 2>&1; then
    echo -e "${RED}❌ bun is not on PATH, so the stylesheet cannot be regenerated to compare.${NC}"
    echo -e "${YELLOW}   Install bun (the project's only package manager) and re-run.${NC}"
    FAILED=true
elif [[ ! -f "$COMMITTED" ]]; then
    echo -e "${RED}❌ ${COMMITTED} is missing.${NC}"
    echo -e "${YELLOW}   Generate it: ${REGENERATE}${NC}"
    FAILED=true
else
    FRESH="$TMP/hosted_page.css"
    # The generator's exit status is judged on its own, before anything reads
    # the file it was supposed to write.
    if ! GEN_OUT="$(bun "$GENERATOR" --out "$FRESH" 2>&1)"; then
        echo -e "${RED}❌ The generator failed:${NC}"
        printf '%s\n' "$GEN_OUT" | sed 's/^/   /'
        FAILED=true
    elif [[ ! -s "$FRESH" ]]; then
        echo -e "${RED}❌ The generator exited 0 but wrote nothing — nothing was compared.${NC}"
        FAILED=true
    elif ! grep -q '^@media (prefers-color-scheme: dark) {$' "$FRESH" \
        || ! grep -q '^  color-scheme: light dark;$' "$FRESH"; then
        echo -e "${RED}❌ The generated sheet carries no dark scheme — the generator is not emitting what this gate expects.${NC}"
        FAILED=true
    elif ! diff -u "$COMMITTED" "$FRESH" > "$TMP/stale.diff"; then
        echo -e "${RED}❌ ${COMMITTED} is stale against the tokens:${NC}"
        head -40 "$TMP/stale.diff" | sed 's/^/   /'
        echo -e "${YELLOW}   Regenerate it and commit the result:${NC}"
        echo -e "${YELLOW}     ${REGENERATE}${NC}"
        FAILED=true
    else
        echo -e "${GREEN}✅ Stylesheet: the committed sheet is the generator's output ($(printf '%s' "$GEN_OUT" | tail -1 | sed -E 's/ -> .*//')).${NC}"
    fi
fi

# ---------------------------------------------------------------------------
# Check 2: each committed client token block is the generator's output
# ---------------------------------------------------------------------------
if ! command -v bun >/dev/null 2>&1; then
    echo -e "${RED}❌ bun is not on PATH, so the client token blocks cannot be regenerated to compare.${NC}"
    FAILED=true
elif ! CLIENT_OUT="$(bun "$CLIENT_GENERATOR" --out-dir "$TMP/client" 2>&1)"; then
    echo -e "${RED}❌ The client stylesheet generator failed:${NC}"
    printf '%s\n' "$CLIENT_OUT" | sed 's/^/   /'
    FAILED=true
else
    CLIENT_STALE=false
    for entry in "${CLIENT_SHEETS[@]}"; do
        IFS='|' read -r _sheet block dark_line <<< "$entry"
        fresh="$TMP/client/$block"
        if [[ ! -s "$fresh" ]]; then
            echo -e "${RED}❌ The client generator exited 0 but wrote no ${block} — nothing was compared.${NC}"
            CLIENT_STALE=true
        elif ! grep -q "$dark_line" "$fresh"; then
            echo -e "${RED}❌ The generated ${block} carries no dark scheme — the generator is not emitting what this gate expects.${NC}"
            CLIENT_STALE=true
        elif [[ ! -f "$block" ]]; then
            echo -e "${RED}❌ ${block} is missing.${NC}"
            CLIENT_STALE=true
        elif ! diff -u "$block" "$fresh" > "$TMP/client.diff"; then
            echo -e "${RED}❌ ${block} is stale against the tokens:${NC}"
            head -40 "$TMP/client.diff" | sed 's/^/   /'
            CLIENT_STALE=true
        fi
    done
    if [[ "$CLIENT_STALE" == "true" ]]; then
        echo -e "${YELLOW}   Regenerate them and commit the result:${NC}"
        echo -e "${YELLOW}     ${CLIENT_REGENERATE}${NC}"
        FAILED=true
    else
        echo -e "${GREEN}✅ Client token blocks: both committed blocks are the generator's output.${NC}"
    fi
fi

# ---------------------------------------------------------------------------
# Check 3: every hosted template draws with the shared sheet, and only with it
# ---------------------------------------------------------------------------
# Discovered, never listed: a page added later is held to the same rule.
TEMPLATES="$(grep -rl --include='*.html' '<html' crates/*/templates crates/*/src 2>/dev/null | sort || true)"
TEMPLATE_COUNT="$(printf '%s\n' "$TEMPLATES" | grep -c . || true)"

if [[ "$TEMPLATE_COUNT" -eq 0 ]]; then
    echo -e "${RED}❌ Found zero hosted templates under crates/ — this check is stale.${NC}"
    FAILED=true
else
    OFFENDERS=""
    while IFS= read -r template; do
        [[ -z "$template" ]] && continue
        if ! grep -qF "$PLACEHOLDER" "$template"; then
            OFFENDERS+="   ${template}: does not ask for the shared sheet (${PLACEHOLDER})\n"
        fi
        STYLE_BLOCKS="$(grep -o '<style' "$template" | wc -l | tr -d ' ')"
        if [[ "$STYLE_BLOCKS" -gt 1 ]]; then
            OFFENDERS+="   ${template}: declares a <style> block of its own beside the shared sheet\n"
        fi
        if grep -qE 'style="|--pierre-|#7[Cc]3[Aa][Ee][Dd]|#06[Bb]6[Dd]4|linear-gradient' "$template"; then
            OFFENDERS+="   ${template}: carries an inline style, a --pierre-* variable or the retired violet palette\n"
        fi
    done <<< "$TEMPLATES"

    if [[ -n "$OFFENDERS" ]]; then
        echo -e "${RED}❌ Hosted templates drawing outside the shared Boreal sheet:${NC}"
        printf '%b' "$OFFENDERS"
        echo -e "${YELLOW}   A hosted page styles itself with the classes in ${COMMITTED}; a class${NC}"
        echo -e "${YELLOW}   it needs goes into ${GENERATOR}, measured in both schemes.${NC}"
        FAILED=true
    else
        echo -e "${GREEN}✅ Templates: all ${TEMPLATE_COUNT} hosted pages embed the shared sheet and nothing else.${NC}"
    fi
fi

# ---------------------------------------------------------------------------
# Check 4: each client stylesheet imports its block and keeps no palette of its own
# ---------------------------------------------------------------------------
# A custom property declared beside the import is how the hand copy comes back:
# it silently overrides or extends the generated value on one client only.
SHEET_OFFENDERS=""
for entry in "${CLIENT_SHEETS[@]}"; do
    IFS='|' read -r sheet _block _dark <<< "$entry"
    if [[ ! -f "$sheet" ]]; then
        SHEET_OFFENDERS+="   ${sheet}: missing\n"
        continue
    fi
    if ! grep -qxF "$CLIENT_IMPORT" "$sheet"; then
        SHEET_OFFENDERS+="   ${sheet}: does not import its generated block (${CLIENT_IMPORT})\n"
    fi
    DECLARED="$(grep -nE '^[[:space:]]*--[A-Za-z0-9-]+[[:space:]]*:' "$sheet" || true)"
    if [[ -n "$DECLARED" ]]; then
        SHEET_OFFENDERS+="   ${sheet}: declares custom properties of its own:\n$(printf '%s\n' "$DECLARED" | head -5 | sed 's/^/     /')\n"
    fi
done
if [[ -n "$SHEET_OFFENDERS" ]]; then
    echo -e "${RED}❌ Client stylesheets drawing outside their generated token block:${NC}"
    printf '%b' "$SHEET_OFFENDERS"
    echo -e "${YELLOW}   A token lives in packages/shared-constants/src/design-system.ts; a variable${NC}"
    echo -e "${YELLOW}   a client needs goes into ${CLIENT_GENERATOR}, measured in both schemes.${NC}"
    FAILED=true
else
    echo -e "${GREEN}✅ Client stylesheets: both import their generated block and declare no palette of their own.${NC}"
fi

if [[ "$FAILED" == "true" ]]; then
    echo ""
    echo -e "${RED}❌ BOREAL GENERATED STYLESHEET GATE FAILED${NC}"
    exit 1
fi
exit 0
