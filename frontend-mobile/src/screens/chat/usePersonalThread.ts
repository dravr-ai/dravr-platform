// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The athlete's personal thread on mobile — the one the server resolves — and the new chats opened beside it
// ABOUTME: Onboarding and a Today tap land in the personal thread; "+" and a Store pick open a thread of their own once it exists

import { useCallback } from 'react';
import { Alert } from 'react-native';
import { useQueryClient } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import type { CreateConversationOptions, OpenTopicOptions } from '@pierre/api-client';
import { classifyApiError, describeApiError, type ApiErrorTranslate } from '@pierre/ui-logic';
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
 * A divider the coach reads the thread afresh from, rebound to the agent
 * `options` names, whose welcome opens the topic. Onboarding's Start takes
 * this path, and so does a Store pick for an athlete who has no personal
 * thread yet — their first chat becomes it. Never "+" on a thread that
 * exists: that would rebind its agent (carnet#1473).
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

/**
 * Create a new conversation beside the personal thread and answer its id.
 *
 * What "New chat" means once the athlete has a personal thread (carnet#1473):
 * a thread of its own, bound to the agent `options` names or the athlete's
 * selected one, whose welcome the server has posted before it is read. The
 * personal thread is left as it was — no divider, no rebind — so the athlete
 * can talk to more than one agent. The list is re-read to show the new row.
 */
export function useCreateConversation(): (options?: CreateConversationOptions) => Promise<string> {
  const queryClient = useQueryClient();
  return useCallback(
    async (options?: CreateConversationOptions) => {
      const created = await chatApi.createConversation(options);
      void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.conversations() });
      return created.id;
    },
    [queryClient],
  );
}

/**
 * Tell the athlete a new chat could not be opened.
 *
 * The server caps active conversations and refuses one more with a 429 naming
 * `max_active_conversations`; that refusal says so, with the counts, since
 * deleting a conversation is the way out. Anything else is the plain
 * "could not open a chat".
 */
export function alertNewChatFailure(err: unknown, t: ApiErrorTranslate): void {
  if (classifyApiError(err).quota?.limit_type === 'max_active_conversations') {
    Alert.alert(
      t('chat.conversationLimitTitle'),
      describeApiError(err, { t, fallbackKey: 'app.couldNotOpenChat' }),
    );
    return;
  }
  Alert.alert(t('common.error'), t('app.couldNotOpenChat'));
}
