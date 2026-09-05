#!/usr/bin/env bash
# ABOUTME: Refuses a Cargo.lock in which any dravr-ecosystem crate resolves more than once
# ABOUTME: Compile-free — a text parse of the lockfile, so a bump lane can run it before it pushes
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Why this exists:
#   Cargo treats two git tags of one crate as two packages and resolves both
#   without complaint, so a bump that moves one arm of a diamond leaves the
#   lockfile carrying the crate twice. The workspace then compiles two copies
#   and a type from one is handed to a trait from the other: E0053/E0308 with
#   nothing in the message naming the duplicate. On 2026-09-04 (carnet#323) the
#   enforme v0.1.49 bump pulled dravr-equilibre 0.2.5 while the platform still
#   pinned 0.2.4, and pierre-services/src/health_sync.rs implemented one copy's
#   trait with the other copy's types.
#
#   ci-backend's security-audit job catches the same thing with `cargo tree -d`,
#   minutes into a compile. This is the lockfile-only form of that gate: it reads
#   the text cargo already wrote and runs in about a second, which is what lets
#   every bump lane (embacle, enforme, photograveur) refuse the push instead of
#   discovering the split twenty minutes into a gate, on a branch that cannot merge.
#   Before this script each lane counted entries for the one or two crates it
#   moved by hand; carnet#323 was a duplicate in a crate none of them counted.
#
# Membership — a package is in the ecosystem when EITHER holds:
#   - its name starts with `dravr-` or `embacle` (embacle and embacle-tool-host
#     ship from crates.io under bare names, as does dravr-tronc), or
#   - its source is a git dependency under github.com/dravr-ai/ (photograveur is
#     pinned from dravr-ai/dravr-photograveur under its bare crate name).
#   Both, so a rename or a registry move cannot slip a crate past the gate.
#
# A duplicate is a name that owns more than one `[[package]]` block: two
#   versions, or one version from two sources — Cargo never writes the same
#   (name, version, source) triple twice, so any second block is a split graph.
#
# Usage:
#   check-lockfile-duplicates.sh [path/to/Cargo.lock]
#   (default: the Cargo.lock at the root of the enclosing git repository)
#
# Exit 0  every ecosystem crate resolves exactly once
# Exit 1  at least one resolves more than once (each is printed with every
#         version and source it holds), or the lockfile holds no ecosystem
#         crate at all — a scan that verified nothing must not report success
# Exit 2  the lockfile cannot be read
#
# Portable on purpose: the runners' awk is mawk, so no gawk-only builtins.
set -euo pipefail

LOCK="${1:-$(git rev-parse --show-toplevel)/Cargo.lock}"

if [ ! -r "$LOCK" ]; then
  echo "usage: $0 [path/to/Cargo.lock]" >&2
  echo "cannot read lockfile '${LOCK}'" >&2
  exit 2
fi

RED='\033[0;31m'; GREEN='\033[0;32m'; NC='\033[0m'

# One line per [[package]] block: name, version, source — tab-separated. A
# workspace member has no `source` line and is reported as `path`, so a crate
# that is both vendored in-tree and pulled from git still counts as two.
# Any other table header ([metadata], [[patch.unused]]) closes the block.
entries="$(awk '
  function flush() {
    if (in_pkg && name != "") {
      print name "\t" version "\t" (source == "" ? "path" : source)
    }
    in_pkg = 0; name = ""; version = ""; source = ""
  }
  function unquote(line) {
    sub(/^[a-z]+ = "/, "", line); sub(/"$/, "", line); return line
  }
  /^\[\[package\]\]$/ { flush(); in_pkg = 1; next }
  /^\[/               { flush(); next }
  in_pkg && /^name = "/    { name = unquote($0) }
  in_pkg && /^version = "/ { version = unquote($0) }
  in_pkg && /^source = "/  { source = unquote($0) }
  END { flush() }
' "$LOCK" \
  | awk -F '\t' '$1 ~ /^(dravr-|embacle)/ || $3 ~ /^git\+https:\/\/github\.com\/dravr-ai\// { print }' \
  | sort)"

if [ -z "$entries" ]; then
  echo -e "${RED}❌ lockfile-duplicates: ${LOCK} holds no dravr-ecosystem crate — the scan found nothing to verify.${NC}"
  exit 1
fi

scanned="$(printf '%s\n' "$entries" | wc -l | tr -d ' ')"
dups="$(printf '%s\n' "$entries" | cut -f1 | uniq -d)"

if [ -n "$dups" ]; then
  echo "::error::dravr-ecosystem duplicate(s) detected — every dravr crate must converge on one version:"
  while IFS= read -r name; do
    [ -z "$name" ] && continue
    echo -e "  ${RED}❌ ${name} resolves more than once:${NC}"
    printf '%s\n' "$entries" | awk -F '\t' -v n="$name" '$1 == n { print "       " $2 "  " $3 }'
  done <<< "$dups"
  echo "   Cargo resolves two git tags (or a tag and a registry release) of one crate as"
  echo "   two packages, and the workspace then compiles both. Move every pin of each"
  echo "   crate above to the same version in the same commit — the lockfile cannot"
  echo "   converge while one consumer still asks for the other one."
  exit 1
fi

echo -e "${GREEN}✅ lockfile-duplicates: ${scanned} dravr-ecosystem crate(s), each resolved exactly once${NC}"
