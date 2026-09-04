#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: Compile-free review gate on every PermissionDenied message the server
# ABOUTME: ships to a client — the set must match the reviewed inventory exactly.

# WHY THIS EXISTS
# ---------------
# `AppError::sanitized_message()` passes `ErrorCode::PermissionDenied` messages
# through verbatim, so a refusal names what was refused ("Group coaching
# requires a Professional or Enterprise plan") instead of the one constant
# sentence the code description carries. That is a standing commitment, not a
# one-time review: every future PermissionDenied message a handler writes ships
# to the client the moment it is written, and its author has no way of knowing
# that from the call site — `AppError::new(ErrorCode::PermissionDenied, "…")`
# looks exactly like the ~30 codes whose messages are replaced.
#
# So the reviewed set is committed, and this gate holds the source to it. A new
# refusal message fails the push until someone adds its line, and adding the
# line is the act of reading it. A message that goes away fails too, so the
# inventory cannot accumulate lines nobody can trace to code.
#
# WHAT COUNTS AS A CONSTRUCTION SITE
#   AppError::new(ErrorCode::PermissionDenied, <message>)
#   Self::new(ErrorCode::PermissionDenied, <message>)   (the ToolError conversions)
#   pierre_core::error_helpers::user_state_error(<message>), which mints
#     PermissionDenied with a "User state error: " prefix
#
# Comparisons (`e.code == ErrorCode::PermissionDenied`), match arms and doc
# comments are read sites, not constructions, and are skipped. Anything the
# classifier does not recognise, and any message it cannot resolve to a literal,
# is reported as a blind spot and FAILS — a partial scan that reported "in sync"
# would be worse than no gate at all.
#
# WHAT THIS PINS, AND WHAT IT DOES NOT
# A `format!` message is pinned by its TEMPLATE, so the reviewer sees the shape —
# "Permission required: {permission}" — and judges whether interpolating that
# value is safe. What the template does not pin is the arguments: rewriting
# `format!("… {:?}", a)` to interpolate `b` instead leaves the template, and this
# gate, unchanged. A `&str` constant IS resolved to its value, so editing the
# constant fails here rather than slipping past. The residual case is `{:?}` of a
# type that later grows a field — read the arguments, not only the template.
#
# MODES
#   check-permission-denied-messages.sh           — compare src against the inventory
#   check-permission-denied-messages.sh --list    — print the scanned set (to seed
#                                                   or refresh the inventory body)

set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PROJECT_ROOT="$( cd "$SCRIPT_DIR/../.." && pwd )"
cd "$PROJECT_ROOT"

INVENTORY="scripts/ci/permission-denied-messages.txt"
MODE="${1:-check}"

