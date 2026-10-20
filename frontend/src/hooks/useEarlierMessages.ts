// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The older windows of a thread the reader scrolled back through, read one window at a time
// ABOUTME: Held beside the newest window's query, so a re-read of the newest never drops what was read back

import { useCallback, useRef, useState } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { prependWindow, type EarlierWindows } from '@pierre/chat-utils';
import type { Message } from '@pierre/shared-types';
import { chatApi } from '../services/api';
import { QUERY_KEYS } from '../constants/queryKeys';

export interface EarlierMessages {
  /** The windows read back so far, oldest first; `undefined` before the first. */
  earlier: EarlierWindows | undefined;
  /** A window is being read. */
  loading: boolean;
  /** Read the window before the oldest of `held`, every row the reader holds. */
  loadBefore: (held: readonly Message[]) => Promise<void>;
}

/**
 * The thread is read in windows from the newest back (carnet#840): the
 * newest window is `QUERY_KEYS.chat.messages`, and each scroll to the top
 * reads the one before the oldest row held. One read at a time; a failed one
 * leaves what is held and can be retried by scrolling back again.
 */
export function useEarlierMessages(conversationId: string | null): EarlierMessages {
  const queryClient = useQueryClient();
  const key = QUERY_KEYS.chat.earlierMessages(conversationId);
  // Nothing fetches this key: it holds what `loadBefore` wrote, for as long
  // as the thread's newest window is cached.
  const { data: earlier } = useQuery<EarlierWindows | null>({
    queryKey: key,
    queryFn: () => queryClient.getQueryData<EarlierWindows>(key) ?? null,
    enabled: false,
    staleTime: Infinity,
  });
  const [loading, setLoading] = useState(false);
  const inFlight = useRef(false);

  const loadBefore = useCallback(
    async (held: readonly Message[]) => {
      const oldestId = held[0]?.id;
      if (!conversationId || !oldestId || inFlight.current) return;
      inFlight.current = true;
      setLoading(true);
      try {
        const window = await chatApi.getConversationMessages(conversationId, { before: oldestId });
        queryClient.setQueryData<EarlierWindows>(
          QUERY_KEYS.chat.earlierMessages(conversationId),
          () => prependWindow(held, window),
        );
      } catch (err) {
        console.error('Failed to read earlier messages:', err);
      } finally {
        inFlight.current = false;
        setLoading(false);
      }
    },
    [conversationId, queryClient],
  );

  return { earlier: earlier ?? undefined, loading, loadBefore };
}
