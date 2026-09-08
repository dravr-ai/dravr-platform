#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: Ensures gh CLI is installed and authenticated for CI monitoring.
# ABOUTME: Handles local (interactive) and containerized (web) environments differently.

GH_INSTALL_DIR="$HOME/.local/bin"

# Add local bin to PATH if not already there
if [[ ":$PATH:" != *":$GH_INSTALL_DIR:"* ]]; then
    export PATH="$GH_INSTALL_DIR:$PATH"
fi

# ---------------------------------------------------------------------------
# Detect environment: local vs containerized (Claude Code for Web)
# ---------------------------------------------------------------------------
# CLAUDE_CODE_REMOTE_SESSION_ID is the one that matters in practice: an Anthropic-hosted
# cloud session runs in a VM, not a container, so it has no /.dockerenv and no
# /run/.containerenv. Without this the script took the local branch there and tried to
# install gh — which cannot work and reported GH_INSTALL_FAILED, burying the fact that
# GitHub is reachable by proxy-authenticated curl.
IS_CONTAINER=false
if [ -f "/.dockerenv" ] || [ -f "/run/.containerenv" ] || [ -n "${KUBERNETES_SERVICE_HOST:-}" ] \
   || [ -n "${CLAUDE_CODE_REMOTE_SESSION_ID:-}" ]; then
    IS_CONTAINER=true
fi

# ---------------------------------------------------------------------------
# Containerized environment — cannot install software or run interactive auth
# ---------------------------------------------------------------------------
if [ "$IS_CONTAINER" = true ]; then
    # GH_TOKEN / GITHUB_TOKEN are not evidence of anything in a cloud session. When the
    # agent proxy handles GitHub auth they read as a short placeholder ("proxy-injected"),
    # which is non-empty — so testing them reports a credential the session does not have
    # while gh is not even installed. Probe the capability instead: the proxy injects the
    # real credential on the wire, so an unauthenticated request to the API answers 200 as
    # the authenticated user, at the app-installation rate limit rather than the anonymous one.
    if command -v gh &>/dev/null && gh auth status &>/dev/null; then
        echo "✅ gh CLI ready (container) - can monitor workflows"
        echo "CI_MONITORING=gh"
    elif [ "$(curl -s -o /dev/null -w '%{http_code}' --max-time 5 https://api.github.com/user 2>/dev/null)" = "200" ]; then
        echo "✅ GitHub API authenticated by the agent proxy (container) - gh absent, curl works"
        echo "CI_MONITORING=curl"
        echo "GH_VIA_PROXY: curl against https://api.github.com is authenticated on the wire — send no Authorization header (one you send is overridden), and do not read GH_TOKEN, which is a placeholder rather than a credential. Use curl for GitHub REST, or mcp__github__* for issue and PR operations."
    else
        echo "CONTAINERIZED_ENVIRONMENT=true"
        echo "CI_MONITORING=fallback"
        echo "GH_UNAVAILABLE: Running in a containerized environment (Claude Code for Web). Cannot install gh or run interactive auth. Use GitHub MCP tools (mcp__github__*) for CI monitoring and workflow operations. If those are unavailable, use WebFetch with https://github.com/dravr-ai/dravr-platform/actions."
    fi
    exit 0
fi

# ---------------------------------------------------------------------------
# Local environment — can install gh and prompt user for interactive auth
# ---------------------------------------------------------------------------

# Step 1: Install gh if missing
if ! command -v gh &>/dev/null && ! [ -x "$GH_INSTALL_DIR/gh" ]; then
    echo "⚠️ gh CLI not found — installing..."

    OS="$(uname -s)"
    ARCH="$(uname -m)"

    case "$OS" in
        Darwin)
            if command -v brew &>/dev/null; then
                brew install gh 2>/dev/null
            else
                echo "GH_INSTALL_FAILED: brew not available on macOS. Tell the user to run: ! brew install gh"
                echo "CI_MONITORING=fallback"
                exit 0
            fi
            ;;
        Linux)
            # Download latest gh binary
            mkdir -p "$GH_INSTALL_DIR"
            case "$ARCH" in
                x86_64|amd64) GH_ARCH="amd64" ;;
                aarch64|arm64) GH_ARCH="arm64" ;;
                *) echo "GH_INSTALL_FAILED: unsupported architecture $ARCH"; echo "CI_MONITORING=fallback"; exit 0 ;;
            esac
            GH_VERSION=$(curl -sL "https://api.github.com/repos/cli/cli/releases/latest" | grep '"tag_name"' | head -1 | sed 's/.*"v\(.*\)".*/\1/')
            if [ -n "$GH_VERSION" ]; then
                curl -sL "https://github.com/cli/cli/releases/download/v${GH_VERSION}/gh_${GH_VERSION}_linux_${GH_ARCH}.tar.gz" | tar xz -C /tmp
                cp "/tmp/gh_${GH_VERSION}_linux_${GH_ARCH}/bin/gh" "$GH_INSTALL_DIR/gh"
                chmod +x "$GH_INSTALL_DIR/gh"
            else
                echo "GH_INSTALL_FAILED: could not determine latest gh version"
                echo "CI_MONITORING=fallback"
                exit 0
            fi
            ;;
        *)
            echo "GH_INSTALL_FAILED: unsupported OS $OS"
            echo "CI_MONITORING=fallback"
            exit 0
            ;;
    esac
fi

# Step 2: Verify installation
if ! command -v gh &>/dev/null && ! [ -x "$GH_INSTALL_DIR/gh" ]; then
    echo "GH_INSTALL_FAILED: gh installation did not succeed"
    echo "CI_MONITORING=fallback"
    exit 0
fi

# Step 3: Check authentication
if gh auth status &>/dev/null; then
    echo "✅ gh CLI ready - can monitor workflows"
    echo "CI_MONITORING=gh"
else
    echo "CI_MONITORING=blocked"
    echo "GH_AUTH_REQUIRED: gh CLI is installed but not authenticated. This is a private repo — gh auth is required for CI monitoring and workflow dispatch. Prompt the user to run:  ! gh auth login --web"
fi
