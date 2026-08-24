#!/usr/bin/env bash
# ABOUTME: Fixture test for cargo-sweep-nightly.sh discovery — builds throwaway scan
# ABOUTME: roots and asserts which build trees are found, labelled, and addressed.
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# A repository can hold more than one build tree: `target/` plus whatever a side
# build pointed CARGO_TARGET_DIR at. Discovery matched the literal name `target`,
# so an alternate tree was never swept, never aged out, and never counted toward
# the fleet cap — invisible disk that only grew. These cases pin that it is found,
# that it carries its own label, and that cargo-sweep is pointed at the tree that
# was actually discovered rather than the manifest default.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UNDER_TEST="$SCRIPT_DIR/cargo-sweep-nightly.sh"

failures=0
pass() { echo "  ✅ $1"; }
fail() { echo "  ❌ $1"; failures=$((failures + 1)); }

[[ -x "$UNDER_TEST" ]] || { echo "not executable: $UNDER_TEST" >&2; exit 1; }

# A build tree as cargo leaves one, without paying for a compile. Discovery keys
# off CACHEDIR.TAG (or .rustc_info.json) plus a manifest beside the tree, so that
# is the whole shape a fixture needs.
make_tree() { # $1 = repo dir, $2 = target dir name
  mkdir -p "$1/$2/debug"
  printf 'Signature: 8a477f597d28d172789f06886806bc55\n' > "$1/$2/CACHEDIR.TAG"
  head -c 200000 /dev/zero > "$1/$2/debug/blob.bin"
}

make_repo() { # $1 = scan root, $2 = repo name
  mkdir -p "$1/$2"
  printf '[package]\nname = "%s"\nversion = "0.1.0"\nedition = "2021"\n' "$2" > "$1/$2/Cargo.toml"
  printf '%s\n' "$1/$2"
}

new_root() { mktemp -d "${TMPDIR:-/tmp}/cargo-sweep-test.XXXXXX"; }

# Labels as `status` prints them, one per line. The table runs from the line
# after the header to the rule above the fleet totals, and is empty when nothing
# was discovered — so the extraction has to stop at the rule, not run into the
# FLEET/CAP/HEADROOM footer below it.
labels_of() { # $1 = scan root
  # awk must drain its input rather than exit at the rule: closing the pipe
  # early sends SIGPIPE upstream, and pipefail would turn that into a failure.
  "$UNDER_TEST" status --root "$1" 2>/dev/null | awk '
    NR == 1 { next }
    /^-----/ { done = 1 }
    !done && NF { print $1 }
  '
}

echo "cargo-sweep-nightly.sh — discovery"

# An alternate tree alone must be found: before, `-name target` matched nothing
# here and the whole repository reported as having no build output at all.
root="$(new_root)"; repo="$(make_repo "$root" "alt-only")"
make_tree "$repo" "target-featurecheck"
got="$(labels_of "$root")"
if [[ "$got" == "alt-only[target-featurecheck]" ]]; then
  pass "an alternate build tree is discovered and labelled by its directory"
else
  fail "an alternate build tree is discovered and labelled by its directory (got: ${got:-<none>})"
fi
rm -rf "$root"

# The plain tree keeps its bare repository label, so protected lists and cap
# ledgers written against the old name still match.
root="$(new_root)"; repo="$(make_repo "$root" "plain-only")"
make_tree "$repo" "target"
got="$(labels_of "$root")"
if [[ "$got" == "plain-only" ]]; then
  pass "a plain target/ keeps the bare repository label"
else
  fail "a plain target/ keeps the bare repository label (got: ${got:-<none>})"
fi
rm -rf "$root"

# Two trees in one repository must not collapse into one entry: the label keys
# the protected list, the cap ledger and the wholesale-reclaim staging path.
root="$(new_root)"; repo="$(make_repo "$root" "both")"
make_tree "$repo" "target"
make_tree "$repo" "target-featurecheck"
got="$(labels_of "$root" | sort | tr '\n' ' ')"
if [[ "$got" == "both both[target-featurecheck] " ]]; then
  pass "two trees in one repository get distinct labels"
else
  fail "two trees in one repository get distinct labels (got: ${got:-<none>})"
fi
rm -rf "$root"

# Widening the glob must not hand a destructive tool a directory that merely
# starts with "target". No CACHEDIR.TAG means it is not build output.
root="$(new_root)"; repo="$(make_repo "$root" "decoy")"
mkdir -p "$repo/targets/fixtures"
head -c 200000 /dev/zero > "$repo/targets/fixtures/data.bin"
got="$(labels_of "$root")"
if [[ -z "$got" ]]; then
  pass "a targets/ directory that is not build output is left alone"
else
  fail "a targets/ directory that is not build output is left alone (got: $got)"
fi
rm -rf "$root"

# Build output with no manifest beside it stays out of the swept set: cargo-sweep
# resolves through a manifest and cannot act on it.
root="$(new_root)"
mkdir -p "$root/orphaned"
make_tree "$root/orphaned" "target-featurecheck"
got="$(labels_of "$root")"
report="$("$UNDER_TEST" status --root "$root" 2>&1 || true)"
if [[ -z "$got" ]] && printf '%s' "$report" | grep -q "^orphan .*orphaned:"; then
  pass "an alternate tree with no Cargo.toml is reported as orphan, not swept"
else
  fail "an alternate tree with no Cargo.toml is reported as orphan, not swept (got: ${got:-<none>})"
fi
rm -rf "$root"

echo "cargo-sweep-nightly.sh — sweep addressing"

# cargo-sweep resolves the build tree from the manifest, which finds `target/`
# and nothing else. Unless the discovered tree is named through CARGO_TARGET_DIR,
# an alternate tree is reported as swept while keeping every byte.
if ! command -v cargo >/dev/null 2>&1 || ! command -v cargo-sweep >/dev/null 2>&1; then
  fail "sweep addressing requires cargo and cargo-sweep on PATH (cargo install cargo-sweep)"
else
  root="$(new_root)"; repo="$(make_repo "$root" "addressed")"
  mkdir -p "$repo/src"
  echo 'fn main() {}' > "$repo/src/main.rs"
  ( cd "$repo" && CARGO_TARGET_DIR=target-featurecheck cargo build -q ) >/dev/null 2>&1
  out="$(CARGO_SWEEP_DAYS=0 "$UNDER_TEST" sweep --root "$root" --no-cap --dry-run 2>&1 || true)"
  if printf '%s' "$out" | grep -qF "$repo/target-featurecheck"; then
    pass "cargo-sweep is pointed at the discovered tree, not the manifest default"
  else
    fail "cargo-sweep is pointed at the discovered tree, not the manifest default"
  fi
  rm -rf "$root"
fi

echo ""
if [[ "$failures" -eq 0 ]]; then
  echo "✅ all cargo-sweep-nightly discovery cases passed"
else
  echo "❌ $failures case(s) failed"
fi
exit "$((failures > 0))"
