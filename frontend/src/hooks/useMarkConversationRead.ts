// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Advances the caller's read marker on the open thread and zeroes its row's unread count optimistically
// ABOUTME: Fires when a thread with unread rows opens and whenever its newest message changes while the tab is visible

import { useEffect, useRef, useState } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { chatApi } from '../services/api';
import { QUERY_KEYS } from '../constants/queryKeys';
import { cachedConversations, patchCachedConversation } from './useConversationList';

function documentVisible(): boolean {
  return typeof document === 'undefined' || document.visibilityState === 'visible';
}

/**
 * Keep the read marker of the open thread in step with what the athlete has
 * on screen.
 *
 * Two triggers, one rule. Opening a thread marks it read once its messages
 * resolve — but only when the cached row says something is unread, so a
 * thread reopened after it was already read costs no request. From then on,
 * every change of the newest message — a reply landing at the end of a turn,
 * a message that arrived from Telegram and came in on refetch — marks again,
 * unconditionally: a new row is unread by definition. A change that lands
 * while the tab is hidden waits for the tab to come back; the athlete has
 * not read what they cannot see.
 *
 * The row's `unread_count` is zeroed in the cache the moment the marker is
 * sent, so the list and the nav badge agree with the open thread without
 * waiting for the refetch that settles the mutation.
 *
 * `latestMessageId` is the newest persisted row of the thread — the caller
 * leaves the optimistic user row it appends during a turn out of it, since
 * that row has no server id to mark up to.
 */
export function useMarkConversationRead(
  conversationId: string | null,
  latestMessageId: string | null,
): void {
  const queryClient = useQueryClient();
  const [visible, setVisible] = useState(documentVisible);
  // The last `conversation:message` pair marked, so the same message never
  // marks twice and a switch back to a thread counts as an open.
  const lastMarked = useRef<string | null>(null);

  useEffect(() => {
    if (typeof document === 'undefined') return;
    const onVisibilityChange = () => setVisible(documentVisible());
    document.addEventListener('visibilitychange', onVisibilityChange);
    return () => document.removeEventListener('visibilitychange', onVisibilityChange);
  }, []);

  const { mutate } = useMutation({
    mutationFn: (id: string) => chatApi.markConversationRead(id),
    onMutate: (id) => {
      patchCachedConversation(queryClient, id, { unread_count: 0 });
    },
    onSettled: () => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.conversations() });
    },
  });

  useEffect(() => {
    if (!conversationId || !latestMessageId || !visible) return;
    const key = `${conversationId}:${latestMessageId}`;
    if (lastMarked.current === key) return;

    // First observation of this thread since it was opened: skip when the
    // list already knows there is nothing to read. A later message change
    // on the same thread always marks.
    const isOpen = lastMarked.current?.startsWith(`${conversationId}:`) ?? false;
    lastMarked.current = key;
    if (!isOpen) {
      const row = cachedConversations(queryClient).find((c) => c.id === conversationId);
      if (row && (row.unread_count ?? 0) === 0) return;
    }
    mutate(conversationId);
  }, [conversationId, latestMessageId, visible, mutate, queryClient]);
}
