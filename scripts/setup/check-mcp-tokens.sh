#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 dravr.ai
# ABOUTME: Checks if MCP tokens are available.
# ABOUTME: Outputs instructions for Claude to ask the user for missing credentials via AskUserQuestion.

missing_tokens=()

if [ -z "$GITHUB_PERSONAL_ACCESS_TOKEN" ]; then
  missing_tokens+=("GITHUB_PERSONAL_ACCESS_TOKEN (GitHub MCP - for repo operations)")
fi

if [ ${#missing_tokens[@]} -gt 0 ]; then
  echo "MCP_TOKENS_MISSING: The following MCP tokens are not in the environment:"
  for token in "${missing_tokens[@]}"; do
    echo "  - $token"
  done
  echo "NOTE: DO NOT ask the user for these tokens. MCP tools that require them will be unavailable — use fallback methods instead (e.g., WebFetch for GitHub). Only mention missing tokens if the user explicitly tries to use a feature that requires them."
fi
