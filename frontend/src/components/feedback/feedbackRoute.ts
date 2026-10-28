// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The feedback thread's hash routes — `feedback`, or `feedback/reply/<conversation>/<message>` about one agent reply
// ABOUTME: The thumbs-down link under a reply opens the second; a malformed hand-typed hash opens the plain thread

import type { FeedbackAbout } from '@pierre/shared-types';

/** The tab the feedback thread lives on. */
export const FEEDBACK_TAB = 'feedback';

const REPLY_SEGMENT = 'reply';

/** The route that opens the feedback thread about one agent reply. */
export function feedbackReplyRoute(conversationId: string, messageId: string): string {
  return `${FEEDBACK_TAB}/${REPLY_SEGMENT}/${encodeURIComponent(conversationId)}/${encodeURIComponent(messageId)}`;
}

/**
 * The reply a feedback subview is about, or `null` for the plain thread —
 * no subview, any other shape, or a malformed escape in a hand-typed hash.
 */
export function parseFeedbackAbout(sub: string): FeedbackAbout | null {
  const parts = sub.split('/');
  if (parts.length !== 3 || parts[0] !== REPLY_SEGMENT || parts[1] === '' || parts[2] === '') {
    return null;
  }
  try {
    return { conversation_id: decodeURIComponent(parts[1]), message_id: decodeURIComponent(parts[2]) };
  } catch (error) {
    if (error instanceof URIError) return null;
    throw error;
  }
}
