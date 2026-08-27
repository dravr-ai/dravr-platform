// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The two ways an installed coach is used from any chat, as the post-install hint teaches them
// ABOUTME: `/coach add @handle` binds it to the conversation; `@handle` in a message brings it in for one turn

import { COMMAND_DRAFTS, MENTION_PREFIX } from '@pierre/shared-constants';

/**
 * What the hint shows in place of a handle when the payload carries none.
 * Every published listing is approved with a handle and an installed copy
 * inherits it, so the placeholder reads as the literal word rather than an
 * empty name.
 */
const PLACEHOLDER_HANDLE = 'handle';

/** The `@handle` the hint teaches. */
export function coachMention(handle: string | undefined): string {
  return `${MENTION_PREFIX}${handle || PLACEHOLDER_HANDLE}`;
}

/** The command that binds an installed coach to the open conversation. */
export function coachAddDraft(handle: string | undefined): string {
  return COMMAND_DRAFTS.coachAdd(handle || PLACEHOLDER_HANDLE);
}
