#!/usr/bin/env bash
# ABOUTME: Fixture suite for satellite-pin.sh — one fixture per pin shape the fleet actually uses
# ABOUTME: Half the cases assert a FAILURE; a guard that has never been made to fire is not tested

# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# The shapes here are not invented. Every one was read out of the platform's
# manifests on 2026-09-10: bare crates.io strings, crates.io tables with `optional`
# and with a `features` array, git+tag, git+rev, enforme's version+tag hybrid,
# equilibre-sync's package-first alias, and `{ workspace = true }` inheritance.
#
# The byte-comparison case is the important one. architectural-validation.sh:852
# reads dravr-canot's tag with a regex requiring `{ git = "...", tag = "..."` in that
# exact order; enforme puts `version` first and equilibre-sync puts `package` first.
# A rewriter that re-emits a line from parsed parts would normalise that order and
# break a CI check on a green local push. So the test asserts the rewritten line
# differs from the original in the version bytes and nowhere else.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PIN="${SCRIPT_DIR}/satellite-pin.sh"
PASS=0; FAIL=0
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

ok()   { PASS=$(( PASS + 1 )); echo "  ✅ $1"; }
bad()  { FAIL=$(( FAIL + 1 )); echo "  ❌ $1"; [ -n "${2:-}" ] && echo "     $2"; }

# A fixture repo: satellites.toml + the manifests + a lockfile.
fixture() { # $1=name -> prints root
  local root="${WORK}/$1"
  rm -rf "$root"; mkdir -p "$root/crates/alpha" "$root/crates/beta"

  cat > "$root/satellites.toml" <<'TOML'
[gates]
backend  = "ci-backend.yml ci-postgres.yml"
frontend = "frontend-tests.yml"

[embacle]
repo   = "dravr-embacle"
source = "crates-io"
crates = "embacle embacle-tool-host"

[dravr-cageux]
repo   = "dravr-cageux"
source = "git-tag"

[dravr-canot]
repo   = "dravr-canot"
source = "git-tag"

[dravr-enforme]
repo   = "dravr-enforme"
source = "git-tag+version"

[dravr-equilibre]
repo   = "dravr-equilibre"
source = "git-tag"

[dravr-contremaitre]
repo   = "dravr-contremaitre"
source = "git-rev"

[photograveur]
repo   = "dravr-photograveur"
source = "git-tag"
gates  = "backend frontend"
TOML

  cat > "$root/Cargo.toml" <<'TOML'
[workspace.dependencies]
embacle-tool-host = "0.26.0"
dravr-cageux = { git = "https://github.com/dravr-ai/dravr-cageux.git", tag = "v0.18.0" }
dravr-canot = { git = "https://github.com/dravr-ai/dravr-canot.git", tag = "v0.4.28" }
dravr-contremaitre = { git = "https://github.com/dravr-ai/dravr-contremaitre.git", rev = "6d7d4e4c840edb4dfa6adac1d942fb6dc0872131" }
dravr-enforme = { version = "0.1.56", git = "https://github.com/dravr-ai/dravr-enforme.git", tag = "v0.1.56", default-features = false }
dravr-equilibre = { git = "https://github.com/dravr-ai/dravr-equilibre.git", tag = "v0.2.8" }
dravr-equilibre-sync = { package = "dravr-equilibre", git = "https://github.com/dravr-ai/dravr-equilibre.git", tag = "v0.2.8" }
photograveur = { git = "https://github.com/dravr-ai/dravr-photograveur.git", tag = "v0.3.1" }
TOML

  cat > "$root/crates/alpha/Cargo.toml" <<'TOML'
[dependencies]
embacle = { version = "0.26.0", optional = true }
dravr-cageux = { workspace = true }
TOML

  cat > "$root/crates/beta/Cargo.toml" <<'TOML'
[dependencies]
embacle = { version = "0.26.0", features = ["copilot-headless", "openai-api"] }
embacle-tool-host = "0.26.0"
dravr-canot = { workspace = true }
TOML

  cat > "$root/Cargo.lock" <<'TOML'
[[package]]
name = "embacle"
version = "0.26.0"

[[package]]
name = "embacle-tool-host"
version = "0.26.0"

[[package]]
name = "dravr-cageux"
version = "0.18.0"
source = "git+https://github.com/dravr-ai/dravr-cageux.git?tag=v0.18.0#abc1234"
TOML
  printf '%s' "$root"
}

