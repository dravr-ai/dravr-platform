// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The token links' shape — their landing paths and the token each carries in its fragment
// ABOUTME: Mirrors the paths and the #token= fragment the server builds for invitations, address confirmation and login takeovers

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
 * The login-takeover page: a sign-in a provider's robot check parked, for the
 * athlete to answer (ADR-034). The server builds the link (`TAKEOVER_PAGE` in
 * `crates/pierre-routes-auth/src/sciotte.rs`), so the two must name the same
 * path.
 */
export const TAKEOVER_PATH = '/sign-in-check';

/**
 * Every landing whose fragment (`#token=…`) carries a token, never a
 * dashboard route.
 */
export const TOKEN_LINK_PATHS: readonly string[] = [ACCEPT_INVITATION_PATH, VERIFY_EMAIL_PATH, TAKEOVER_PATH];

/** Read a mailed link's token from its fragment (`#token=…`), or `null`. */
export function parseLinkToken(hash: string): string | null {
  const token = new URLSearchParams(hash.replace(/^#/, '')).get('token');
  return token && token.trim() ? token.trim() : null;
}

/** What an invitation link says it is, for the accept page's wording. */
export type InvitationLinkKind = 'account' | 'coach_seat';

/**
 * Read an invitation link's kind from its fragment (`#token=…&kind=coach_seat`).
 * The server names it (`invitation_url` in `crates/pierre-services/src/invitation.rs`)
 * for a team-issued coach-seat invitation; any other link is a plain one. A
 * wording hint only: what the account is granted is the stored invitation's.
 */
export function parseInvitationKind(hash: string): InvitationLinkKind {
  return new URLSearchParams(hash.replace(/^#/, '')).get('kind') === 'coach_seat' ? 'coach_seat' : 'account';
}
