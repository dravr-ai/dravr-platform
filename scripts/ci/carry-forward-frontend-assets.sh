#!/usr/bin/env bash
# ABOUTME: Copies the deployed frontend image's own hashed assets into a new dist, so open tabs keep loading old chunks
# ABOUTME: One release back only: each image records its own asset list and carries forward only the previous image's list
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Why: Vite names every chunk by content hash, and a tab opened before a deploy
# keeps the old index.html in memory. The first lazy import it makes after the
# deploy asks for a chunk the new image no longer holds, gets a 404 (a 503 mid-
# rollout), and the app fails with "Failed to fetch dynamically imported
# module". ErrorBoundary reloads on that error, but the user still sees a failed
# navigation — on 2026-10-28 it was the Google sign-in landing. Serving the
# previous release's chunks from the new image lets those tabs finish what they
# were doing and pick up the new bundle on their next navigation.
#
# Bounded to one generation. Before merging anything, the script writes the
# new build's own asset list to <dist>/release-assets.txt, and it carries
# forward only the files the previous image's list names — never what that image
# itself carried. Each image therefore holds its own assets plus the previous
# release's, never an ever-growing history. An image built before this script
# has no list; all of its assets are its own, so all are carried.
#
# The previous image is the digest <package>:<tag> names (deployed-dev: what dev
# serves now, kept by the registry's keep-referenced-tags rule). Only files under
# assets/ are ever copied, and a name the new build already has is never
# overwritten; index.html, the service worker and every other root file come
# from the new build alone.
#
# Usage: carry-forward-frontend-assets.sh --package <registry>/<package> --tag <tag> --dist <dir>
#
# A package with no <tag> is a first deploy: logged, nothing carried, exit 0.
# A failed tag listing, a tag naming something other than a digest, a failed
# pull/create/copy, a dist or previous image with no assets of its own, an asset
# list naming a path outside assets/ or a file the image lacks each exit 1 with
# an ::error:: annotation.

set -euo pipefail

die() {
  echo "::error::$1" >&2
  exit 1
}

usage() {
  die "usage: $0 --package <registry>/<package> --tag <tag> --dist <dir> — $1"
}

PACKAGE=""
TAG=""
DIST=""
while [ $# -gt 0 ]; do
  case "$1" in
    --package) [ $# -ge 2 ] || usage "--package needs a value"; PACKAGE="$2"; shift 2 ;;
    --tag) [ $# -ge 2 ] || usage "--tag needs a value"; TAG="$2"; shift 2 ;;
    --dist) [ $# -ge 2 ] || usage "--dist needs a value"; DIST="$2"; shift 2 ;;
    *) usage "unknown argument '$1'" ;;
  esac
done
[ -n "${PACKAGE}" ] || usage "--package is required"
[ -n "${TAG}" ] || usage "--tag is required"
[ -n "${DIST}" ] || usage "--dist is required"
[ -f "${DIST}/index.html" ] || die "${DIST} has no index.html; it is not a built frontend dist"
[ -d "${DIST}/assets" ] || die "${DIST} has no assets/ directory; it is not a built frontend dist"

DIGEST_RE='^sha256:[0-9a-f]{64}$'
# A Vite asset path: under assets/, no `..` segment, no spaces or shell metacharacters.
ASSET_RE='^assets/[A-Za-z0-9._@+-]+(/[A-Za-z0-9._@+-]+)*$'
MANIFEST="release-assets.txt"
# Where docker/images/frontend/Dockerfile copies the dist.
IMAGE_ROOT="/usr/share/nginx/html"

WORK=$(mktemp -d)
CONTAINER=""
cleanup() {
  if [ -n "${CONTAINER}" ]; then docker rm "${CONTAINER}" >/dev/null 2>&1 || true; fi
  rm -rf "${WORK}"
}
trap cleanup EXIT

# 1. Record this build's own assets, before anything is merged into it.
(cd "${DIST}" && find assets -type f) > "${WORK}/own.unsorted" \
  || die "could not list the assets of ${DIST}"
