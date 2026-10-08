// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: React Query read behind Home's calendar — the cached workouts and the plan's weeks over one strip week or one month grid
// ABOUTME: Served from the server's cache: paging to an older week never starts a provider fetch or a backfill

import { keepPreviousData, useQuery } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import type { CalendarResponse } from '@pierre/shared-types';
import { athleteApi } from '../services/api';

/**
 * The athlete's days `from..=to` (`YYYY-MM-DD`). `response` is null until
 * the answer for exactly these days arrives — the previous span's answer
 * kept on screen while paging is never read as these days' — while
 * `historyStart` carries over, so the back arrow does not flicker on.
 */
export function useHomeCalendar(from: string, to: string) {
  const query = useQuery<CalendarResponse>({
    queryKey: QUERY_KEYS.home.calendar(from, to),
    queryFn: () => athleteApi.getCalendar(from, to),
    placeholderData: keepPreviousData,
  });
  return {
    response: query.isPlaceholderData ? null : (query.data ?? null),
    historyStart: query.data?.history_start ?? null,
    isError: query.isError,
    refetch: query.refetch,
  };
}
