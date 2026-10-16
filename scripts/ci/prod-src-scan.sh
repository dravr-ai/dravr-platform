#!/bin/bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: Prints the production-code ripgrep matches of PATTERN, dropping each file's trailing test module
# ABOUTME: Fails closed: an rg crash or a crash of .build's test-module filter exits 2, never an empty match list
#
# Usage: prod-src-scan.sh PATTERN PATH-OR-RG-FLAG...
#
# architectural-validation.sh counts its unwrap / expect / panic / NULL-UUID /
# mock / underscore-name findings from this output. Those gates pass on zero
# lines, so a scan that crashed must not print zero lines and exit 0
# (carnet#927): ripgrep's exit 1 means "no match" and is a clean result,
# anything above it (a bad pattern, a moved path) fails, and so does the
# filter exiting non-zero.
#
# TEST_MODULE_LINES names the filter; it defaults to the .build submodule's
# validation/test-module-lines.sh (the one validate.sh uses, ADR-027).

set -euo pipefail

if [[ $# -lt 2 ]]; then
    echo "prod-src-scan.sh: usage: prod-src-scan.sh PATTERN PATH..." >&2
    exit 2
fi

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
FILTER="${TEST_MODULE_LINES:-$ROOT/.build/validation/test-module-lines.sh}"
if [[ ! -x "$FILTER" ]]; then
    echo "prod-src-scan.sh: test-module filter $FILTER is missing or not executable" >&2
    exit 2
fi

pattern="$1"
shift

rg_rc=0
matches=$(rg -n --with-filename "$pattern" "$@") || rg_rc=$?
if [[ $rg_rc -gt 1 ]]; then
    echo "prod-src-scan.sh: rg failed (exit $rg_rc) scanning for '$pattern' in: $*" >&2
    exit 2
fi
[[ $rg_rc -eq 1 ]] && exit 0

filter_rc=0
filtered=$(printf '%s\n' "$matches" | "$FILTER") || filter_rc=$?
if [[ $filter_rc -ne 0 ]]; then
    echo "prod-src-scan.sh: test-module filter $FILTER failed (exit $filter_rc)" >&2
    exit 2
fi
[[ -n "$filtered" ]] && printf '%s\n' "$filtered"
exit 0
