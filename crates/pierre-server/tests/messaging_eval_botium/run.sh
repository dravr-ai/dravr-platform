#!/usr/bin/env bash
# ABOUTME: Entry point for real-Slack messaging-eval Botium scenarios
# ABOUTME: Scaffold script — activation requires the second Slack test-driver app (see inline)
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai

# Real-Slack messaging-eval scenario runner.
#
# The Phase 0+1+2 Rust integration tests cover everything that can be
# exercised in-process: asserters, fixture derivation, channel
# renderer shapes, tool-loop invocation, and the /internal/
# conversation-turn endpoint contract. This script runs the remaining
# tier: BotiumScript scenarios driven by botium-cli against a real
# Slack workspace, asserting the coach's live reply.
#
# ## Activation (Phase 2)
#
# Before this script can do anything useful, the test-driver Slack
# app needs to exist. Steps:
#
# 1. Create a second Slack app in the same workspace as the coach
#    (so they share the channel but act as two distinct identities).
#    Enable `chat:write`, `channels:history`, `app_mentions:read`,
#    and Socket Mode (generate an `xapp-…` app-level token).
# 2. Install the app in the test channel (e.g. `#qa-automation`).
#    Invite the coach bot into the same channel.
# 3. Export the creds into the environment (add to .envrc on main):
#
#      BOTIUM_SLACK_BOT_TOKEN=xoxb-…     # test driver's bot token
#      BOTIUM_SLACK_APP_TOKEN=xapp-…     # test driver's app-level token
#      BOTIUM_SLACK_SIGNING_SECRET=…     # test driver's signing secret
#      BOTIUM_SLACK_CHANNEL=Cxxxxxxxx   # shared test channel id
#      BOTIUM_SLACK_COACH_BOT_USER_ID=Uxxxxxxxx   # the *coach* bot's user id
#
# 4. Install JS deps:
#
#      cd crates/pierre-server/tests/messaging_eval_botium
#      bun install
#
# 5. Run this script. It will error loudly when any env var is
#    missing — do NOT paper over the error, each one is required.
#
# ## What success looks like
#
# `bun run scenario:slack` exits zero, prints one PASS line per
# `.convo.txt`, and the Slack channel shows the scenario transcript.
# A nonzero exit means a guardrail didn't fire, a tool wasn't called,
# or a numeric claim didn't match its fixture — see the botium-cli
# output for the per-scenario diagnostic.

set -euo pipefail

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
cd "$SCRIPT_DIR"

require_env() {
  local name="$1"
  if [[ -z "${!name:-}" ]]; then
    echo "ERROR: \$${name} is not set — see activation checklist in run.sh for the required Slack test-driver app creds." >&2
    exit 2
  fi
}

for var in \
  BOTIUM_SLACK_BOT_TOKEN \
  BOTIUM_SLACK_APP_TOKEN \
  BOTIUM_SLACK_SIGNING_SECRET \
  BOTIUM_SLACK_CHANNEL \
  BOTIUM_SLACK_COACH_BOT_USER_ID; do
  require_env "$var"
done

if ! command -v bun >/dev/null 2>&1; then
  echo "ERROR: bun is not on PATH — this project uses bun as the only package manager (see CLAUDE.md)." >&2
  exit 2
fi

if [[ ! -d node_modules ]]; then
  echo "Installing JS dependencies (one-time) via bun…" >&2
  bun install --frozen-lockfile
fi

echo "Running Botium scenarios against Slack workspace…" >&2
bun run scenario:slack
