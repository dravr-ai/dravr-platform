#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: Fails when "operator" names an admin again in crates/, frontend/, frontend-mobile/ or packages/
# ABOUTME: Every remaining "operator" must match one entry of the allowlist below, each with its reason

# carnet#1644. Directive from ChefFamille, 2026-11-13: "Please use admin and
# stop using operator", and "Admin == super admin since day 1". About 2,000
# occurrences were renamed in one change; a rename enforced by reading once is
# a snapshot, and this is the fence that keeps it.
#
# What is scanned: every tracked file under the four trees (`git grep`, so
# node_modules and build output never count). A line passes when, after the
# allowlisted spans for its path are removed, no "operator" (any case) is left
# on it. An allowlist entry is a path regex, a span regex and the reason the
# word stays; add one only for a meaning that is not an admin, or a name a
# database, a shipped client or a third party fixes.
#
# Usage:
#   check-admin-vocabulary.sh           the CI invocation: the four trees of this repo
#   check-admin-vocabulary.sh PATH...   exactly these files or directories (the self-test's form)

set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m'

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PROJECT_ROOT="$( cd "$SCRIPT_DIR/../.." && pwd )"

if [ "$#" -eq 0 ]; then
    cd "$PROJECT_ROOT"
    set -- crates frontend frontend-mobile packages
fi

for root in "$@"; do
    if [ ! -e "$root" ]; then
        echo -e "${RED}MISSING $root${NC}"
        exit 1
    fi
done

echo -e "${BLUE}==== Admin vocabulary (carnet#1644) ====${NC}"

# `git grep` exits 1 on no match, which is the clean case; 2+ is an error.
set +e
HITS="$(git grep -n -I -i 'operator' -- "$@")"
STATUS=$?
set -e
if [ "$STATUS" -gt 1 ]; then
    echo -e "${RED}git grep failed (status $STATUS); nothing was verified${NC}"
    exit 1
fi

if ! OUT="$(HITS="$HITS" python3 - <<'PY' 2>&1
import os, re, sys

# (path regex, span regex, reason). The span is removed from the line before
# the line is checked again, so a line that also says "operator" about an
# admin still fails.
ALLOW = [
    # Database: the admin_tokens.operator_user_id column is never renamed (a
    # column rename makes a rollback deploy unsafe), and the Rust and
    # TypeScript fields that map it keep its name with a doc line saying so.
    # A log field is not that mapping: `operator_user_id = %x` in a tracing
    # macro names an admin and says admin_user_id.
    (r"", r"operator_user_id(?!\s*=\s*[%?])", "admin_tokens column name"),
    # Persisted: the purge attestation reason rows already hold.
    (r"^crates/", r"operator_disconnect", "stored provider_purges reason value"),
    # Persisted: the deletion receipt actor, under a CHECK constraint and
    # covered by every sealed receipt hash.
    (r"^crates/pierre-database/src/repositories/account_deletion\.rs$", r"[\"`]operator[\"`]", "sealed deletion_receipts.actor value"),
    (r"^crates/pierre-services/src/admin_ops/caller\.rs$", r"\"operator\"", "sealed deletion_receipts.actor value"),
    (r"^crates/pierre-server/tests/it/admin_user_removal_test\.rs$", r"\"operator\"", "sealed deletion_receipts.actor value"),
    (r"^crates/pierre-cli/src/commands/deletion\.rs$", r"\"operator\"", "sealed deletion_receipts.actor value"),
    (r"^crates/pierre-server/tests/it/account_deletion_ledger_test\.rs$", r"'operator'", "sealed deletion_receipts.actor value"),
    # Migration: the refusal text an applied migration raises (immutable by
    # checksum), which the test asserts verbatim.
    (r"^crates/pierre-server/tests/it/athlete_home_activity_upload_test\.rs$", r"\"operator decision\"", "applied migration text"),
    # Not an admin: the SQL LIKE/ILIKE comparison.
    (r"^crates/pierre-database/src/", r"match operator", "SQL text-match operator"),
    # Not an admin: Rust expression operators in the code-shape normaliser.
    (r"^crates/pierre-sechoir/src/normalize\.rs$", r"(binary_|unary_)?operators?", "Rust syntax operators"),
    # Not an admin: the company that runs an OAuth client (Anthropic, OpenAI).
    (r"^crates/pierre-routes-identity/src/oauth2/consent_copy\.rs$", r"operators?", "OAuth client operator company"),
    # Not an admin: the ESTree AST property in ESLint selectors.
    (r"^packages/eslint-config-pierre/index\.js$", r"\.operator=", "ESTree AST property"),
    # Not an admin: JavaScript bitwise operators.
    (r"^packages/domain-utils/src/route-sketch\.ts$", r"bitwise operators", "JavaScript operators"),
    # Third party: the OpenStreetMap operator tag in recorded Overpass replies.
    (r"^crates/pierre-server/tests/fixtures/overpass/", r"\"operator\"", "OpenStreetMap tag"),
    # Satellite: the dravr-cageux progression value "variation operator",
    # recorded in the MCP surface snapshot.
    (r"^crates/pierre-server/tests/fixtures/mcp_surface/", r"\"const\": \"operator\"|variation operator", "dravr-cageux progression term"),
    # History: commit subjects the admin screen theory quotes verbatim, which
    # its test checks against git log (a commit message is never rewritten).
    (r"^frontend/src/components/admin/theory/screenTheory\.ts$", r"operator tier/plan|operators lose personal panes|every operator write", "quoted commit subjects"),
    # Third party: a PDF content operation's operator field (lopdf's
    # `Operation::operator`), read by the lab PDF child's form-draw check.
    (r"^crates/pierre-pdf-text/src/", r"operation\.operator", "lopdf content operation field"),
    # Third party: Babel plugin package names in the lockfiles.
    (r"(^|/)bun\.lock$", r"[a-z-]*operators?", "third-party package names"),
]
ALLOW = [(re.compile(p), re.compile(s), why) for p, s, why in ALLOW]
WORD = re.compile(r"operator", re.I)

findings = []
for raw in os.environ["HITS"].splitlines():
    path, _, rest = raw.partition(":")
    line_no, _, text = rest.partition(":")
    left = text
    for path_re, span_re, _why in ALLOW:
        if path_re.search(path):
            left = span_re.sub("", left)
    if WORD.search(left):
        findings.append(f"{path}:{line_no}: {text.strip()[:160]}")

for finding in findings:
    print(finding)
sys.exit(1 if findings else 0)
PY
)"; then
    echo -e "${RED}\"operator\" names an admin again. Say admin (carnet#1644):${NC}"
    echo "$OUT"
    echo ""
    echo "If the word is not about an admin (a language operator, a third party's"
    echo "field, a stored value), add an entry with its reason to the allowlist in"
    echo "scripts/ci/check-admin-vocabulary.sh."
    exit 1
fi

echo -e "${GREEN}No \"operator\" names an admin${NC}"
