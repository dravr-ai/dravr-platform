// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Reads a refused new chat for the active-conversation cap (a 429 naming max_active_conversations)
// ABOUTME: Shared by every surface that opens a conversation, so each says the same thing at the cap

import { classifyApiError } from '@pierre/ui-logic';

/**
 * The active-conversation cap a refused new chat ran into: the cap the server
 * named (`limit`, possibly unknown), or `null` when the refusal was anything
 * else. Keyed on the limit the server named, not on the status: a spent
 * request budget is a 429 too, and it is not the conversation cap. The
 * refusal is read by `classifyApiError`, the one parser both clients share,
 * so web and mobile agree on what a cap refusal is.
 */
export function conversationCapOf(error: unknown): { limit: number | null } | null {
  const { quota } = classifyApiError(error);
  if (quota?.limit_type !== 'max_active_conversations') return null;
  return { limit: quota.limit ?? null };
}
