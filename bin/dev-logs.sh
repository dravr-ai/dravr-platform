#!/usr/bin/env bash
# ABOUTME: Tail dev Cloud Run (dravr-mcp-server-api) logs, keeping ONLY the binary's
# ABOUTME: structured tracing output (jsonPayload) — drops 100% of the gcsfuse sidecar noise.
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
#
# The dev service mounts the dravr-dev-sciotte-scripts GCS bucket via gcsfuse, which
# floods Cloud Run logs with plain-text banners (GCSFuse Config / mount-id / ...).
# gcsfuse only ever emits `textPayload`; the dravr binary logs via tracing -> `jsonPayload`.
# So filtering on `jsonPayload:*` keeps every app line and discards every gcsfuse line,
# regardless of what new banners Google adds. The only thing it misses is genuinely
# unstructured app output (raw panics on stderr, pre-tracing-init lines) — `--errors`
# adds a targeted `severity>=ERROR AND textPayload:*` pass to catch those.
#
# Usage:
#   bin/dev-logs.sh                 # last 30m of structured app logs (newest last)
#   bin/dev-logs.sh --since 2h      # widen the window
#   bin/dev-logs.sh --errors        # errors only, + a raw-stderr (panic) pass
#   bin/dev-logs.sh garmin sciotte  # keyword-filter the structured logs (OR'd, case-insensitive)
set -euo pipefail

PROJECT="dravr-dev"
SERVICE="dravr-mcp-server-api"
SINCE="30m"
ERRORS=0
KEYWORDS=()

while [ $# -gt 0 ]; do
  case "$1" in
    --errors)    ERRORS=1; shift ;;
    --since)     SINCE="$2"; shift 2 ;;
    --since=*)   SINCE="${1#*=}"; shift ;;
    -h|--help)   sed -n '15,21p' "$0"; exit 0 ;;
    *)           KEYWORDS+=("$1"); shift ;;
  esac
done

FILTER="resource.type=\"cloud_run_revision\" AND resource.labels.service_name=\"$SERVICE\" AND jsonPayload:*"
[ "$ERRORS" = "1" ] && FILTER="$FILTER AND severity>=ERROR"

OUT="$(gcloud logging read "$FILTER" --project="$PROJECT" --limit=300 --freshness="$SINCE" \
  --format='value(timestamp, severity, jsonPayload.message)' 2>/dev/null | tac)"

if [ "${#KEYWORDS[@]}" -gt 0 ]; then
  PAT="$(IFS='|'; echo "${KEYWORDS[*]}")"
  printf '%s\n' "$OUT" | grep -iE "$PAT" || true
else
  printf '%s\n' "$OUT"
fi

if [ "$ERRORS" = "1" ]; then
  echo "=== raw stderr errors (textPayload — panics / pre-tracing-init) ==="
  gcloud logging read \
    "resource.type=\"cloud_run_revision\" AND resource.labels.service_name=\"$SERVICE\" AND severity>=ERROR AND textPayload:*" \
    --project="$PROJECT" --limit=40 --freshness="$SINCE" --format='value(timestamp, textPayload)' 2>/dev/null \
    | grep -ivE 'GCSFuse|mount-id|GetStorageLayout|UniverseDomain|storageLayout' | tac || true
fi
