// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: React Query read behind Home's calendar — the cached workouts and the plan's weeks over one strip week or one month grid
// ABOUTME: Served from the server's cache: paging to an older week never starts a provider fetch or a backfill

import { keepPreviousData, useQuery } from '@tanstack/react-query';
import { useTranslation } from '@pierre/i18n';
import { QUERY_KEYS } from '@pierre/shared-constants';
import type { CalendarResponse } from '@pierre/shared-types';
import { athleteApi } from '../services/api';

/**
 * The athlete's days `from..=to` (`YYYY-MM-DD`), a plan day's evidence chip
 * and relabel line in the language the app renders. The span on screen stays
 * up while the next one pages in, so a tap on the arrows never blanks the
 * strip into a row that reads as empty days.
 */
export function useHomeCalendar(from: string, to: string) {
  const { language } = useTranslation();
  return useQuery<CalendarResponse>({
    queryKey: QUERY_KEYS.home.calendar(from, to, language),
    queryFn: () => athleteApi.getCalendar(from, to, language),
    placeholderData: keepPreviousData,
  });
}
