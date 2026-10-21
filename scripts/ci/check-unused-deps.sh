#!/usr/bin/env bash
# ABOUTME: Fails when a workspace crate declares a dependency its code never names, or a normal dependency
# ABOUTME: only its tests use — cargo-shear over every member, failing closed on a missing tool or an empty scan
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Why this exists:
#   The coding standards forbid a phantom dependency ("if I deleted this
#   dependency line, what breaks?"), and nothing checked it. The first run of
#   this gate found 87 issues across 24 crates: 68 dependencies nothing
#   named, 17 normal ones only tests used, one only a feature forwarded and
#   one unlinked file. Among them a redis client, an RSA stack, a cron parser
#   and a dozen pierre-* edges left behind by moves. cargo-shear parses
#   every target's source (lib, bins, tests, benches, examples) without
#   compiling, regardless of `cfg`, so a feature-gated use still counts.
#
# What fails (cargo-shear runs with --deny-warnings, so warnings fail too):
#   - a dependency no target of its crate names (unused_dependency)
#   - an optional dependency nothing names (unused_optional_dependency)
#   - a normal dependency only tests/benches name (misplaced_dependency):
#     move it to [dev-dependencies]
#   - a `[workspace.dependencies]` entry no member inherits
#   - a source file no module tree reaches (unlinked_files)
#
# A dependency that IS needed but is never named in source — a feature-
# unification edge, a crate only a macro expansion names — is ignored per
# crate, with the reason on the line above it:
#     # <one line: why nothing names it>
#     [package.metadata.cargo-shear]
#     ignored = ["crate-name"]
#   Never in [workspace.metadata]: a blanket ignore hides the next real one.
#   cargo-shear reports an ignore that suppresses nothing, so a stale one fails.
#
# Exit 0  every member scanned, no finding
# Exit 1  a finding; cargo-shear missing, the wrong version, crashed or
#         printed no report; or the workspace has no member to scan — a
#         scan that verified nothing must not report success
#
# Usage: check-unused-deps.sh [WORKSPACE_ROOT]
#   CARGO_SHEAR  the cargo-shear binary to run (default: cargo-shear on PATH)
set -euo pipefail

# The one pin. ci-backend.yml's fast gate installs exactly this version, and
# check-unused-deps.test.sh fails when the two disagree: another release can
# judge the same tree differently.
CARGO_SHEAR_VERSION="1.14.0"

if [[ "${1:-}" == "--pinned-version" ]]; then
    echo "$CARGO_SHEAR_VERSION"
    exit 0
fi

ROOT="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
SHEAR="${CARGO_SHEAR:-cargo-shear}"
INSTALL_HINT="cargo install --locked cargo-shear@${CARGO_SHEAR_VERSION}"

if [[ ! -f "$ROOT/Cargo.toml" ]]; then
    echo "❌ check-unused-deps: no Cargo.toml under $ROOT; the check verified nothing"
    exit 1
fi

if ! command -v "$SHEAR" >/dev/null 2>&1; then
    echo "❌ check-unused-deps: cargo-shear is not installed; the check verified nothing"
    echo "   Install it: $INSTALL_HINT"
    exit 1
fi

if ! version_out="$("$SHEAR" shear --version 2>&1)"; then
    echo "❌ check-unused-deps: '$SHEAR shear --version' failed:"
    printf '%s\n' "$version_out" | sed 's/^/   /'
    exit 1
fi
if ! grep -qE "(^|[^0-9.])${CARGO_SHEAR_VERSION//./\\.}([^0-9.]|$)" <<< "$version_out"; then
    echo "❌ check-unused-deps: cargo-shear ${CARGO_SHEAR_VERSION} is required, found: $(head -1 <<< "$version_out")"
    echo "   Install it: $INSTALL_HINT"
    exit 1
fi

