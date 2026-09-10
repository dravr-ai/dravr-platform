#!/usr/bin/env bash
# ABOUTME: Regenerates packages/scene-types from a dravr-photograveur tag, using that tag's own generator
# ABOUTME: The after_rewrite step of the photograveur bump lane — runs inside the bump commit, never after it

# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# packages/scene-types is generated from photograveur's Rust types by ts-rs. Bumping
# the crate without regenerating leaves web and mobile type-checking against the
# previous Scene shape — which compiles, and then fails to render the field that
# moved. So this runs inside the bump commit rather than as a follow-up: the pins and
# the bindings they describe land together or not at all.
#
# The generator is the crate's own script at the target tag, so it is whatever the new
# release says it is. Nothing here fabricates types on a miss — an absent checkout or
# script fails the run rather than pushing a bump with stale bindings.

set -euo pipefail

VERSION="${1:?usage: regen-scene-types.sh <version>   (bare, e.g. 0.3.1)}"
TAG="v${VERSION#v}"
WORK="${TMPDIR:-/tmp}/photograveur-typegen"

rm -rf "${WORK}"
git clone --depth 1 --branch "${TAG}" \
  https://github.com/dravr-ai/dravr-photograveur.git "${WORK}"
( cd "${WORK}" && ./scripts/generate-ts-types.sh )

if ! ls "${WORK}"/bindings/*.ts >/dev/null 2>&1; then
  echo "::error::the generator produced no bindings — refusing to bump with stale scene types"
  exit 1
fi

# packages/scene-types/src is generated in full — types plus the barrel the generator
# writes — so it is replaced wholesale rather than merged. A type deleted upstream has
# to disappear here too, or the barrel re-exports a file that no longer backs anything.
rm -f packages/scene-types/src/*.ts
cp "${WORK}"/bindings/*.ts packages/scene-types/src/
git diff --stat -- packages/scene-types || true
