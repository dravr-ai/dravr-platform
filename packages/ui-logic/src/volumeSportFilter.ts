// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The sport picked on Home's volume card, held above the card so the recent-activities list filters on it too
// ABOUTME: One choice per page on web and mobile alike; a sport that drops out of the weeks falls back to every sport

import { useMemo, useState } from 'react';
import { ALL_SPORTS } from '@pierre/shared-constants';
import { volumeSports, type VolumeWeekInput } from '@pierre/domain-utils';

/** The sport filter Home's volume card draws and its activity list reads. */
export interface VolumeSportFilter {
  /** The sports the weeks hold, most time first. */
  sports: string[];
  /** The tab shown as active: a sport, or {@link ALL_SPORTS}. */
  selected: string;
  /** The sport to filter on, `null` for every sport. */
  sport: string | null;
  /** Pick a tab. */
  choose: (tab: string) => void;
}

/**
 * The volume card's sport choice over `weeks`, the weeks the card draws —
 * `undefined` until they are read, which is every sport. A sport that
 * dropped out of the window on a re-read falls back to all of them, for the
 * card and the list alike.
 */
export function useVolumeSportFilter(weeks: readonly VolumeWeekInput[] | undefined): VolumeSportFilter {
  const sports = useMemo(() => volumeSports(weeks ?? []), [weeks]);
  const [chosen, choose] = useState<string>(ALL_SPORTS);
  const selected = chosen !== ALL_SPORTS && sports.includes(chosen) ? chosen : ALL_SPORTS;
  return { sports, selected, sport: selected === ALL_SPORTS ? null : selected, choose };
}
