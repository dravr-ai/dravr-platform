// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: React Query read of one workout's view — its Home row, the figures the cache holds, its splits, laps and thread
// ABOUTME: Bound to each client's athlete API; a workout the athlete does not hold is said as such and never asked again

import { useQuery, useQueryClient } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import type { AthleteApi } from '@pierre/api-client';
import type { ActivityDetailResponse } from '@pierre/shared-types';
import { classifyApiError } from './apiError';
import { pendingActivityConversationLink } from './activityConversation';

/** How many times a failed read is asked again before the view says so. */
const DETAIL_READ_RETRIES = 2;

/** What an activity's view reads from the detail query. */
export interface ActivityDetailState {
  data: ActivityDetailResponse | undefined;
  /** The server answered that the athlete holds no such activity. */
  notFound: boolean;
  /** The read failed for any other reason. */
  failed: boolean;
  refetch: () => void;
}

/**
 * Build `useActivityDetail` over one client's athlete API.
 *
 * The read is served from the server's activity cache, which reaches a
 * provider only for the workout's one detail read, so a completed activity's
 * answer is kept for the session — and
 * read again each time the view opens, since it also names the thread the
 * view opened about the activity, which another device can have linked
 * meanwhile. The kept answer shows at once while that read runs. A 404 is an
 * answer, not a fault: it is not asked again, and the view says the activity
 * is not among the athlete's. A read that answers while the view's thread
 * link is still on its way keeps the thread being linked.
 */
export function createActivityDetailHook(athleteApi: Pick<AthleteApi, 'getActivityDetail'>) {
  return function useActivityDetail(provider: string, activityId: string): ActivityDetailState {
    const queryClient = useQueryClient();
    const query = useQuery<ActivityDetailResponse>({
      queryKey: QUERY_KEYS.home.activityDetail(provider, activityId),
      queryFn: async ({ signal }) => {
        const detail = await athleteApi.getActivityDetail(provider, activityId, { signal });
        // A thread link still on its way is newer than this answer; the
        // link's own settling reads the detail again.
        const pending = pendingActivityConversationLink(queryClient, provider, activityId);
        return pending === null ? detail : { ...detail, conversation_id: pending.conversationId };
      },
      staleTime: Infinity,
      refetchOnMount: 'always',
      retry: (failures, error) => classifyApiError(error).kind !== 'notFound' && failures < DETAIL_READ_RETRIES,
    });
    const notFound = query.isError && classifyApiError(query.error).kind === 'notFound';
    return {
      data: query.data,
      notFound,
      failed: query.isError && !notFound,
      refetch: () => void query.refetch(),
    };
  };
}