LC_ALL=C sort "${WORK}/own.unsorted" > "${DIST}/${MANIFEST}"
# The next release reads this list and refuses a name outside ASSET_RE, so a
# name it would refuse fails this build, the one that introduced it.
while IFS= read -r rel; do
  [[ "${rel}" =~ ${ASSET_RE} ]] \
    || die "this build's asset '${rel}' is not a name the next release can carry forward"
done < "${DIST}/${MANIFEST}"
OWN_COUNT=$(awk 'END { print NR }' "${DIST}/${MANIFEST}")
# A Vite build always emits hashed assets; an empty list recorded nothing, and
# the next release would carry nothing forward from this image.
[ "${OWN_COUNT}" -gt 0 ] || die "${DIST}/assets holds no files; it is not a built frontend dist"
echo "This build has ${OWN_COUNT} assets, recorded in ${MANIFEST}"

# 2. Which digest is the previous release.
if ! LISTING=$(gcloud artifacts docker tags list "${PACKAGE}" --format='value(tag.basename(),version.basename())'); then
  die "could not list the tags of ${PACKAGE}, so the previous release is unknown"
fi
PREVIOUS=$(awk -v tag="${TAG}" '$1 == tag { print $2 }' <<<"${LISTING}")
if [ -z "${PREVIOUS}" ]; then
  echo "${PACKAGE} has no ${TAG} tag: its first deploy, so there are no previous assets to carry forward"
  exit 0
fi
[[ "${PREVIOUS}" =~ ${DIGEST_RE} ]] \
  || die "${TAG} on ${PACKAGE} reads as '${PREVIOUS}', not one digest"
IMAGE="${PACKAGE}@${PREVIOUS}"

# 3. Copy the previous image's web root out of it.
echo "Reading the previous release's assets from ${IMAGE} (${TAG})"
docker pull --quiet "${IMAGE}" >/dev/null || die "could not pull ${IMAGE}"
CONTAINER=$(docker create "${IMAGE}") || die "could not create a container from ${IMAGE}"
docker cp "${CONTAINER}:${IMAGE_ROOT}/." "${WORK}/previous" \
  || die "could not copy ${IMAGE_ROOT} out of ${IMAGE}"
[ -d "${WORK}/previous/assets" ] || die "${IMAGE} has no ${IMAGE_ROOT}/assets; it is not a frontend image"

# 4. Which of its files were its own.
if [ -f "${WORK}/previous/${MANIFEST}" ]; then
  cp "${WORK}/previous/${MANIFEST}" "${WORK}/carry.list"
  echo "  its ${MANIFEST} names the assets it built itself"
else
  (cd "${WORK}/previous" && find assets -type f) > "${WORK}/carry.list" \
    || die "could not list the assets of ${IMAGE}"
  echo "  it predates ${MANIFEST}, so every asset it holds is its own"
fi
# A frontend image always holds its own assets: a list naming none would carry
# nothing and report success, a merge that verified nothing.
grep -q . "${WORK}/carry.list" || die "${IMAGE} names no assets of its own; it is not a frontend image"

# 5. Merge, never overwriting what this build produced.
CARRIED=0
ALREADY=0
while IFS= read -r rel || [ -n "${rel}" ]; do
  [ -n "${rel}" ] || continue
  [[ "${rel}" =~ ${ASSET_RE} ]] && [[ "/${rel}/" != */../* ]] \
    || die "${IMAGE} lists '${rel}', which is not a path under assets/"
  src="${WORK}/previous/${rel}"
  [ -f "${src}" ] && [ ! -L "${src}" ] \
    || die "${IMAGE} lists ${rel} but holds no regular file there"
  dest="${DIST}/${rel}"
  if [ -e "${dest}" ]; then
    # Same name, same hash: the chunk did not change between the releases.
    if ! cmp -s "${src}" "${dest}"; then
      echo "::warning::${rel} differs between ${IMAGE} and this build; keeping this build's copy"
    fi
    ALREADY=$((ALREADY + 1))
    continue
  fi
  mkdir -p "$(dirname "${dest}")"
  cp "${src}" "${dest}" || die "could not copy ${rel} into ${DIST}"
  CARRIED=$((CARRIED + 1))
done < "${WORK}/carry.list"

echo "Carried ${CARRIED} assets forward from ${IMAGE}; ${ALREADY} were already in this build"
