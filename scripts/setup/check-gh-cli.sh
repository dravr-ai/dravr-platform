#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: Checks gh CLI and GitHub MCP availability for CI monitoring.
# ABOUTME: Outputs a CI_MONITORING directive telling Claude which method to use.

GH_INSTALL_DIR="$HOME/.local/bin"

# Add local bin to PATH if not already there
if [[ ":$PATH:" != *":$GH_INSTALL_DIR:"* ]]; then
    export PATH="$GH_INSTALL_DIR:$PATH"
fi

GH_AVAILABLE=false
GH_AUTHENTICATED=false

# Check if gh is installed
if command -v gh &> /dev/null || [ -x "$GH_INSTALL_DIR/gh" ]; then
    GH_AVAILABLE=true
    # Check if gh is authenticated
    if gh auth status &> /dev/null; then
        GH_AUTHENTICATED=true
    fi
fi

# Determine CI monitoring method and output directive
if [ "$GH_AUTHENTICATED" = true ]; then
    echo "✅ gh CLI ready - can monitor workflows"
    echo "CI_MONITORING=gh"
elif [ "$GH_AVAILABLE" = true ]; then
    echo "⚠️ gh CLI installed but not authenticated"
    echo "CI_MONITORING=fallback"
    echo "CI_FALLBACK: gh CLI is not authenticated. DO NOT ask for a GitHub token. Use GitHub MCP tools (mcp__github__*) if available, otherwise use WebFetch with https://github.com/dravr-ai/dravr-platform/actions to monitor CI."
else
    echo "⚠️ gh CLI not installed"
    echo "CI_MONITORING=fallback"
    echo "CI_FALLBACK: gh CLI is not available. DO NOT ask for a GitHub token or try to install gh. Use GitHub MCP tools (mcp__github__*) if available, otherwise use WebFetch with https://github.com/dravr-ai/dravr-platform/actions to monitor CI."
fi
