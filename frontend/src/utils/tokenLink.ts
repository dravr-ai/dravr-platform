// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The mailed single-use links' shape — their landing paths and the token each carries in its fragment
// ABOUTME: Mirrors the paths and the #token= fragment the server builds for invitations and address confirmation

/**
 * The invitation landing. The server builds the link (`invitation_url` in
 * `crates/pierre-services/src/invitation.rs`), so the two must name the same
 * path.
 */
export const ACCEPT_INVITATION_PATH = '/accept-invite';

/**
 * The address-confirmation landing. The server builds the link
 * (`confirmation_page_url` in `crates/pierre-routes-auth/src/email_verification.rs`),
 * so the two must name the same path.
 */
export const VERIFY_EMAIL_PATH = '/verify-email';

/**
 * Every landing whose fragment (`#token=…`) carries a single-use token, never
 * a dashboard route.
 */
export const TOKEN_LINK_PATHS: readonly string[] = [ACCEPT_INVITATION_PATH, VERIFY_EMAIL_PATH];

/** Read a mailed link's token from its fragment (`#token=…`), or `null`. */
export function parseLinkToken(hash: string): string | null {
  const token = new URLSearchParams(hash.replace(/^#/, '')).get('token');
  return token && token.trim() ? token.trim() : null;
}
