// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The window both Settings API key panes count each key's use over, and the consent shown where a credential is made
// ABOUTME: A key's tier and budget are the server's call, from the minting user's plan

/** The window, in days, each key's "requests in the last 30 days" line counts over. */
export const API_KEY_USAGE_WINDOW_DAYS = 30;

/**
 * The express-consent words shown, always visible, beside the action that
 * creates a credential an outside AI tool connects with: a personal MCP token
 * or an API key (EFVP "AI Assistant Connections", decision 9.1b). Creating
 * the credential after reading them is the consent.
 */
export const CREDENTIAL_CONSENT_KEYS = {
  mcpToken: 'app.mcpTokenConsent',
  apiKey: 'settingsTabs.apiKeysConsent',
} as const;
