// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The ids support needs to find one claim verdict, spelled as the text an athlete copies for them
// ABOUTME: One spelling for web and phone, so a pasted reference reads the same whichever client copied it

import type { ClaimVerdict } from '@pierre/shared-types';

/**
 * The reference an athlete hands support for one verdict: the verdict's id,
 * then the message and conversation it was drawn from when the row carries
 * them, one labelled line each.
 */
export function verdictSupportReference(
  verdict: Pick<ClaimVerdict, 'id' | 'message_id' | 'conversation_id'>,
): string {
  const lines = [`verdict ${verdict.id}`];
  if (verdict.message_id) lines.push(`message ${verdict.message_id}`);
  if (verdict.conversation_id) lines.push(`conversation ${verdict.conversation_id}`);
  return lines.join('\n');
}
