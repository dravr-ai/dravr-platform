// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The group invite link an athlete opens — one builder for every mobile surface that shares a code
// ABOUTME: Points at the web app's /groups/join/:code, which both clients route into /group join; the path rule is the one web reads

import { groupInviteLink } from '@pierre/shared-constants';

/** Where a group invite link lands: the web app's public address. */
const INVITE_LINK_ORIGIN = 'https://app.dravr.ai';

/** The shareable link for an invite `code`. */
export function inviteLink(code: string): string {
  return groupInviteLink(INVITE_LINK_ORIGIN, code);
}
