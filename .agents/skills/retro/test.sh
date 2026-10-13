#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: Tests for retro.sh — a throwaway repo, a carnet snapshot, a vault and a transcript per run
# ABOUTME: No network: run `bash .agents/skills/retro/test.sh` from anywhere
set -uo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
RETRO="$HERE/retro.sh"
PASS=0
FAIL=0

ok()  { printf '  ✅ %s\n' "$1"; PASS=$((PASS + 1)); }
bad() { printf '  ❌ %s\n' "$1"; FAIL=$((FAIL + 1)); }
die() { printf '\n  💥 %s\n\n' "$1" >&2; exit 2; }

# An explicit X template: `mktemp -d -t prefix` is BSD-only and returns nothing on GNU (see
# bilan/test.sh, where an empty fixture path once pointed a suite at the real checkout).
T=$(mktemp -d "${TMPDIR:-/tmp}/retro-test.XXXXXX") || die "mktemp failed"
[ -n "$T" ] && [ -d "$T" ] || die "fixture root is empty — refusing to run"
trap 'rm -rf "$T"' EXIT

# A fake HOME so the default transcript scan never reads the developer's real sessions.
export HOME="$T/home"
mkdir -p "$HOME"
R="$T/ws/dravr-platform"
mkdir -p "$R"

expect() { # <label> <fixed string> <file>
    if grep -q -F -- "$2" "$3"; then ok "$1"; else bad "$1 — missing: $2"; fi
}
exits_2() { # <rc> <label>
    if [ "$1" = 2 ]; then ok "$2"; else bad "$2 (rc=$1)"; fi
}
refuse() { # <label> <fixed string> <file>
    if grep -q -F -- "$2" "$3"; then bad "$1 — unexpected: $2"; else ok "$1"; fi
}

# ------------------------------------------------------------------ fixtures
(
    cd "$R" || exit 1
    git init -q -b main . && git config user.email t@example.com && git config user.name t
    git config commit.gpgsign false
    mkdir -p scripts/ci .claude/skills/lonely .claude/skills/named docs
    # shellcheck disable=SC2016  # the literal default the budget gate spells
    printf 'BUDGET="${AGENTS_MD_BUDGET:-1000}"\n' > scripts/ci/check-agents-md-budget.sh
    printf '#!/bin/sh\n' > scripts/ci/check-existing.sh
    printf 'Use the named skill.\n' > AGENTS.md
    printf '# Coding standards\n## Tests\n## Comments\n' > docs/coding-standards.md
    printf -- '---\nname: lonely\n---\n' > .claude/skills/lonely/SKILL.md
    printf -- '---\nname: named\n---\n' > .claude/skills/named/SKILL.md
    printf 'tracker = "acme/register"\n' > registre.toml
    git add -A && git commit -q -m "chore: root"                       # parentless: never counted
    printf 'a\n' > a.rs && git add a.rs && git commit -q -m "feat(x): first thing (carnet#7)"
    printf 'b\n' >> a.rs && git add a.rs && git commit -q -m "fix(x): mend it (carnet#7)"
    printf 'c\n' >> a.rs && git add a.rs && git commit -q -m "fix(x): mend it again (carnet#7)

Reviewed-Standards: abc123"
    git tag pre-review                                                 # no `covers` trailer yet
    first=$(git log --format=%h --grep='first thing')
    printf 'd\n' >> a.rs && git add a.rs && git commit -q -m "review: tidy the first thing

Reviewed-Standards: abc123 covers $first"
    printf 'e\n' >> a.rs && git add a.rs && git commit -q -m "fix(y): unreviewed landing"
    printf '#!/bin/sh\n' > scripts/ci/check-new-thing.sh && git add -A && git commit -q -m "ci: a new gate"
) || die "fixture repo failed"
[ -d "$R/.git" ] || die "fixture repo is not a git repository — refusing to run"

