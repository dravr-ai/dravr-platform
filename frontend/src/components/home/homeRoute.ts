// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The hash route of a conversation opened on Home — `home/chat/<id>` — and its parse back
// ABOUTME: Home's activity view keeps `home/activity/<provider>/<id>`; the two never read as each other

const CONVERSATION_SEGMENT = 'chat';

/** The route that reopens `conversationId` on Home. */
export function homeConversationRoute(conversationId: string): string {
  return `home/${CONVERSATION_SEGMENT}/${encodeURIComponent(conversationId)}`;
}

/**
 * The conversation a Home subview names, or `null` for any other subview —
 * none, an activity's, or a malformed escape in a hand-typed hash.
 */
export function parseHomeConversation(sub: string): string | null {
  const parts = sub.split('/');
  if (parts.length !== 2 || parts[0] !== CONVERSATION_SEGMENT || parts[1] === '') return null;
  try {
    return decodeURIComponent(parts[1]);
  } catch (error) {
    if (error instanceof URIError) return null;
    throw error;
  }
}
