// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The phone's reads and writes for the human coach's reply review — the replies page by page, the week's counts, a flag and a correction
// ABOUTME: Same calls and same cache keys as the web Replies page; a write re-reads the review, and a correction also the group's room (carnet#1304)

import { useCallback, useMemo } from 'react';
import { useInfiniteQuery, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { COACH_REPLIES_PAGE_SIZE, QUERY_KEYS } from '@pierre/shared-constants';
import type {
  CoachReply,
  CoachRepliesPage,
  CoachReviewWeek,
  CorrectReplyResponse,
  FlagReplyResponse,
  ReplyFlagReason,
} from '@pierre/shared-types';
import { coachRepliesApi } from '../services/api';

/** What `useCoachReplies` hands the screen. */
export interface UseCoachRepliesResult {
  /** The first page, which carries the groups and the athlete filter's choices. */
  first: CoachRepliesPage | undefined;
  /** Every reply read so far, newest first. */
  replies: CoachReply[];
  /** This week's flags and corrections; undefined until read, or when it failed. */
  thisWeek: CoachReviewWeek | undefined;
  isLoading: boolean;
  isError: boolean;
  hasNextPage: boolean;
  isFetchingNextPage: boolean;
  fetchNextPage: () => void;
  refetch: () => Promise<void>;
}

/** The reply review narrowed to one athlete, or to all of them with `null`. */
export function useCoachReplies(athleteId: string | null): UseCoachRepliesResult {
  const pages = useInfiniteQuery({
    queryKey: QUERY_KEYS.coachReplies.list(athleteId),
    queryFn: ({ pageParam }) =>
      coachRepliesApi.list({
        athlete_id: athleteId ?? undefined,
        before: pageParam ?? undefined,
        limit: COACH_REPLIES_PAGE_SIZE,
      }),
    initialPageParam: null as string | null,
    getNextPageParam: (last) => last.next_before,
  });
  const weekly = useQuery({
    queryKey: QUERY_KEYS.coachReplies.weekly(),
    queryFn: () => coachRepliesApi.weekly(1),
  });
  const replies = useMemo(() => pages.data?.pages.flatMap((page) => page.replies) ?? [], [pages.data]);
  const { refetch: refetchPages } = pages;
  const { refetch: refetchWeekly } = weekly;
  const refetch = useCallback(async () => {
    await Promise.all([refetchPages(), refetchWeekly()]);
  }, [refetchPages, refetchWeekly]);

  return {
    first: pages.data?.pages[0],
    replies,
    thisWeek: weekly.data?.weeks.at(-1),
    isLoading: pages.isLoading,
    isError: pages.isError,
    hasNextPage: pages.hasNextPage,
    isFetchingNextPage: pages.isFetchingNextPage,
    fetchNextPage: () => void pages.fetchNextPage(),
    refetch,
  };
}

/** The two writes on one reply. */
export interface ReplyWrites {
  flag: {
    mutateAsync: (input: { reason: ReplyFlagReason; note: string }) => Promise<FlagReplyResponse>;
    isPending: boolean;
  };
  correct: {
    mutateAsync: (content: string) => Promise<CorrectReplyResponse>;
    isPending: boolean;
  };
}

/** Flag or correct `reply`; either one re-reads the review, and a correction the group's room too. */
export function useReplyWrites(reply: Pick<CoachReply, 'id' | 'group_id'>): ReplyWrites {
  const queryClient = useQueryClient();
  const refresh = () => queryClient.invalidateQueries({ queryKey: QUERY_KEYS.coachReplies.all });

  const flag = useMutation({
    mutationFn: ({ reason, note }: { reason: ReplyFlagReason; note: string }) =>
      coachRepliesApi.flag(reply.id, reason, note),
    onSuccess: refresh,
  });
  const correct = useMutation({
    mutationFn: (content: string) => coachRepliesApi.correct(reply.id, content.trim()),
    onSuccess: async () => {
      // The athlete's thread shows the correction; a thread this phone has
      // open reads the room again.
      void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.groups.room(reply.group_id) });
      await refresh();
    },
  });
  return { flag, correct };
}
