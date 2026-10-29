#!/usr/bin/env bash
# ABOUTME: Prints a key naming everything a server build compiles from: git object ids of its inputs plus a variant
# ABOUTME: Two commits with the same key build the same binaries, so an e2e lane reuses an earlier build's artifact
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Why this exists:
#   The e2e lanes compile the release server on every run: 23.8 min on the iOS
#   macOS runner, 15 on Linux. On Oct 22-29, 13 of 85 iOS builds, 5 of 75
#   Android and 26 of 217 Server E2E builds compiled inputs an earlier green
#   build had already compiled (carnet#1258). The build uploads its binaries
#   under an artifact named after this key; the next run with the same key
#   downloads them instead (.github/actions/prebuilt-server).
#
# What the key covers:
#   The git object id (tree or blob) of each path in INPUTS, read from HEAD, so
#   it costs no file reads and is exact: any byte changed under crates/ changes
#   the crates tree id. INPUTS is every file outside crates/ the server's
#   compile reads (include_str! of the locales and constants, the migrations
#   sqlx embeds, the SDK build.rs builds, the command catalogue) plus the
#   manifests and toolchain. It is wider than the compile needs: an extra path
#   costs a cache miss, a missing one ships a stale binary to the e2e tests.
#   scripts/ci/check-rust-inputs.py keeps ci-backend-changes' `rust` filter
#   honest about files outside crates/; keep INPUTS a superset of that
#   filter's compile-time entries.
#
#   The variant (argument 1) names everything that is not a file at HEAD:
#   runner OS and architecture, profile, features. Further arguments are extra
#   paths, normally the calling workflow, so a change to how it builds (the
#   toolchain it pins, the command it runs) starts a new key.
#
# Output: a 24-hex-digit key on stdout, exit 0. A path absent at HEAD hashes
# as absent, so deleting an input changes the key too.
# Exit 2 on a missing variant; exit 3 when git cannot read HEAD (a caller must
# then build, never reuse on a key it could not compute).
set -euo pipefail

INPUTS=(
  crates
  Cargo.toml
  Cargo.lock
  rust-toolchain.toml
  rust-toolchain
  .cargo
  migrations
  migrations_pg
  packages/i18n
  packages/shared-constants
  packages/mcp-types
  frontend/src/utils/tokenLink.ts
  sdk
  commands
)

VARIANT="${1:-}"
if [ -z "$VARIANT" ]; then
  echo "usage: $0 <variant> [extra-path...]" >&2
  exit 2
fi
shift

if ! git rev-parse --verify --quiet 'HEAD^{commit}' >/dev/null; then
  echo "server-build-key: git cannot read HEAD in $(pwd)" >&2
  exit 3
fi

{
  printf 'variant %s\n' "$VARIANT"
  for path in "${INPUTS[@]}" "$@"; do
    if id="$(git rev-parse --verify --quiet "HEAD:${path}")"; then
      printf '%s %s\n' "$id" "$path"
    else
      printf 'absent %s\n' "$path"
    fi
  done
} | sha256sum | cut -c1-24