# The scanner prints one `<file>:<message>` line per reviewed construction site
# on success, or a blind-spot report and exit 1 when it cannot see everything.
scan() {
    python3 - <<'PY'
import pathlib
import re
import sys

MARK = "ErrorCode::PermissionDenied"
HELPER = "user_state_error("
HELPER_PREFIX = "User state error: "

WS = re.compile(r"\s+")
IDENT_PATH = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*(::[A-Za-z_][A-Za-z0-9_]*)*$")
CONST_DECL = re.compile(r'pub const ([A-Z][A-Z0-9_]*): &str = "((?:[^"\\]|\\.)*)"')


def unescape(body):
    """The text a Rust string literal renders, on one line."""
    # A trailing `\` swallows the newline and the following indent.
    body = re.sub(r"\\\n\s*", "", body)
    for src, dst in (('\\"', '"'), ("\\n", " "), ("\\t", " "), ("\\\\", "\\")):
        body = body.replace(src, dst)
    return WS.sub(" ", body).strip()


def line_of(text, at):
    return text.count("\n", 0, at) + 1


def in_comment(text, at):
    start = text.rfind("\n", 0, at) + 1
    end = text.find("\n", at)
    line = text[start : end if end != -1 else len(text)]
    slashes = line.find("//")
    return (slashes != -1 and slashes < at - start) or line.lstrip().startswith("*")


def argument_at(text, at):
    """The source of the call argument starting at `at`, up to its own comma or
    the closing paren of the enclosing call. String-aware, so a comma or paren
    inside a literal does not end it, and comment-aware, so a note explaining
    the refusal does not hide it: this repo asks for that note, and a comma in
    English prose would otherwise read as the end of the argument."""
    depth = 0
    out = []
    i = at
    while i < len(text):
        ch = text[i]
        if text.startswith("//", i):
            newline = text.find("\n", i)
            if newline == -1:
                break
            i = newline + 1
            continue
        if text.startswith("/*", i):
            close = text.find("*/", i + 2)
            if close == -1:
                break
            i = close + 2
            continue
        if ch == '"':
            out.append(ch)
            i += 1
            while i < len(text):
                out.append(text[i])
                if text[i] == "\\":
                    out.append(text[i + 1])
                    i += 2
                    continue
                if text[i] == '"':
                    i += 1
                    break
                i += 1
            continue
        if ch in "([{":
            depth += 1
        elif ch in ")]}":
            if depth == 0:
                break
            depth -= 1
        elif ch == "," and depth == 0:
            break
        out.append(ch)
        i += 1
    return "".join(out).strip()


def message_of(expr, constants):
    """The text an argument expression renders, or None when the scan cannot
    see it. `format!` keeps its placeholders — the shape is what gets read."""
    expr = expr.strip().rstrip(",").strip()
    if expr.startswith("format!("):
        expr = argument_at(expr, len("format!("))
    if expr.startswith('"') and expr.endswith('"') and len(expr) > 1:
        return unescape(expr[1:-1])
    if IDENT_PATH.match(expr):
        return constants.get(expr.split("::")[-1])
    return None


files = sorted(pathlib.Path("crates").glob("*/src/**/*.rs"))
if not files:
    print("no crates/*/src sources found — this check is stale")
    sys.exit(1)

constants = {}
sources = {}
for path in files:
    text = path.read_text(encoding="utf-8")
    sources[path.as_posix()] = text
    for name, body in CONST_DECL.findall(text):
        constants[name] = unescape(body)

sites = set()
blind = []

for rel, text in sources.items():
    for hit in re.finditer(re.escape(MARK), text):
        at = hit.start()
        if in_comment(text, at):
            continue
        before = text[:at].rstrip()
        after = text[at + len(MARK) :].lstrip()
        if before.endswith("::new("):
            comma = text.find(",", at + len(MARK))
            message = message_of(argument_at(text, comma + 1), constants) if comma != -1 else None
            if message is None:
                blind.append(f"{rel}:{line_of(text, at)}: message does not resolve to a literal")
            else:
                sites.add((rel, message))
        elif before.endswith("==") or after.startswith("=>") or after.startswith("|"):
            continue
        else:
            blind.append(f"{rel}:{line_of(text, at)}: unrecognised use of {MARK}")

    for hit in re.finditer(re.escape(HELPER), text):
        at = hit.start()
        if in_comment(text, at):
            continue
        head = text[:at].rstrip()
        if head.endswith("fn") or head.endswith("::"):
            continue
        line_start = text.rfind("\n", 0, at) + 1
        if text[line_start:at].lstrip().startswith("use "):
            continue
        message = message_of(argument_at(text, at + len(HELPER)), constants)
        if message is None:
            blind.append(f"{rel}:{line_of(text, at)}: message does not resolve to a literal")
        else:
            sites.add((rel, HELPER_PREFIX + message))

if blind:
    print("\n".join(blind))
    sys.exit(1)

for rel, message in sorted(sites):
    print(f"{rel}:{message}")
PY
}

if [[ "$MODE" != "check" && "$MODE" != "--list" ]]; then
    echo "usage: $(basename "$0") [--list]" >&2
    exit 2
fi

if [[ "$MODE" == "--list" ]]; then
    scan
    exit $?
fi