run() { SATELLITE_PIN_ROOT="$1" "$PIN" "${@:2}" 2>&1; }

# Never pipe run() into grep. `set -o pipefail` would then hand the pipeline
# run()'s exit status, and every refusal case — where run() exits non-zero BY
# DESIGN — would read as a failed assertion even with the pattern matched. Capture
# first, judge second: the same rule CLAUDE.md states for CI gates.
says() { # $1=output $2=pattern
  printf '%s' "$1" | grep -q "$2"
}

echo "==== satellite-pin.sh fixtures ===="

# --------------------------------------------------------------------------
echo "-- shape: crates.io, three sites, two crates, one version"
R=$(fixture crates-io)
if run "$R" rewrite embacle 0.27.0 >/dev/null && run "$R" verify embacle 0.27.0 >/dev/null; then
  n=$(grep -c '0\.27\.0' "$R/Cargo.toml" "$R/crates/alpha/Cargo.toml" "$R/crates/beta/Cargo.toml" | awk -F: '{s+=$2} END {print s}')
  [ "$n" -eq 4 ] && ok "all 4 crates.io sites moved" || bad "expected 4 sites at 0.27.0, got $n"
  grep -q 'optional = true' "$R/crates/alpha/Cargo.toml" && ok "optional = true survived" || bad "lost optional = true"
  grep -q 'features = \["copilot-headless", "openai-api"\]' "$R/crates/beta/Cargo.toml" \
    && ok "features array survived" || bad "features array mangled"
  grep -q '^embacle-tool-host = "0.27.0"$' "$R/Cargo.toml" && ok "bare-string form moved" || bad "bare string not moved"
else
  bad "crates.io rewrite/verify failed" "$(run "$R" verify embacle 0.27.0)"
fi

