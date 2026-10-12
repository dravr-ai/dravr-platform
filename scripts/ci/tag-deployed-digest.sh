#!/usr/bin/env bash
# ABOUTME: Moves deployed-<env> onto the digest a deploy just shipped and keeps the outgoing one as deployed-<env>-previous
# ABOUTME: The one registry tag move both dev deploy paths run; every failure exits 1, only a first deploy has no N-1
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# Why: dev deploys by digest, so nothing on a serving image tells the registry
# cleanup policy (infra/artifacts) to spare it. Its recent-versions floor keeps
# two VERSIONS per package and every push writes three (the OCI index carrying
# the tags, its linux/amd64 manifest and an attestation), so the floor holds less
# than one build, and delete-superseded reaps any version past the stale window
# whatever tags it carries, unless a keep rule matches it first.
# keep-referenced-tags matches the `deployed-` prefix and wins over every delete
# rule, so a deploy leaves two anchors per package:
#
#   deployed-<env>           the digest the last successful deploy shipped
#   deployed-<env>-previous  the digest deployed-<env> named before that deploy:
#                            the N-1 rollback target (carnet#137)
#
# Without the second tag the outgoing digest keeps only its SHA tag once
# deployed-<env> moves, and delete-superseded reaps it on the first cleanup pass
# after it is older than the stale window: for an image that served longer than
# that window, the very next pass. A rollback to the image dev served before the
# deploy would then hold for a few hours at most. The anchor keeps exactly one
# more build per package and leaves keep_count at its carnet#166 cost floor.
#
# The move, in order:
#   1. read which digest deployed-<env> names now: the outgoing digest;
#   2. tag that digest deployed-<env>-previous, unless there is none (the
#      package's first deploy) or it is the digest being shipped (a re-run after
#      deployed-<env> already moved, which must not overwrite N-1 with N);
#   3. move deployed-<env> onto the shipped digest.
# Step 2 runs before step 3, so the outgoing digest is never without a
# `deployed-` tag. A run that fails between them leaves deployed-<env> where it
# was, so a re-run reads the same outgoing digest and completes the move.
# `gcloud artifacts docker tags add` deletes a tag that already exists and
# re-creates it on the given digest, which makes every write here safe to repeat
# (and is why the publisher holds artifactregistry.tags.delete,
# infra/artifacts/main.tf).
#
# "Previous" is the previous digest a deploy tail shipped. A rollback by hand
# (`gcloud run services update --image`) moves neither tag, so the deploy after
# one anchors the digest deployed-<env> still names, not the hand-picked one.
#
# Usage: tag-deployed-digest.sh --image <package>@sha256:<64 hex> --env <env>
#   --image  the digest-pinned image the deploy shipped, as the publish jobs
#            output it (<registry>/<package>@sha256:...)
#   --env    the environment segment of the tag: dev
#
# A failed tag listing, a deployed-<env> tag that names something other than a
# digest, and a failed tag write each exit 1 with an ::error:: annotation. Only a
# package with no deployed-<env> tag at all is read as a first deploy.

set -euo pipefail

# Annotation and message go to stderr, so a failure is never swallowed by a
# caller capturing stdout.
die() {
  echo "::error::$1" >&2
  exit 1
}

usage() {
  die "usage: $0 --image <package>@sha256:<64 hex> --env <env> — $1"
}

IMAGE=""
ENV_NAME=""
while [ $# -gt 0 ]; do
  case "$1" in
    --image) [ $# -ge 2 ] || usage "--image needs a value"; IMAGE="$2"; shift 2 ;;
    --env) [ $# -ge 2 ] || usage "--env needs a value"; ENV_NAME="$2"; shift 2 ;;
    *) usage "unknown argument '$1'" ;;
  esac
done
[ -n "${IMAGE}" ] || usage "--image is required"
[ -n "${ENV_NAME}" ] || usage "--env is required"

DIGEST_RE='^sha256:[0-9a-f]{64}$'
PACKAGE="${IMAGE%@*}"
DIGEST="${IMAGE##*@}"
if [ "${PACKAGE}" = "${IMAGE}" ] || ! [[ "${DIGEST}" =~ ${DIGEST_RE} ]]; then
  usage "--image must be digest-pinned (<package>@sha256:<64 hex>), got '${IMAGE}'"
fi
[[ "${ENV_NAME}" =~ ^[a-z][a-z0-9]*$ ]] || usage "--env must be one lowercase word, got '${ENV_NAME}'"

CURRENT="deployed-${ENV_NAME}"
PREVIOUS="deployed-${ENV_NAME}-previous"

# The tags list command's own table prints these two columns (tag.basename(),
# version.basename()); value() prints them tab-separated, one tag per line.
if ! LISTING=$(gcloud artifacts docker tags list "${PACKAGE}" --format='value(tag.basename(),version.basename())'); then
  die "could not list the tags of ${PACKAGE}, so the outgoing digest is unknown; neither ${PREVIOUS} nor ${CURRENT} was moved"
fi
OUTGOING=$(awk -v tag="${CURRENT}" '$1 == tag { print $2 }' <<<"${LISTING}")

if [ -z "${OUTGOING}" ]; then
  echo "${PACKAGE} has no ${CURRENT} tag: its first deploy, so there is no outgoing digest to keep as ${PREVIOUS}"
elif ! [[ "${OUTGOING}" =~ ${DIGEST_RE} ]]; then
  die "${CURRENT} on ${PACKAGE} reads as '${OUTGOING}', not one digest; neither ${PREVIOUS} nor ${CURRENT} was moved"
elif [ "${OUTGOING}" = "${DIGEST}" ]; then
  echo "${CURRENT} already names ${DIGEST} (a re-run): ${PREVIOUS} stays where it is"
else
  echo "Tagging the outgoing ${PACKAGE}@${OUTGOING} as ${PACKAGE}:${PREVIOUS}"
  gcloud artifacts docker tags add "${PACKAGE}@${OUTGOING}" "${PACKAGE}:${PREVIOUS}" --quiet \
    || die "could not tag the outgoing ${PACKAGE}@${OUTGOING} as ${PREVIOUS}; ${CURRENT} was left on it"
fi

echo "Tagging ${IMAGE} as ${PACKAGE}:${CURRENT}"
gcloud artifacts docker tags add "${IMAGE}" "${PACKAGE}:${CURRENT}" --quiet \
  || die "could not move ${CURRENT} onto ${IMAGE}"
