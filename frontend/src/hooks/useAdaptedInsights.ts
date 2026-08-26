// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: React Query hook paging the user's adapted-insight history on web
// ABOUTME: Mirrors the mobile AdaptedInsightsScreen cursor paging over socialApi.getAdaptedInsights

import { useInfiniteQuery } from '@tanstack/react-query';
import { useMemo } from 'react';
import type { AdaptedInsight, ListAdaptedInsightsResponse } from '@pierre/shared-types';
import { socialApi } from '../services/api';
import { QUERY_KEYS } from '../constants/queryKeys';

/** Page size used for both the first page and every "load more". Matches the
 * mobile screen so the two surfaces walk the same offsets. */
export const ADAPTED_INSIGHTS_PAGE_SIZE = 20;

/** Shape returned by `useAdaptedInsights`. */
export interface UseAdaptedInsightsResult {
  /** Every page flattened, oldest page first, in server order. */
  insights: AdaptedInsight[];
  isLoading: boolean;
  isError: boolean;
  /** True while a "load more" request is in flight. */
  isFetchingNextPage: boolean;
  /** True when the server reported another page after the last one loaded. */
  hasNextPage: boolean;
  fetchNextPage: () => void;
}

/**
 * Page the adapted-insight history. The cursor a page carries is the offset the
 * next page starts at — `socialApi.getAdaptedInsights` sends it as `offset`,
 * which is the parameter the endpoint actually reads.
 *
 * Pass `enabled: false` to hold the first request until the history is on screen.
 */
export function useAdaptedInsights(options?: { enabled?: boolean }): UseAdaptedInsightsResult {
  const query = useInfiniteQuery<ListAdaptedInsightsResponse>({
    queryKey: QUERY_KEYS.social.adapted(),
    // The history is its own view; nothing fetches until the user opens it.
    enabled: options?.enabled ?? true,
    queryFn: ({ pageParam }) =>
      socialApi.getAdaptedInsights({
        limit: ADAPTED_INSIGHTS_PAGE_SIZE,
        cursor: typeof pageParam === 'string' ? pageParam : undefined,
      }),
    initialPageParam: undefined as string | undefined,
    // `has_more` false means the server has nothing past this page, so stop
    // even if it echoed a cursor back.
    getNextPageParam: (lastPage) => (lastPage.has_more ? lastPage.next_cursor ?? undefined : undefined),
  });

  const insights = useMemo(
    () => (query.data?.pages ?? []).flatMap((page) => page.insights ?? []),
    [query.data],
  );

  return {
    insights,
    isLoading: query.isLoading,
    isError: query.isError,
    isFetchingNextPage: query.isFetchingNextPage,
    hasNextPage: query.hasNextPage,
    fetchNextPage: () => {
      void query.fetchNextPage();
    },
  };
}
