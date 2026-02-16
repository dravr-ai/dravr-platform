#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2025 Pierre Fitness Intelligence
# ABOUTME: Checks if MCP server tokens (Linear, GitHub) are set in the environment.
# ABOUTME: Outputs instructions for Claude to ask the user for missing tokens via AskUserQuestion.

missing=()

if [ -z "$LINEAR_API_KEY" ]; then
  missing+=("LINEAR_API_KEY (Linear MCP - for issue tracking)")
fi

if [ -z "$GITHUB_PERSONAL_ACCESS_TOKEN" ]; then
  missing+=("GITHUB_PERSONAL_ACCESS_TOKEN (GitHub MCP - for repo operations)")
fi

if [ ${#missing[@]} -gt 0 ]; then
  echo "MCP_TOKEN_SETUP_REQUIRED: The following MCP tokens are missing from the environment:"
  for token in "${missing[@]}"; do
    echo "  - $token"
  done
  echo "ACTION: Use AskUserQuestion to ask the user to provide these tokens. After receiving them, export each token using the Bash tool (e.g., export LINEAR_API_KEY=<value>). The tokens are needed by MCP servers configured in .mcp.json."
fi
