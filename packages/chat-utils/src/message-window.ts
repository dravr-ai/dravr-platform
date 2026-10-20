// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A thread read in windows — the older windows a reader scrolled back through joined to the newest one
// ABOUTME: Plus the channel tag a row of the shared personal thread carries when it came from a messaging app

import type { Message } from '@pierre/shared-types';
import { resolveChannelOrigin, type MessageChannelOrigin } from './conversation';

/**
 * The windows of a thread read back so far, oldest first, and whether more
 * remain before them.
 */
export interface EarlierWindows {
  /** Every row of the windows read back, oldest first. */
  messages: Message[];
  /** Whether older rows remain before the first one held. */
  hasMore: boolean;
}

/**
 * The thread as a reader holds it: the older windows they scrolled back
 * through, then the newest window. The newest window is re-read on its own —
 * a reply lands, the tab regains focus — so a row both hold is drawn once,
 * from the newest window, which carries its latest state.
 */
export function joinMessageWindows(
  earlier: readonly Message[] | undefined,
  newest: readonly Message[] | undefined,
): Message[] {
  const latest = newest ?? [];
  if (!earlier || earlier.length === 0) return [...latest];
  const held = new Set(latest.map((m) => m.id));
  return [...earlier.filter((m) => !held.has(m.id)), ...latest];
}

/**
 * Whether the reader can scroll back further: what the oldest window read
 * says once one has been read, else what the newest window says.
 */
export function hasEarlierMessages(
  earlier: EarlierWindows | undefined,
  newestHasMore: boolean | undefined,
): boolean {
  if (earlier && earlier.messages.length > 0) return earlier.hasMore;
  return newestHasMore === true;
}

/**
 * Put a window read back before every row the reader holds.
 *
 * `held` is the whole thread as held now — the newest window included —
 * because the newest window is re-read on its own and moves on as rows land:
 * kept only as the older windows, the rows between them and a newer newest
 * window would drop out of the thread.
 */
export function prependWindow(
  held: readonly Message[],
  window: { messages: Message[]; has_more?: boolean },
): EarlierWindows {
  const ids = new Set(held.map((m) => m.id));
  return {
    messages: [...window.messages.filter((m) => !ids.has(m.id)), ...held],
    hasMore: window.has_more === true,
  };
}

/**
 * The messaging channel an athlete's message came over, for the tag a row of
 * the personal thread draws: the thread is shared by the app and the
 * athlete's DM, so their Telegram words read as such. `null` for a row typed
 * in the app, a row with no channel, and every row that is not theirs.
 */
export function messageChannelOrigin(
  message: Pick<Message, 'role' | 'channel_type'>,
): MessageChannelOrigin | null {
  if (message.role !== 'user') return null;
  return resolveChannelOrigin({ channel_type: message.channel_type });
}
