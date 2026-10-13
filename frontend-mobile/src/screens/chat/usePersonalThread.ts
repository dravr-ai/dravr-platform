// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The athlete's personal thread on mobile — the one the server resolves, and the topics "+" opens in it
// ABOUTME: New chat, a Store or onboarding pick and a Today tap all land in it, so the app never forges a thread of its own

import { useCallback } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import type { OpenTopicOptions } from '@pierre/api-client';
import { chatApi } from '../../services/api';

/**
 * Find the athlete's personal thread, by id, or `null` when they have none.
 *
 * The server names it — adopting the athlete's newest thread of their own the
 * first time — so this app, the web and the coach's own posts agree on which
 * thread it is. A look forges nothing: an athlete who has not begun gets an
 * empty composer, whose first send opens their thread. Asked on every use, so
 * one the coach's own post opened since is found.
 */
export function useFindPersonalThread(): () => Promise<string | null> {
  const queryClient = useQueryClient();
  return useCallback(async () => {
    const thread = await queryClient.fetchQuery({
      queryKey: QUERY_KEYS.chat.personal(),
      queryFn: () => chatApi.getPersonalConversation(),
    });
    return thread?.id ?? null;
  }, [queryClient]);
}

/**
 * Open a new topic in the athlete's personal thread and answer its id.
 *
 * What "New chat" means: a divider the coach reads the thread afresh from,
 * rebound to the agent `options` names, whose welcome opens the topic.
 * The thread's rows and the list are re-read, since both changed.
 */
export function useOpenPersonalTopic(): (options?: OpenTopicOptions) => Promise<string> {
  const queryClient = useQueryClient();
  return useCallback(
    async (options?: OpenTopicOptions) => {
      const opened = await chatApi.openPersonalTopic(options);
      const id = opened.conversation.id;
      queryClient.setQueryData(QUERY_KEYS.chat.personal(), opened.conversation);
      void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.conversations() });
      void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.messages(id) });
      return id;
    },
    [queryClient],
  );
}
