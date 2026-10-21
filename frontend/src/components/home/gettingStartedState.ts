// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Where Getting started stands — open, folded or gone — from the server's answer and the athlete's Hide
// ABOUTME: One rule and one read, shared by Today's section and the narrow-screen Today line so they never disagree (carnet#1098)

import { useQuery } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import type { GettingStartedResponse } from '@pierre/shared-types';
import { gettingStartedApi } from '../../services/api';
import { useHomePreferences } from '../../hooks/useHome';

/** How long the folded one-liner stays after the last flow is done. */
const FOLDED_FOR_MS = 7 * 24 * 60 * 60 * 1000;

/** Where Getting started stands: shown with its rows, folded to one line, or gone. */
export type GettingStartedState = 'open' | 'folded' | 'gone';

/**
 * The section's state from the server's answer and the athlete's Hide — the
 * one rule Today's section and the narrow-screen Today line both follow.
 * `null` while either is unknown, so nothing flashes in and out on load.
 */
export function gettingStartedState(
  data: GettingStartedResponse | undefined,
  hidden: boolean | null,
  now: number = Date.now(),
): GettingStartedState | null {
  if (data === undefined || hidden === null) return null;
  if (hidden || data.hidden) return 'gone';
  if (data.folded_at === null) return 'open';
  const folded = Date.parse(data.folded_at);
  return Number.isFinite(folded) && now - folded > FOLDED_FOR_MS ? 'gone' : 'folded';
}

/** The Getting started read, shared by the section and the Today line. */
export function useGettingStarted() {
  const query = useQuery({
    queryKey: QUERY_KEYS.home.gettingStarted(),
    queryFn: () => gettingStartedApi.get(),
  });
  const home = useHomePreferences();
  // A choice that could not be read shows the section: it can still be hidden.
  const hidden = home.preferences
    ? home.preferences.getting_started_hidden === true
    : home.isError
      ? false
      : null;
  return { query, home, state: gettingStartedState(query.data, hidden) };
}