SNAP="$T/snap"
mkdir -p "$SNAP"
cat > "$SNAP/issues.json" <<'EOF'
[
  {"number": 7, "title": "[platform] Login stalls on the second submit", "state": "open", "created_at": "2099-01-01T00:00:00Z", "closed_at": null, "labels": [{"name": "bug"}]},
  {"number": 8, "title": "[platform] Login stalls behind the token wait", "state": "closed", "created_at": "2099-01-01T00:00:00Z", "closed_at": "2099-01-02T00:00:00Z", "labels": [{"name": "bug"}]},
  {"number": 9, "title": "[platform] scheduled lane unhealthy: CI: Benchmarks login", "state": "open", "created_at": "2099-01-01T00:00:00Z", "closed_at": null, "labels": []},
  {"number": 10, "title": "a pull request", "state": "open", "created_at": "2099-01-01T00:00:00Z", "closed_at": null, "labels": [], "pull_request": {}}
]
EOF
cat > "$SNAP/comments.json" <<'EOF'
[
  {"id": 1, "issue_url": "https://api.github.com/repos/acme/register/issues/660", "created_at": "2099-01-01T00:00:00Z",
   "body": "**review-standards run — slice one**\n\nFindings by standards section:\n- **Comments / evergreen prose (1):** a history comment.\n- **Single source (1):** a duplicated literal.\n- **Checked, nothing found:** error handling."},
  {"id": 2, "issue_url": "https://api.github.com/repos/acme/register/issues/660", "created_at": "2099-01-01T00:00:00Z",
   "body": "review-standards run — slice two\n\n| Section | Findings |\n|---|---|\n| Comments, naming | 2 (stale docs) |\n| Error handling | 0 |\n| Weird new label | 1 |"},
  {"id": 3, "issue_url": "https://api.github.com/repos/acme/register/issues/12", "created_at": "2099-01-01T00:00:00Z",
   "body": "**Directive from JF (2099-01-01):** ship the strict form."},
  {"id": 4, "issue_url": "https://api.github.com/repos/acme/register/issues/660", "created_at": "1999-01-01T00:00:00Z",
   "body": "review-standards run — too old\n- **Security (1):** outside the window."}
]
EOF

VAULT="$T/vault"
mkdir -p "$VAULT/Work Log/2099-01"
cat > "$VAULT/Work Log/2099-01/A Post-mortem.md" <<'EOF'
---
type: worklog
kind: post-mortem
area: infra
status: completed
date: 2099-01-01
---

# A Post-mortem

## Lessons applied
- Close only after the deployed page is seen.
EOF
cat > "$VAULT/Work Log/2099-01/Old Note.md" <<'EOF'
---
type: worklog
kind: plan
date: 1999-01-01
---
EOF