# --------------------------------------------------------------------------
echo "-- shape: git+tag, and the byte-preservation rule"
R=$(fixture git-tag)
BEFORE=$(grep '^dravr-canot = ' "$R/Cargo.toml")
run "$R" rewrite dravr-canot v0.4.29 >/dev/null
AFTER=$(grep '^dravr-canot = ' "$R/Cargo.toml")
EXPECT=${BEFORE//v0.4.28/v0.4.29}
[ "$AFTER" = "$EXPECT" ] && ok "line changed in the version bytes and nowhere else" \
  || bad "key order or spacing changed" "before: $BEFORE
     after:  $AFTER"
# The canot regex in architectural-validation.sh must still match.
echo "$AFTER" | grep -qE 'dravr-canot = \{ git = "[^"]+", tag = "v[0-9.]+"' \
  && ok "architectural-validation's canot regex still matches" || bad "canot regex broken by rewrite"
run "$R" verify dravr-canot v0.4.29 >/dev/null && ok "verify accepts the moved tag" || bad "verify rejected a good tree"

# --------------------------------------------------------------------------
echo "-- shape: git-tag+version hybrid, both halves must move together"
R=$(fixture hybrid)
run "$R" rewrite dravr-enforme v0.1.57 >/dev/null
L=$(grep '^dravr-enforme = ' "$R/Cargo.toml")
[ "$(printf '%s' "$L" | grep -o '0\.1\.57' | wc -l | tr -d ' ')" -eq 2 ] \
  && ok "version and tag both moved" || bad "hybrid moved only one half" "$L"
printf '%s' "$L" | grep -q 'default-features = false' && ok "trailing keys survived" || bad "lost default-features"
printf '%s' "$L" | grep -qE '^dravr-enforme = \{ version = ' && ok "version-first key order preserved" || bad "key order normalised"
run "$R" verify dravr-enforme v0.1.57 >/dev/null && ok "verify accepts the agreeing pair" || bad "verify rejected an agreeing pair"
# Now break the agreement by hand and prove verify catches it.
perl -i -pe 's/tag = "v0\.1\.57"/tag = "v0.1.56"/' "$R/Cargo.toml"
run "$R" verify dravr-enforme v0.1.57 >/dev/null 2>&1 \
  && bad "verify PASSED a version/tag disagreement" || ok "verify fails when version and tag disagree"

# --------------------------------------------------------------------------
echo "-- shape: package-first alias (carnet#323)"
R=$(fixture alias)
run "$R" rewrite dravr-equilibre v0.2.9 >/dev/null
a=$(grep -c 'tag = "v0.2.9"' "$R/Cargo.toml")
[ "$a" -eq 2 ] && ok "both the direct pin and the -sync alias moved" || bad "alias missed — carnet#323 shape, got $a of 2"
grep -q '^dravr-equilibre-sync = { package = "dravr-equilibre", git' "$R/Cargo.toml" \
  && ok "package-first key order preserved" || bad "alias line key order normalised"

# --------------------------------------------------------------------------
echo "-- shape: git+rev"
R=$(fixture git-rev)
NEWREV=0123456789abcdef0123456789abcdef01234567
run "$R" rewrite dravr-contremaitre "$NEWREV" >/dev/null
grep -q "rev = \"${NEWREV}\"" "$R/Cargo.toml" && ok "rev moved" || bad "rev not moved"
run "$R" rewrite dravr-contremaitre v1.2.3 >/dev/null 2>&1 \
  && bad "accepted a version where a sha was required" || ok "rejects a non-sha rev target"

# --------------------------------------------------------------------------
echo "-- shape: workspace = true inherits, and is never itself a pin"
R=$(fixture inherit)
run "$R" rewrite dravr-cageux v0.19.0 >/dev/null
grep -q '^dravr-cageux = { workspace = true }$' "$R/crates/alpha/Cargo.toml" \
  && ok "inherited site left untouched" || bad "rewrote a workspace = true line"
OUT=$(run "$R" verify dravr-cageux v0.19.0)
says "$OUT" '1 pin(s) at v0.19.0, 1 inherited' && ok "counted 1 pin + 1 inherited" || bad "miscounted" "$OUT"
# A tree with ONLY inherited sites is the photograveur failure: exit 2, never a pass.
R=$(fixture only-inherited)
perl -i -ne 'print unless /^dravr-cageux = \{ git/' "$R/Cargo.toml"
run "$R" verify dravr-cageux v0.19.0 >/dev/null 2>&1
[ $? -eq 2 ] && ok "all-inherited tree exits 2, not 0" || bad "an all-inherited tree did not exit 2"

# --------------------------------------------------------------------------
echo "-- refusals: shapes the rewriter must not silently accept"
R=$(fixture two-component)
perl -i -pe 's/tag = "v0\.18\.0"/tag = "v0.18"/ if /^dravr-cageux = \{ git/' "$R/Cargo.toml"
run "$R" verify dravr-cageux v0.19.0 >/dev/null 2>&1 \
  && bad "verify PASSED a two-component version" || ok "verify rejects a two-component version"

R=$(fixture path-override)
perl -i -pe 's|^dravr-cageux = .*|dravr-cageux = { path = "../dravr-cageux" }|' "$R/Cargo.toml"
run "$R" verify dravr-cageux v0.19.0 >/dev/null 2>&1 \
  && bad "verify PASSED a path override" || ok "verify rejects a path override"

R=$(fixture partial)
run "$R" rewrite embacle 0.27.0 >/dev/null
perl -i -pe 's/0\.27\.0/0.26.0/' "$R/crates/beta/Cargo.toml"     # one site falls back
run "$R" verify embacle 0.27.0 >/dev/null 2>&1 \
  && bad "verify PASSED a partial bump" || ok "verify rejects a partial bump"

# --------------------------------------------------------------------------
echo "-- lockfile assertions"
R=$(fixture lock)
run "$R" lock-assert embacle 0.26.0 >/dev/null && ok "lock-assert passes on a matching lockfile" || bad "lock-assert rejected a good lockfile"
run "$R" lock-assert embacle 0.27.0 >/dev/null 2>&1 \
  && bad "lock-assert PASSED a stale lockfile" || ok "lock-assert fails when the lockfile did not move"
run "$R" lock-assert dravr-cageux v0.18.0 >/dev/null && ok "lock-assert reads a git+tag source" || bad "lock-assert missed a git+tag source"
run "$R" lock-assert dravr-cageux v0.19.0 >/dev/null 2>&1 \
  && bad "lock-assert PASSED a stale git+tag source" || ok "lock-assert fails on a stale git+tag source"

# --------------------------------------------------------------------------
echo "-- declaration self-policing"
R=$(fixture toml-guards)
printf '\n[dravr-typo]\nrepo = "dravr-typo"\nsource = "git-tag"\ngatez = "backend"\n' >> "$R/satellites.toml"
OUT=$(run "$R" resolve dravr-typo)
says "$OUT" "unknown key 'gatez'" && ok "an unknown key is an error, not a default" || bad "unknown key silently ignored" "$OUT"
R=$(fixture gates-guards)
printf '\n[dravr-bad]\nrepo = "dravr-bad"\nsource = "git-tag"\ngates = "nosuchalias"\n' >> "$R/satellites.toml"
OUT=$(run "$R" gates dravr-bad)
says "$OUT" "not defined under .gates." && ok "an undefined gates alias is an error" || bad "undefined gates alias accepted" "$OUT"
R=$(fixture gates-ok)
[ "$(run "$R" gates photograveur)" = "ci-backend.yml ci-postgres.yml frontend-tests.yml" ] \
  && ok "gates aliases resolve and dedupe" || bad "gates resolution wrong: $(run "$R" gates photograveur)"
[ "$(run "$R" gates dravr-cageux)" = "ci-backend.yml ci-postgres.yml" ] \
  && ok "gates defaults to backend" || bad "gates default wrong"
[ "$(run "$R" crates dravr-cageux)" = "dravr-cageux" ] && ok "crates defaults to the stanza name" || bad "crates default wrong"
OUT=$(run "$R" resolve dravr-nonexistent)
says "$OUT" "no stanza for" && ok "an unknown satellite is an error" || bad "unknown satellite accepted" "$OUT"

# --------------------------------------------------------------------------
echo "-- lane ownership is derived from the callers, never declared"
R=$(fixture lanes)
mkdir -p "$R/.github/workflows"
cat > "$R/.github/workflows/bump-cageux.yml" <<'WF'
jobs:
  bump:
    uses: ./.github/workflows/satellite-bump.yml
    with:
      satellite: dravr-cageux
WF
[ "$(run "$R" lane dravr-cageux)" = "bump-cageux.yml" ] \
  && ok "a caller naming the satellite is its lane" || bad "did not derive the lane from the caller"
[ -z "$(run "$R" lane dravr-canot)" ] \
  && ok "a satellite with no caller has no lane" || bad "invented a lane for a satellite with no caller"
# A declared lane must never override the derivation: that is how the stripe table lied.
printf '\n[dravr-canot]\nrepo = "dravr-canot"\nsource = "git-tag"\nlane = "some-other-chain.yml"\n' >> "$R/satellites.toml"
[ "$(run "$R" lane dravr-canot)" = "some-other-chain.yml" ] \
  && ok "a declared lane still answers for a chain that is not the spine" || bad "declared lane ignored"
cat > "$R/.github/workflows/bump-canot.yml" <<'WF'
jobs:
  bump:
    uses: ./.github/workflows/satellite-bump.yml
    with:
      satellite: dravr-canot
WF
[ "$(run "$R" lane dravr-canot)" = "bump-canot.yml" ] \
  && ok "a real caller wins over a declared claim" || bad "a stale declaration outranked a live caller"

echo
echo "==== ${PASS} passed, ${FAIL} failed ===="
[ "$FAIL" -eq 0 ] || exit 1
