// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Groups tab's hash subview — `chat/<id>`, or `chat/<id>/info` with the thread's info open — parsed back
// ABOUTME: A notice about a group's link opens the info, where the link waits for the athlete's answer

import { WEB_THREAD_INFO_SUBVIEW } from '@pierre/shared-constants';

/** The thread a Groups-tab subview names, and whether its info opens over it. */
export interface ChatSubview {
  conversationId: string;
  openInfo: boolean;
}

/**
 * The thread a Groups-tab subview names — `<id>`, or `<id>/info` to open its
 * info over it — or `null` for none, any other shape, or a malformed escape
 * in a hand-typed hash.
 */
export function parseChatSubview(sub: string): ChatSubview | null {
  const parts = sub.split('/');
  const openInfo = parts.length === 2 && parts[1] === WEB_THREAD_INFO_SUBVIEW;
  if (parts[0] === '' || (parts.length === 2 && !openInfo) || parts.length > 2) return null;
  try {
    return { conversationId: decodeURIComponent(parts[0]), openInfo };
  } catch (error) {
    if (error instanceof URIError) return null;
    throw error;
  }
}
