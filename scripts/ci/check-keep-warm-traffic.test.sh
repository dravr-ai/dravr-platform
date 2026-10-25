#!/usr/bin/env bash
# ABOUTME: Fixture test for check-keep-warm-traffic.sh — builds throwaway infra/ trees and asserts each verdict
# ABOUTME: Pins that an uptime check or scheduler job without a '# Idle-cost:' note fails and an annotated one passes
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UNDER_TEST="${UNDER_TEST:-$SCRIPT_DIR/check-keep-warm-traffic.sh}"
# Every fixture lives under one directory made here, at top level: tree_with
# runs inside $(...), a subshell, so it could not record what it made.
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
OUT="$WORK/out"

failures=0
pass() { echo "  ✅ $1"; }
fail() { echo "  ❌ $1"; failures=$((failures + 1)); }

tree_with() { # $1 = contents of infra/environments/dev/probe.tf
  local dir
  dir="$(mktemp -d "$WORK/tree.XXXXXX")"
  mkdir -p "$dir/infra/environments/dev"
  printf '%s\n' "$1" >"$dir/infra/environments/dev/probe.tf"
  printf '%s\n' "$dir"
}

run_gate() {
  local code=0
  bash "$UNDER_TEST" "$1" >"$OUT" 2>&1 || code=$?
  echo "$code"
}

expect() { # $1 = label, $2 = actual exit, $3 = expected exit
  if [ "$2" = "$3" ]; then pass "$1"; else
    fail "$1 (exit $2, expected $3)"
    sed 's/^/      /' "$OUT"
  fi
}

echo "check-keep-warm-traffic.sh fixtures"

expect "no infra/ directory passes" "$(run_gate "$(mktemp -d "$WORK/empty.XXXXXX")")" 0

expect "infra with neither resource passes" "$(run_gate "$(tree_with 'resource "google_storage_bucket" "b" {
  name = "x"
}')")" 0

expect "uptime check with no note fails" "$(run_gate "$(tree_with '# Probes sign-in every five minutes.
resource "google_monitoring_uptime_check_config" "sign_in" {
  period = "300s"
}')")" 1

expect "scheduler job with no note fails" "$(run_gate "$(tree_with 'resource "google_cloud_scheduler_job" "poke" {
  schedule = "*/5 * * * *"
}')")" 1

expect "uptime check with a note passes" "$(run_gate "$(tree_with '# Probes the status page, a static bucket.
# Idle-cost: none, the target is a Cloud Storage site, not a Cloud Run service.
resource "google_monitoring_uptime_check_config" "status" {
  period = "900s"
}')")" 0

expect "scheduler job with a note passes" "$(run_gate "$(tree_with '# Idle-cost: none, it starts a Cloud Run job through the :run API and wakes no service.
resource "google_cloud_scheduler_job" "drift" {
  schedule = "0 6 * * *"
}')")" 0

expect "an empty note fails" "$(run_gate "$(tree_with '# Idle-cost:
resource "google_cloud_scheduler_job" "poke" {
  schedule = "0 * * * *"
}')")" 1

expect "a note more than 10 lines above fails" "$(run_gate "$(tree_with '# Idle-cost: none.
#
#
#
#
#
#
#
#
#
#
resource "google_cloud_scheduler_job" "poke" {
  schedule = "0 * * * *"
}')")" 1

expect "a note inside a string does not count" "$(run_gate "$(tree_with 'locals {
  doc = "Idle-cost: none"
}
resource "google_monitoring_uptime_check_config" "sign_in" {
  period = "300s"
}')")" 1

expect "one annotated, one not: fails" "$(run_gate "$(tree_with '# Idle-cost: none, starts a job.
resource "google_cloud_scheduler_job" "a" {
  schedule = "0 6 * * *"
}

resource "google_cloud_scheduler_job" "b" {
  schedule = "0 7 * * *"
}')")" 1

expect "a commented-out resource is ignored" "$(run_gate "$(tree_with '# resource "google_monitoring_uptime_check_config" "old" {
# }')")" 0

if [ "$failures" -ne 0 ]; then
  echo "❌ $failures fixture(s) failed"
  exit 1
fi
echo "✅ all fixtures passed"
