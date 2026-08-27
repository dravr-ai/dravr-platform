// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Advances the open thread's read marker while the athlete is actually looking at it
// ABOUTME: Fires on open, on every new last message and after a turn lands; clears the row's badge first

import { useCallback, useEffect, useRef, useState } from 'react';
import { AppState, type AppStateStatus } from 'react-native';
import { useIsFocused } from 'expo-router';
import { useQueryClient } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import { chatApi } from '../../services/api';
import { patchCachedConversation } from '../conversations/useConversationList';

/** What the thread hands the read marker. */
export interface UseMarkConversationReadOptions {
  /** The open conversation, or null while the composer has no thread yet. */
  conversationId: string | null;
  /**
   * The newest message the thread holds. A new id is new unread material —
   * the transcript finished loading, a turn landed, or a messaging reply
   * arrived — and is what makes the marker move again.
   */
  lastMessageId: string | null;
}

/** True while the app is in the foreground; false in background or inactive. */
function useAppIsActive(): boolean {
  const [isActive, setIsActive] = useState(() => AppState.currentState === 'active');

  useEffect(() => {
    const subscription = AppState.addEventListener('change', (status: AppStateStatus) => {
      setIsActive(status === 'active');
    });
    return () => subscription.remove();
  }, []);

  return isActive;
}

/**
 * Mark the open thread read, the way a messaging app does it.
 *
 * Reading is looking: the marker only moves while this screen is the focused
 * route *and* the app is in the foreground, so a thread left open under a
 * locked phone does not quietly swallow the messages that arrive on it. It
 * moves again on every new last message, which covers all three moments the
 * athlete sees new material — the transcript loading, their own turn landing,
 * and a reply arriving from a messaging channel.
 *
 * The cached row drops to zero unread before the request goes out, so the
 * list and the tab badge clear the moment the thread opens; the invalidate
 * that follows replaces the optimistic row with the server's own count.
 */
export function useMarkConversationRead({
  conversationId,
  lastMessageId,
}: UseMarkConversationReadOptions): void {
  const queryClient = useQueryClient();
  const isFocused = useIsFocused();
  const isActive = useAppIsActive();
  // The pair already sent, so a re-render on the same transcript does not
  // re-post the marker the server has already advanced past.
  const markedRef = useRef<string | null>(null);

  const markRead = useCallback(
    async (id: string) => {
      patchCachedConversation(queryClient, id, (conversation) => ({ ...conversation, unread_count: 0 }));
      try {
        await chatApi.markConversationRead(id);
      } catch (err) {
        // The marker is an optimisation over what the transcript already
        // shows: a failed advance costs a stale badge until the next read,
        // never a message. Let the next fire retry it.
        markedRef.current = null;
        console.error('Failed to mark conversation read:', err);
      }
      await queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.conversations() });
    },
    [queryClient],
  );

  useEffect(() => {
    if (!conversationId || !lastMessageId || !isFocused || !isActive) return;
    const token = `${conversationId}:${lastMessageId}`;
    if (markedRef.current === token) return;
    markedRef.current = token;
    void markRead(conversationId);
  }, [conversationId, lastMessageId, isFocused, isActive, markRead]);
}
