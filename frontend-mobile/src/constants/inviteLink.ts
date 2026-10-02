// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The group invite link an athlete opens — one builder for every mobile surface that shares a code
// ABOUTME: Points at the web app's /groups/join/:code, which both clients route into /group join

/** Where a group invite link lands: the web app's join route. */
const INVITE_LINK_BASE = 'https://app.dravr.ai/groups/join';

/** The shareable link for an invite `code`. */
export function inviteLink(code: string): string {
  return `${INVITE_LINK_BASE}/${encodeURIComponent(code)}`;
}
