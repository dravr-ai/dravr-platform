#!/usr/bin/env bash
# ABOUTME: Fixture test for check-rust-inputs.py — a throwaway tree with the real filter and triggers, one crate and its reads
# ABOUTME: Pins that the real tree passes, an uncovered read or an unreachable glob fails by name, and an empty scan fails closed
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
UNDER_TEST="${UNDER_TEST:-$SCRIPT_DIR/check-rust-inputs.py}"
ACTION=".github/actions/ci-backend-changes/action.yml"
WORKFLOW=".github/workflows/ci-backend.yml"
WORK="$(mktemp -d)"
OUT="$WORK/out"
trap 'rm -rf "$WORK"' EXIT

failures=0
pass() { echo "  ✅ $1"; }
fail() { echo "  ❌ $1"; failures=$((failures + 1)); }

run_check() { # $1 = repo root; prints the exit status, output lands in $OUT
  local code=0
  python3 "$UNDER_TEST" "$1" >"$OUT" 2>&1 || code=$?
  echo "$code"
}

expect() { # $1 = label, $2 = actual exit, $3 = expected exit
  if [ "$2" = "$3" ]; then pass "$1"; else
    fail "$1 (exit $2, expected $3)"
    sed 's/^/      /' "$OUT"
  fi
}

has() { # $1 = label, $2 = fixed string the output must contain
  if grep -qF -- "$2" "$OUT"; then pass "$1"; else
    fail "$1"
    sed 's/^/      /' "$OUT"
  fi
}

# A tree with the real filter and one crate whose source reads a locale
# through include_str! (file-relative), the migrations through sqlx::migrate!
# (crate-relative) and the SDK through a join chain.
fixture() {
  rm -rf "$WORK/tree"
  local t="$WORK/tree"
  mkdir -p "$t/$(dirname "$ACTION")" "$t/$(dirname "$WORKFLOW")" "$t/crates/demo/src" "$t/packages/i18n/src" "$t/migrations" "$t/sdk/src" "$t/docs"
  cp "$REPO_ROOT/$ACTION" "$t/$ACTION"
  cp "$REPO_ROOT/$WORKFLOW" "$t/$WORKFLOW"
  printf '[package]\nname = "demo"\n' > "$t/crates/demo/Cargo.toml"
  echo '{}' > "$t/packages/i18n/src/en.json"
  echo 'select 1;' > "$t/migrations/0001_init.sql"
  echo 'export {}' > "$t/sdk/src/types.ts"
  echo '# guide' > "$t/docs/guide.md"
  cat > "$t/crates/demo/src/lib.rs" <<'RS'
const EN: &str = include_str!("../../../packages/i18n/src/en.json");
static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");
fn sdk(root: &Path) -> PathBuf {
    root.join("sdk")
        .join("src")
        .join("types.ts")
}
// A literal that resolves to nothing on disk is not a read:
const ATTACK: &str = "../../../../etc/passwd-not-here";
RS
}

echo "the real tree"
expect "every read in this repository is covered" "$(run_check "$REPO_ROOT")" 0

echo "a covered fixture"
fixture
expect "include_str!, sqlx::migrate! and a join chain are all covered" "$(run_check "$WORK/tree")" 0
has "it counts the reads it judged" "3 reads of files outside crates/"

echo "an uncovered read"
fixture
echo 'const GUIDE: &str = include_str!("../../../docs/guide.md");' >> "$WORK/tree/crates/demo/src/lib.rs"
expect "a read the filter misses fails" "$(run_check "$WORK/tree")" 1
has "it names the path" "docs/guide.md"
has "it names where it is read" "crates/demo/src/lib.rs:10"

echo "a repo-root literal"
fixture
echo 'fn conf() -> String { read("docs/guide.md") }' >> "$WORK/tree/crates/demo/src/lib.rs"
expect "a repo-root path read through a helper is judged too" "$(run_check "$WORK/tree")" 1

echo "a glob the workflow's triggers never reach"
fixture
grep -v "^      - 'commands/\*\*'$" "$WORK/tree/$WORKFLOW" > "$WORK/wf" && mv "$WORK/wf" "$WORK/tree/$WORKFLOW"
expect "a rust glob no push path starts a run for fails" "$(run_check "$WORK/tree")" 1
has "it names the glob and the event" "commands/** (push)"

echo "scans that verified nothing"
fixture
echo 'pub fn nothing() {}' > "$WORK/tree/crates/demo/src/lib.rs"
expect "no read at all fails closed" "$(run_check "$WORK/tree")" 2
has "it says why" "verified nothing"
fixture
printf 'runs:\n  using: composite\n  steps: []\n' > "$WORK/tree/$ACTION"
expect "a filter it cannot find fails closed" "$(run_check "$WORK/tree")" 2
fixture
rm "$WORK/tree/$WORKFLOW"
expect "triggers it cannot read fail closed" "$(run_check "$WORK/tree")" 2

echo
if [ "$failures" -eq 0 ]; then
  echo "all check-rust-inputs cases pass"
  exit 0
fi
echo "$failures case(s) failed"
exit 1