# The members the scan must cover, counted independently of cargo-shear: a
# workspace whose glob matches nothing would otherwise scan nothing and pass.
# stderr apart from stdout: a cargo warning mixed into the JSON would fail
# the parse below on a tree cargo read fine.
err_file="$(mktemp)"
report_file="$(mktemp)"
trap 'rm -f "$err_file" "$report_file"' EXIT
if ! metadata="$(cd "$ROOT" && cargo metadata --no-deps --format-version 1 2>"$err_file")"; then
    echo "❌ check-unused-deps: cargo metadata failed under $ROOT:"
    tail -20 "$err_file" | sed 's/^/   /'
    exit 1
fi
if ! members="$(python3 -c 'import json, sys; print(len(json.load(sys.stdin)["workspace_members"]))' <<< "$metadata")"; then
    echo "❌ check-unused-deps: could not read the workspace members from cargo metadata"
    exit 1
fi
if [[ "$members" -eq 0 ]]; then
    echo "❌ check-unused-deps: the workspace under $ROOT has no member; the check verified nothing"
    exit 1
fi

# Exit status first, report second. cargo-shear exits 0 clean, 1 on a
# finding, 2 when it could not process the workspace; anything but 0 or 1 is
# a crash and the report, if any, is not trusted.
code=0
(cd "$ROOT" && "$SHEAR" shear --deny-warnings --locked --format=json) >"$report_file" 2>"$err_file" || code=$?

if [[ "$code" -ne 0 && "$code" -ne 1 ]]; then
    echo "❌ check-unused-deps: cargo-shear crashed (exit $code); the check verified nothing:"
    tail -20 "$err_file" | sed 's/^/   /'
    exit 1
fi

# The report is the success marker: it must parse, carry a summary, and agree
# with the exit status. An empty or malformed report fails, whatever the exit.
if ! verdict="$(python3 - "$code" "$report_file" 2>&1 <<'PY'
import json, sys

code = int(sys.argv[1])
try:
    with open(sys.argv[2], encoding="utf-8") as fh:
        doc = json.load(fh)
    summary = doc["summary"]
    findings = doc["findings"]
    count = int(summary["errors"]) + int(summary["warnings"])
except (ValueError, KeyError, TypeError) as exc:
    print(f"NOREPORT cargo-shear printed no readable report ({exc})")
    sys.exit(0)
if count != len(findings) or (code == 0) != (count == 0):
    print(f"INCONSISTENT the report disagrees with the exit status: exit {code}, summary {count}, findings {len(findings)}")
    sys.exit(0)
if count == 0:
    print("CLEAN")
    sys.exit(0)
print(f"FINDINGS {count}")
for f in findings:
    where = f.get("file") or "(workspace)"
    line = f"{where}: {f.get('message', '?').splitlines()[0]}"
    rest = f.get("message", "").splitlines()[1:]
    if rest:
        line += " — " + ", ".join(rest)
    if f.get("help"):
        line += f" ({f['help']})"
    print(line)
PY
)"; then
    echo "❌ check-unused-deps: could not judge the cargo-shear report:"
    printf '%s\n' "$verdict" | sed 's/^/   /'
    exit 1
fi

case "$(head -1 <<< "$verdict")" in
    CLEAN)
        echo "✅ check-unused-deps: cargo-shear ${CARGO_SHEAR_VERSION} scanned ${members} workspace crates, no unused or misplaced dependency"
        ;;
    FINDINGS*)
        echo "❌ check-unused-deps: cargo-shear found $(head -1 <<< "$verdict" | cut -d' ' -f2) issue(s):"
        tail -n +2 <<< "$verdict" | sed 's/^/   /'
        echo "   Remove the dependency (or move it to [dev-dependencies]), or, when it is needed but never"
        echo "   named in source, ignore it in that crate with the reason above it:"
        echo "     # <why nothing names it>"
        echo "     [package.metadata.cargo-shear]"
        echo "     ignored = [\"crate-name\"]"
        echo "   Re-run: scripts/ci/check-unused-deps.sh   (cargo shear --fix applies removals and moves)"
        exit 1
        ;;
    *)
        echo "❌ check-unused-deps: ${verdict#* }; the check verified nothing"
        tail -20 "$err_file" | sed 's/^/   /'
        exit 1
        ;;
esac
