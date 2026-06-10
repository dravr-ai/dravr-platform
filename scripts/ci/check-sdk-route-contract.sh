#!/bin/bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: WARNS when an api-client ENDPOINTS path has no matching mounted backend axum route
# ABOUTME: Catches phantom "404-by-construction" SDK methods whose path the server never registers

# The cross-platform api-client centralizes its HTTP paths in
# packages/api-client/src/core/endpoints.ts (the ENDPOINTS map). Every one of
# those paths must correspond to a route the backend actually mounts via
# `.route("<path>", ...)`. A path present in the SDK but absent from the
# backend router is a phantom method: it compiles, ships, and 404s by
# construction the first time anyone calls it.
#
# Path matching is necessarily heuristic — axum routers can be `.nest()`ed
# under a prefix, params are spelled `{id}` (backend) vs `${id}` (SDK), and
# some routes are assembled dynamically. To avoid false-positive CI breakage
# this gate is WARNING-ONLY: it normalizes params, compares the SDK paths
# against the union of all backend `.route(...)` literals, and lists the
# unmatched ones for a human to confirm. It never fails the build.

# No `set -e`: advisory gate; all logic lives in the embedded python, which
# always exits 0.
set -uo pipefail

BLUE='\033[0;34m'
NC='\033[0m'

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PROJECT_ROOT="$( cd "$SCRIPT_DIR/../.." && pwd )"
cd "$PROJECT_ROOT"

echo -e "${BLUE}==== SDK path vs mounted-route contract (advisory) ====${NC}"

ENDPOINTS_FILE="packages/api-client/src/core/endpoints.ts"
if [ ! -f "$ENDPOINTS_FILE" ]; then
    echo "No $ENDPOINTS_FILE — nothing to check"
    exit 0
fi

python3 - "$ENDPOINTS_FILE" <<'PY'
import re, glob, sys

YELLOW = '\033[1;33m'
GREEN = '\033[0;32m'
NC = '\033[0m'

endpoints_file = sys.argv[1]

def norm(p):
    p = re.sub(r'\$\{[^}]*\}', '{param}', p)              # TS template ${id}
    p = re.sub(r'\{[^}]*\}', '{param}', p)                # axum {id}
    p = re.sub(r':[a-zA-Z_][a-zA-Z0-9_]*', '{param}', p)  # axum :id
    p = p.split('?')[0]                                    # drop query string
    return p.rstrip('/') or '/'

# --- backend: every string literal that is the first arg to .route( ---
backend = set()
for f in glob.glob('crates/**/*.rs', recursive=True):
    if '/tests/' in f:
        continue
    try:
        text = open(f, encoding='utf-8', errors='ignore').read()
    except OSError:
        continue
    for m in re.finditer(r'\.route\(\s*"([^"]*)"', text):
        backend.add(norm(m.group(1)))

# --- sdk: path-like string/template literals in the ENDPOINTS map ---
sdk = {}
ep = open(endpoints_file, encoding='utf-8').read()
for m in re.finditer(r"[`'](/[A-Za-z0-9][^`'\s]*)[`']", ep):
    raw = m.group(1)
    sdk.setdefault(norm(raw), raw)

missing = sorted((orig, n) for n, orig in sdk.items() if n not in backend)

print(f"  api-client paths: {len(sdk)}   backend routes: {len(backend)}")
if missing:
    print(f"{YELLOW}⚠️  {len(missing)} api-client path(s) with no exact mounted-route match:{NC}")
    for orig, n in missing:
        print(f"    {orig}   (normalized: {n})")
    print(f"{YELLOW}   Each is a phantom-404 candidate: confirm the backend mounts it (it may be{NC}")
    print(f"{YELLOW}   nested under a prefix or assembled dynamically) or fix the SDK path.{NC}")
    print(f"{YELLOW}   Advisory only — not failing the build.{NC}")
else:
    print(f"{GREEN}✅ Every api-client path maps to a mounted backend route{NC}")
PY

exit 0
