// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Whether a stored reply may be re-run: only when the server says its turn changed nothing
// ABOUTME: One rule for web and phone, since re-running sends the question as a new turn on a fresh budget

import type { Message } from '@pierre/shared-types';

/**
 * Whether to offer Regenerate on a stored reply.
 *
 * Regenerating sends the question again as a brand-new turn on a fresh Guardian
 * budget, so it can repeat a write or complete one the Guardian just blocked.
 * Only an explicit `false` is safe: a row that does not say was written before
 * the field existed, or by something other than a model turn.
 *
 * A failed turn the server never stored is a different path, and does not come
 * through here: it offers its own Retry.
 */
export function canRegenerateReply(reply: Pick<Message, 'turn_attempted_write'>): boolean {
  return reply.turn_attempted_write === false;
}
