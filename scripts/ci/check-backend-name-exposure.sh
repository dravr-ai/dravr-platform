#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: Fails when the scraping backend's name (sciotte) reaches an athlete or coach: a
# ABOUTME: translation VALUE in the string catalogue, or the served text of a hosted page template

# carnet#1207. Directive from ChefFamille: "SCIOTTE must not be visible to the
# user/athlete/coach". Sciotte is the backend that signs in to Strava, Garmin,
# TrainingPeaks and COROS with the athlete's own username and password; the
# athlete knows the service, never the backend. Before this gate the name had
# reached the Settings switch dialog ("Strava — Sciotte"), the disconnect
# banner, the reconnect error text the model repeated, and the comments and
# script of the hosted sign-in page — every one of them added by an edit that
# had no reason to think about it, because the provider ids are spelled
# sciotte_* all through the code.
#
# What is scanned, and what is not:
#   - Every string VALUE of a catalogue file (packages/i18n/src/locales/*/
#     translation.json — the copy of dravr-contremaitre's strings/<l>.json that
#     both clients bundle and the messaging-strings registry seeds from, so the
#     messaging strings are this same file). KEYS are not scanned: the i18n keys
#     are identifiers (`providers.stravaSciotte`, `connect.sciotteBack`) and the
#     task that added this gate forbids renaming them.
#   - Every served template under crates/*/templates/*.html, all of it — text,
#     attributes, HTML comments and script — because view-source and the
#     network tab are part of what an athlete can see. Two identifier shapes are
#     allowed, and nothing else:
#       1. the API route prefix `/api/providers/sciotte/` — the provider's id
#          in a URL the page calls, which clients already depend on;
#       2. a quoted provider-id literal ('sciotte' or 'sciotte_<service>')
#          inside a <script> block — the id the page compares against, never
#          rendered.
#   - Not scanned: crates/pierre-core/src/hosted_page.css (generated from the
#     Boreal tokens, whose provider-glyph selectors match on provider ids), Rust
#     sources (the LLM-boundary tests in the leaf crates and the server's `it`
#     suite pin what the model reads), logs and metrics, which may keep the name.
#
# Usage:
#   check-backend-name-exposure.sh            the CI invocation: the repo's catalogue and templates
#   check-backend-name-exposure.sh PATH...    exactly these files or directories, from the caller's
#                                             working directory (*.json read as catalogues, *.html as
#                                             templates). The self-test drives this form, and so can
#                                             dravr-contremaitre on its own strings/ tree.

set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m'

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PROJECT_ROOT="$( cd "$SCRIPT_DIR/../.." && pwd )"

MODE="paths"
if [ "$#" -eq 0 ]; then
    cd "$PROJECT_ROOT"
    MODE="repo"
    set -- packages/i18n/src/locales crates
fi

echo -e "${BLUE}==== Backend name exposure (carnet#1207) ====${NC}"

# The scan asserts its own premise: a run that read no catalogue value, or (on
# the repo) no template, has verified nothing and fails rather than passing.
if ! OUT="$(python3 - "$MODE" "$@" <<'PY' 2>&1
import json, os, re, sys

mode, roots = sys.argv[1], sys.argv[2:]
NAME = re.compile(r"sciotte", re.I)
API_ROUTE = re.compile(r"/api/providers/sciotte/")
SCRIPT = re.compile(r"<script\b[^>]*>(.*?)</script\s*>", re.S | re.I)
ID_LITERAL = re.compile(r"(['\"])sciotte(?:_[a-z0-9_]+)?\1")


