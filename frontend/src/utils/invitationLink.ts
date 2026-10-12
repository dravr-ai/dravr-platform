// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The mailed invitation link's shape — its landing path and the single-use token in its fragment
// ABOUTME: Mirrors ACCEPT_INVITATION_PATH and the #token= fragment the server builds in pierre_services::invitation

/**
 * The invitation landing. Its fragment (`#token=…`) carries a single-use
 * token, never a dashboard route. The server builds the link
 * (`invitation_url` in `crates/pierre-services/src/invitation.rs`), so the two
 * must name the same path.
 */
export const ACCEPT_INVITATION_PATH = '/accept-invite';

/** Read the invitation token from the link's fragment (`#token=…`), or `null`. */
export function parseInvitationToken(hash: string): string | null {
  const token = new URLSearchParams(hash.replace(/^#/, '')).get('token');
  return token && token.trim() ? token.trim() : null;
}
