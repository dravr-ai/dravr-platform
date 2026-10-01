// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Which conversation an activity's view opened — read from the server's activity detail, linked there by the view
// ABOUTME: Reopening the activity on any device resumes that thread, so every question about one workout lands in one place

import { useCallback } from 'react';
import { useMutation, useQueryClient, type QueryClient } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import type { AthleteApi } from '@pierre/api-client';
import type { ActivityDetailResponse } from '@pierre/shared-types';

/** How many times a link the server did not take is sent again. */
const LINK_WRITE_RETRIES = 2;

/** The thread an activity's view shows, and how the view records a new one. */
export interface ActivityConversationState {
  /** The conversation opened about this activity, or `null` before its first question. */
  conversationId: string | null;
  /** Record the thread the view opened (or moved to); `null` forgets it. */
  setConversationId: (conversationId: string | null) => void;
}

/**
 * The mutation key of one activity's thread link. The detail read looks for a
 * link still on its way under this key, so a read that answers before the
 * `PUT` lands does not put back the thread the server held before it.
 */
export function activityConversationLinkKey(provider: string, activityId: string) {
  return ['activityConversationLink', provider, activityId] as const;
}

/** The thread a link still on its way is writing for one activity, when one is pending. */
export function pendingActivityConversationLink(
  queryClient: QueryClient,
  provider: string,
  activityId: string
): { conversationId: string | null } | null {
  const pending = queryClient.getMutationCache().findAll({
    mutationKey: activityConversationLinkKey(provider, activityId),
    exact: true,
    status: 'pending',
  });
  const latest = pending.at(-1);
  if (latest === undefined) return null;
  return { conversationId: latest.state.variables as string | null };
}

/**
 * Build `useActivityConversation` over one client's athlete API.
 *
 * The link lives on the server: the activity's detail read names the thread
 * (`conversation_id`), and the view's first question — or a `/reset` that
 * moved the thread — links the new one with `PUT …/conversation`. So leaving
 * the view and coming back, reloading, or opening the activity on another
 * device resumes the same thread. The detail entry in the query cache is
 * updated at once, so the thread the view just opened is the one it shows
 * while the link is on its way; a detail read in flight is cancelled, and one
 * that starts meanwhile keeps the pending thread
 * ({@link pendingActivityConversationLink}). A link the server still refuses
 * after its retries puts back the thread the view showed before, which is
 * the one the server holds, and the detail is read again once the link
 * settles either way.
 */
export function createActivityConversationHook(
  athleteApi: Pick<AthleteApi, 'linkActivityConversation'>
) {
  return function useActivityConversation(
    provider: string,
    activityId: string,
    detail: ActivityDetailResponse
  ): ActivityConversationState {
    const queryClient = useQueryClient();
    const linked = detail.conversation_id;
    const detailKey = QUERY_KEYS.home.activityDetail(provider, activityId);
    const { mutate: link } = useMutation({
      mutationKey: activityConversationLinkKey(provider, activityId),
      mutationFn: (conversationId: string | null) =>
        athleteApi.linkActivityConversation(provider, activityId, conversationId),
      retry: LINK_WRITE_RETRIES,
      onMutate: async (conversationId) => {
        await queryClient.cancelQueries({ queryKey: detailKey, exact: true });
        const previous = queryClient.getQueryData<ActivityDetailResponse>(detailKey)?.conversation_id ?? null;
        queryClient.setQueryData<ActivityDetailResponse>(detailKey, (current) =>
          current ? { ...current, conversation_id: conversationId } : current
        );
        return { previous };
      },
      onError: (_error, conversationId, context) => {
        if (context === undefined) return;
        // Only while the refused thread is still the one shown: a later link
        // (a `/reset` that moved the thread) owns the entry from then on.
        queryClient.setQueryData<ActivityDetailResponse>(detailKey, (current) =>
          current && current.conversation_id === conversationId
            ? { ...current, conversation_id: context.previous }
            : current
        );
      },
      onSettled: () => queryClient.invalidateQueries({ queryKey: detailKey, exact: true }),
    });
    const setConversationId = useCallback(
      (conversationId: string | null) => {
        if (conversationId === linked) return;
        link(conversationId);
      },
      [linked, link]
    );
    return { conversationId: linked, setConversationId };
  };
}