echo -e "${BLUE}==== PermissionDenied Message Review (static) ====${NC}"

if ! SCAN="$(scan)"; then
    echo -e "${RED}❌ PermissionDenied scan incomplete:${NC}"
    printf '%s\n' "$SCAN" | sed 's/^/   /'
    echo -e "${YELLOW}   A refusal is built somewhere this static scan cannot read — a computed${NC}"
    echo -e "${YELLOW}   message, or a construction shape the classifier does not know. Give the${NC}"
    echo -e "${YELLOW}   message a literal, or extend this check — never ignore it: a partial scan${NC}"
    echo -e "${YELLOW}   reports 'in sync' while an unreviewed sentence ships to clients.${NC}"
    exit 1
fi

if [[ ! -f "$INVENTORY" ]]; then
    echo -e "${RED}❌ ${INVENTORY} not found — the reviewed set is what this gate compares against.${NC}"
    exit 1
fi

REVIEWED="$(grep -v '^[[:space:]]*#' "$INVENTORY" | grep -v '^[[:space:]]*$' || true)"
SCANNED_N="$(printf '%s\n' "$SCAN" | grep -c . || true)"
REVIEWED_N="$(printf '%s\n' "$REVIEWED" | grep -c . || true)"

if [[ "$REVIEWED_N" -eq 0 ]]; then
    echo -e "${RED}❌ ${INVENTORY} holds no entries — every PermissionDenied message would be unreviewed.${NC}"
    exit 1
fi

# The inventory is sorted so a new entry lands next to its neighbours in review
# and two sessions adding one line each do not conflict on the same hunk.
if ! diff -q <(printf '%s\n' "$REVIEWED") <(printf '%s\n' "$REVIEWED" | LC_ALL=C sort) >/dev/null; then
    echo -e "${RED}❌ ${INVENTORY} is not in byte order:${NC}"
    diff <(printf '%s\n' "$REVIEWED") <(printf '%s\n' "$REVIEWED" | LC_ALL=C sort) | sed 's/^/   /' | head -12
    exit 1
fi

UNREVIEWED="$(comm -23 <(printf '%s\n' "$SCAN" | LC_ALL=C sort) \
                       <(printf '%s\n' "$REVIEWED" | LC_ALL=C sort) || true)"
STALE="$(comm -13 <(printf '%s\n' "$SCAN" | LC_ALL=C sort) \
                  <(printf '%s\n' "$REVIEWED" | LC_ALL=C sort) || true)"

FAILED=false

if [[ -n "$UNREVIEWED" ]]; then
    echo -e "${RED}❌ PermissionDenied message(s) in src that nobody has reviewed:${NC}"
    printf '%s\n' "$UNREVIEWED" | sed 's/^/   /'
    echo -e "${YELLOW}   sanitized_message() ships a PermissionDenied message to the client${NC}"
    echo -e "${YELLOW}   verbatim. Read yours: it must name what was refused and nothing else —${NC}"
    echo -e "${YELLOW}   no table or column name, no id, no operator-only fact. Then add the line${NC}"
    echo -e "${YELLOW}   to ${INVENTORY} (kept in byte order).${NC}"
    FAILED=true
fi

if [[ -n "$STALE" ]]; then
    echo -e "${RED}❌ ${INVENTORY} line(s) whose construction site no longer exists:${NC}"
    printf '%s\n' "$STALE" | sed 's/^/   /'
    echo -e "${YELLOW}   The message was reworded, moved, or deleted. Drop the stale line (and add${NC}"
    echo -e "${YELLOW}   the new wording, which arrives as an unreviewed entry above).${NC}"
    FAILED=true
fi

if [[ "$FAILED" == "true" ]]; then
    echo -e "${RED}❌ PERMISSIONDENIED MESSAGE REVIEW FAILED${NC}"
    exit 1
fi

echo -e "${GREEN}✅ PermissionDenied messages: all ${SCANNED_N} construction sites are reviewed (${REVIEWED_N} entries).${NC}"
exit 0
