#!/usr/bin/env bash
# ABOUTME: Clones every satellite in satellites.toml plus dravr-tronc and runs pierre-sechoir over the fleet
# ABOUTME: Prints the markdown report; fails closed when a clone or the scan fails, never reports a partial fleet
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Most duplication in this codebase sits between repositories: satellites that
# copied each other's server scaffolding, and models copied out of the platform.
# A scan inside one repository cannot see that, so this one reads them all.
#
# The repository list comes from satellites.toml through satellite-pin.sh, the
# one reader of that file, so a new satellite is scanned the day it gets a
# stanza. dravr-tronc is named here because it is a crates.io dependency with
# no stanza, and the shared runtime is where most copies belong.
#
# Usage:  fleet-duplication.sh <work-dir> [pierre-sechoir options...]
#   SECHOIR         the pierre-sechoir binary (default target/release/pierre-sechoir)
#   FLEET_GIT_BASE  where repositories are cloned from (default https://github.com/dravr-ai)
#   SATELLITE_PIN_ROOT  read by satellite-pin.sh: the checkout holding satellites.toml
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="${SATELLITE_PIN_ROOT:-$(cd "$SCRIPT_DIR/../.." && pwd)}"
PIN_SH="$SCRIPT_DIR/satellite-pin.sh"
SECHOIR="${SECHOIR:-$REPO_ROOT/target/release/pierre-sechoir}"
GIT_BASE="${FLEET_GIT_BASE:-https://github.com/dravr-ai}"
SHARED_RUNTIME_REPO="dravr-tronc"

WORK="${1:?usage: fleet-duplication.sh <work-dir> [pierre-sechoir options...]}"
shift
[ -x "$SECHOIR" ] || { echo "❌ pierre-sechoir not found at $SECHOIR (cargo build --release -p pierre-sechoir)" >&2; exit 2; }
mkdir -p "$WORK"

if ! names="$("$PIN_SH" names)"; then
  echo "❌ cannot read the satellite list from satellites.toml" >&2
  exit 2
fi

# Several stanzas can name one repository; each is cloned and scanned once.
repos=("$SHARED_RUNTIME_REPO")
for name in $names; do
  repo="$("$PIN_SH" repo "$name")"
  case " ${repos[*]} " in *" $repo "*) ;; *) repos+=("$repo") ;; esac
done

roots=("platform=$REPO_ROOT/crates")
for repo in "${repos[@]}"; do
  dest="$WORK/$repo"
  if [ ! -d "$dest/.git" ]; then
    if ! git clone --quiet --depth 1 "$GIT_BASE/$repo" "$dest" >&2; then
      echo "❌ cannot clone $repo; refusing to report a partial fleet" >&2
      exit 1
    fi
  fi
  roots+=("${repo#dravr-}=$dest")
done

echo "Scanning platform and ${#repos[@]} repositories: ${repos[*]}" >&2
if ! report="$("$SECHOIR" "$@" "${roots[@]}")"; then
  echo "❌ pierre-sechoir failed" >&2
  exit 1
fi
printf '%s\n' "$report"
