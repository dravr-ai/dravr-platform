// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Settings › Physiology on both clients — the React Query read of the athlete's anchors, and the facts under each one
// ABOUTME: The server writes an anchor's value and source; this decides the line under its label: its day, its range, its confidence

import { useQuery, type UseQueryResult } from '@tanstack/react-query';
import { ANCHOR_CONFIDENCE_LABEL_KEY, PHYSIOLOGY_KEY, QUERY_KEYS } from '@pierre/shared-constants';
import type { AthleteApi } from '@pierre/api-client';
import type { PhysiologyAnchor, PhysiologyResponse } from '@pierre/shared-types';
import { formatCivilDate, type Translate } from './homeFormat';
import { useDistanceUnit } from './unitsHook';

/** How an anchor's day reads: "20 Oct 2026", in the app's language. */
const ANCHOR_DATE: Intl.DateTimeFormatOptions = { day: 'numeric', month: 'short', year: 'numeric' };

/** What only the client knows: the language the app renders in, read with its own hook. */
export interface PhysiologyPlatform {
  useLanguage: () => string;
}

/**
 * Build `usePhysiology` over one client's athlete API.
 *
 * The server writes each anchor's value and source in the language asked
 * for and the athlete's units, so both are in the query key: switching
 * either in Settings reads the anchors again rather than showing the words
 * of the old setting.
 */
export function createPhysiologyHook(
  athleteApi: Pick<AthleteApi, 'getPhysiology'>,
  { useLanguage }: PhysiologyPlatform,
) {
  return function usePhysiology(): UseQueryResult<PhysiologyResponse> {
    const language = useLanguage();
    const units = useDistanceUnit();
    return useQuery<PhysiologyResponse>({
      queryKey: QUERY_KEYS.user.physiology(language, units),
      queryFn: () => athleteApi.getPhysiology(language),
    });
  };
}

/**
 * The facts under an anchor's label, in reading order: the day it was set,
 * or that none is recorded; the range it states, when its value reads as a
 * point; and how far its writer said to trust it, as stored.
 */
export function physiologyAnchorFacts(t: Translate, anchor: PhysiologyAnchor, language: string): string[] {
  const facts = [
    anchor.as_of === null
      ? t(PHYSIOLOGY_KEY.noDate)
      : t(PHYSIOLOGY_KEY.asOf, { date: formatCivilDate(anchor.as_of, language, ANCHOR_DATE) }),
  ];
  if (anchor.range_text !== null) {
    facts.push(t(PHYSIOLOGY_KEY.range, { range: anchor.range_text }));
  }
  facts.push(t(ANCHOR_CONFIDENCE_LABEL_KEY[anchor.confidence]));
  return facts;
}