TR="$T/transcripts"
mkdir -p "$TR"
cat > "$TR/s1.jsonl" <<'EOF'
{"type":"assistant","message":{"content":[{"type":"tool_use","id":"u1","name":"Grep","input":{"pattern":"NoSuchSymbol"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"u1","content":"No files found"}]}}
{"type":"assistant","message":{"content":[{"type":"tool_use","id":"u2","name":"Bash","input":{"command":"cd /x && cargo test -p foo"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"u2","content":"boom","is_error":true}]}}
{"type":"assistant","message":{"content":[{"type":"tool_use","id":"u3","name":"Skill","input":{"skill":"named"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"u3","content":"loaded"}]}}
EOF

run() { # <out> <args…>
    local out=$1; shift
    ( cd "$R" && bash "$RETRO" "$@" ) > "$out" 2> "$out.err"
}

# ------------------------------------------------------------------ cases
printf '\nretro.sh\n'

run "$T/d1" --since 2000-01-01 --ref main --carnet-from "$SNAP" --vault "$VAULT" --transcripts "$TR"
rc=$?
if [ "$rc" = 0 ]; then ok "a full offline run exits 0"; else bad "a full offline run exits 0 (rc=$rc: $(cat "$T/d1.err"))"; fi

expect "the parentless root commit is not counted" "— 6 (tip" "$T/d1"
expect "commit types are tallied" "fix 3" "$T/d1"
expect "only the two uncovered landings are unreviewed" "2 total" "$T/d1"
expect "the unreviewed fix is named" "fix(y): unreviewed landing" "$T/d1"
refuse "the covered feat is not named unreviewed" "feat(x): first thing" "$T/d1"
expect "review: commits are listed" "review: tidy the first thing" "$T/d1"
expect "a carnet issue in 3 commits is churn" "carnet#7 ×3" "$T/d1"
expect "a file fixed 3 times is a hotspot" "a.rs ×3" "$T/d1"

expect "two runs hitting comments make it caught twice" "| comments-true | 2 | 2 | **yes** |" "$T/d1"
expect "one run hitting single source is not caught twice" "| single-source | 1 | 1 |  |" "$T/d1"
refuse "a zero-count table row is not a finding" "| error-handling" "$T/d1"
refuse "a comment older than the window is not read" "outside the window" "$T/d1"
expect "an unmatched label is surfaced for the theme table" "- weird new label" "$T/d1"
expect "the evidence quotes the recorded row" "a history comment." "$T/d1"

expect "a new gate file is reported" "added scripts/ci/check-new-thing.sh" "$T/d1"
expect "existing gates are listed for de-duplication" "existing new-thing" "$T/d1"
expect "AGENTS.md is measured against its budget" "AGENTS.md: 21 of 1000 bytes" "$T/d1"
expect "a skill named nowhere is a cut candidate" "lonely (" "$T/d1"
refuse "a skill AGENTS.md names is not a cut candidate" "named (" "$T/d1"

expect "pull requests are not counted as issues" "Created: 3 · closed: 1" "$T/d1"
expect "a title term in 3 issues recurs" "login ×3" "$T/d1"
expect "a CI-red issue is listed" "#9 [open]" "$T/d1"
expect "a JF directive is quoted" "#12 2099-01-01 — Directive from JF (2099-01-01): ship the strict form." "$T/d1"

expect "vault notes in the window are counted" "1 notes. By kind: post-mortem 1" "$T/d1"
expect "a lessons bullet is quoted" "(A Post-mortem) Close only after the deployed page is seen." "$T/d1"

expect "transcripts are read" "1 transcript files" "$T/d1"
expect "a tool error is counted" "| Bash | 1 | 1 |" "$T/d1"
expect "the errored command is named" "1 Bash cargo test -p" "$T/d1"
expect "a leading cd is dropped from the Bash key" "Bash cargo test -p" "$T/d1"
expect "an empty search is a navigation hint" "Grep NoSuchSymbol" "$T/d1"
expect "a skill invocation is counted" "named ×1" "$T/d1"

run "$T/d2" --since 2000-01-01 --ref main --carnet-from "$SNAP" --vault "$VAULT" --transcripts "$TR"
if cmp -s "$T/d1" "$T/d2"; then ok "two runs over the same inputs print the same digest"; else bad "the digest is not deterministic"; fi

run "$T/d3" --since 2000-01-01 --ref main --offline --vault "$T/nowhere" --transcripts "$TR"
expect "--offline without a snapshot says why carnet is missing" "(carnet unavailable: --offline and no --carnet-from)" "$T/d3"
expect "a missing vault says so" "(vault not found at $T/nowhere" "$T/d3"

# A window with no `covers` trailer leaves the covered list empty, and every untrailed landing
# in it is unreviewed: the feat that a later trailer covers on main is not covered yet here.
run "$T/d6" --since 2000-01-01 --ref pre-review --offline
expect "with no covers trailer, both untrailed landings are unreviewed" "2 total" "$T/d6"
expect "with no covers trailer, the feat is named unreviewed" "feat(x): first thing" "$T/d6"

# The tracker over a stub gh: `gh api <path>` serves STUB_ISSUE_PAGES full pages of 100 issues,
# then a page of one, and STUB_COMMENT_PAGES full pages of 100 comments, then an empty page.
STUB="$T/bin"
mkdir -p "$STUB"
cat > "$STUB/gh" <<'EOF'
#!/usr/bin/env bash
page=${2##*page=}
case $2 in
    *issues/comments*) n=0; [ "$page" -le "${STUB_COMMENT_PAGES:-0}" ] && n=100
       jq -n --argjson n "$n" --argjson p "$page" \
           '[range($n) | {id: ($p * 1000 + .), issue_url: "https://api.github.com/repos/acme/register/issues/1", created_at: "2099-01-01T00:00:00Z", body: "stub"}]' ;;
    *) n=1; [ "$page" -le "${STUB_ISSUE_PAGES:-0}" ] && n=100
       jq -n --argjson n "$n" --argjson p "$page" \
           '[range($n) | {number: ($p * 1000 + .), title: "[platform] stub issue", state: "open", created_at: "2099-01-01T00:00:00Z", closed_at: null, labels: []}]' ;;
esac
EOF
chmod +x "$STUB/gh"
( export PATH="$STUB:$PATH" STUB_ISSUE_PAGES=1; run "$T/d7" --since 2000-01-01 --ref main --save-carnet "$T/saved" )
expect "the tracker is read past its first page of 100" "Created: 101 ·" "$T/d7"
if [ "$(jq length "$T/saved/issues.json" 2>/dev/null)" = 101 ]; then ok "--save-carnet writes every page it read"; else bad "--save-carnet writes every page it read"; fi
( export PATH="$STUB:$PATH" STUB_COMMENT_PAGES=99; run "$T/d8" --since 2000-01-01 --ref main )
expect "a window past the page cap says so instead of truncating" "(carnet unavailable: more than 2000 records at repos/acme/register/issues/comments, narrow the window;" "$T/d8"
expect "the issues that were read are still reported" "Created: 1 ·" "$T/d8"
expect "the directives list says the comments are missing rather than listing none" "(comments unavailable: more than 2000 records" "$T/d8"

run "$T/d4" --since yesterday --ref main
exits_2 $? "a malformed --since exits 2"
run "$T/d5" --ref no-such-ref --offline
exits_2 $? "an unknown ref exits 2"
( cd "$T" && bash "$RETRO" --offline ) > /dev/null 2>&1
exits_2 $? "outside a git checkout exits 2"

printf '\n%s passed, %s failed\n' "$PASS" "$FAIL"
[ "$FAIL" = 0 ]
