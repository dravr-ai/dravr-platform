#!/usr/bin/env bash
# ABOUTME: Stops this checkout's Cloudflare tunnel and points BASE_URL back at the local server
# ABOUTME: Resetting is the point — a hostname left in .envrc outlives the process that served it
#
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
. "$SCRIPT_DIR/dev-processes.sh"
. "$SCRIPT_DIR/tunnel-env.sh"

dev_stop tunnel "Cloudflare tunnel"

if tunnel_env_reset "$PROJECT_ROOT"; then
    echo "  BASE_URL and EXPO_PUBLIC_API_URL point at the local server"
    echo "  Run 'direnv allow' and restart Pierre to pick it up"
else
    echo "  BASE_URL and EXPO_PUBLIC_API_URL already point somewhere durable; left as they are"
fi