def collect(roots):
    catalogues, templates = [], []
    for root in roots:
        if os.path.isfile(root):
            (catalogues if root.endswith(".json") else templates).append(root)
            continue
        if not os.path.isdir(root):
            print(f"MISSING {root}")
            raise SystemExit(1)
        for dirpath, dirnames, filenames in os.walk(root):
            dirnames[:] = sorted(d for d in dirnames if d not in ("node_modules", "target", ".git"))
            for name in sorted(filenames):
                path = os.path.join(dirpath, name)
                if mode == "repo":
                    # Only the shapes that ship: the locale copies and the
                    # templates a crate serves. Any other JSON in a crate
                    # (fixtures, test data) is not a catalogue.
                    if name == "translation.json":
                        catalogues.append(path)
                    elif name.endswith(".html") and os.path.basename(dirpath) == "templates":
                        templates.append(path)
                elif name.endswith(".json"):
                    catalogues.append(path)
                elif name.endswith(".html"):
                    templates.append(path)
    return catalogues, templates


def leaves(node, prefix=""):
    if isinstance(node, dict):
        for key, value in node.items():
            yield from leaves(value, f"{prefix}.{key}" if prefix else key)
    elif isinstance(node, list):
        for index, value in enumerate(node):
            yield from leaves(value, f"{prefix}[{index}]")
    elif isinstance(node, str):
        yield prefix, node


catalogues, templates = collect(roots)
findings, values = [], 0

for path in catalogues:
    with open(path, encoding="utf8") as handle:
        data = json.load(handle)
    count = 0
    for key, value in leaves(data):
        count += 1
        if NAME.search(value):
            findings.append(f"VALUE {path} {key} = {value!r}")
    if count == 0:
        print(f"EMPTY CATALOGUE {path}")
        raise SystemExit(1)
    values += count

for path in templates:
    with open(path, encoding="utf8") as handle:
        text = handle.read()
    allowed = [m.span() for m in API_ROUTE.finditer(text)]
    for block in SCRIPT.finditer(text):
        start = block.start(1)
        allowed += [(start + m.start(), start + m.end()) for m in ID_LITERAL.finditer(block.group(1))]
    for hit in NAME.finditer(text):
        if any(lo <= hit.start() < hi for lo, hi in allowed):
            continue
        line_no = text.count("\n", 0, hit.start()) + 1
        line = text.splitlines()[line_no - 1].strip()
        findings.append(f"TEMPLATE {path}:{line_no} {line[:160]}")

print(f"catalogue files: {len(catalogues)} ({values} values)")
print(f"templates: {len(templates)}")
for finding in findings:
    print(finding)
if not catalogues and not templates:
    print("SCAN INCOMPLETE")
    raise SystemExit(1)
if mode == "repo" and (not catalogues or not templates):
    print("SCAN INCOMPLETE")
    raise SystemExit(1)
if findings:
    raise SystemExit(1)
print("BACKEND NAME NOT EXPOSED")
PY
)"; then
    echo "$OUT" | sed 's/^/    /'
    if echo "$OUT" | grep -qE "SCAN INCOMPLETE|EMPTY CATALOGUE|^MISSING "; then
        echo -e "${RED}❌ Scan verified nothing — a missing path, an empty catalogue, or no files to read.${NC}"
        echo -e "${RED}Fix the scan or its inputs; an empty scan is never a pass.${NC}"
    else
        echo -e "${RED}❌ The backend's name reaches an athlete or coach.${NC}"
        echo -e "${RED}Name the service instead (Strava, Garmin, TrainingPeaks, COROS), or describe the${NC}"
        echo -e "${RED}connection as signing in with a username and password from Dravr.${NC}"
        echo -e "${RED}A catalogue value is fixed in dravr-contremaitre's strings/, never in packages/i18n.${NC}"
    fi
    exit 1
fi

# Grep for the success marker, never for the absence of a finding: a crashed
# scan produces no findings either.
if ! echo "$OUT" | grep -q "BACKEND NAME NOT EXPOSED"; then
    echo "$OUT" | sed 's/^/    /'
    echo -e "${RED}❌ Scan did not report success — treating as failure.${NC}"
    exit 1
fi

echo "$OUT" | grep -v "BACKEND NAME NOT EXPOSED" | sed 's/^/    /'
echo -e "${GREEN}✅ No catalogue value or served template names the backend${NC}"
exit 0
