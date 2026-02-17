#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: Checks if MCP tokens and Fougere/Copilot CLI are available.
# ABOUTME: Outputs instructions for Claude to ask the user for missing credentials via AskUserQuestion.

missing_tokens=()

if [ -z "$LINEAR_API_KEY" ]; then
  missing_tokens+=("LINEAR_API_KEY (Linear MCP - for issue tracking)")
fi

if [ -z "$GITHUB_PERSONAL_ACCESS_TOKEN" ]; then
  missing_tokens+=("GITHUB_PERSONAL_ACCESS_TOKEN (GitHub MCP - for repo operations)")
fi

if [ ${#missing_tokens[@]} -gt 0 ]; then
  echo "MCP_TOKEN_SETUP_REQUIRED: The following MCP tokens are missing from the environment:"
  for token in "${missing_tokens[@]}"; do
    echo "  - $token"
  done
  echo "ACTION: Use AskUserQuestion to ask the user to provide these tokens. After receiving them, export each token using the Bash tool (e.g., export LINEAR_API_KEY=<value>). The tokens are needed by MCP servers configured in .mcp.json."
fi

# Check Fougere/Copilot CLI availability
if ! command -v copilot &>/dev/null; then
  echo "FOUGERE_SETUP_REQUIRED: The Copilot CLI is not installed (needed by the Fougere agent/skill)."
  echo "ACTION: Use AskUserQuestion to ask the user whether to install it (npm install -g @github/copilot). If they confirm, run the install via Bash, then authenticate with: copilot auth login --github-token <GITHUB_PERSONAL_ACCESS_TOKEN>."
elif [ ! -f "$HOME/.copilot/.fougere-authed" ]; then
  echo "FOUGERE_AUTH_REQUIRED: Copilot CLI is installed but not authenticated for Fougere."
  echo "ACTION: Use AskUserQuestion to ask the user for a GitHub token to authenticate Copilot CLI. Then run: copilot auth login --github-token <token>."
fi
