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

/**
 * Which Home the athlete is shown (carnet#1528).
 *
 * - `first_arrival`: Getting started is open and the athlete has not yet
 *   answered the lead's welcome — the Meet your team row is still to do. Today
 *   leads with the whole Getting started list.
 * - `returning`: everyone else. Today leads with today's session, and Getting
 *   started, while open, is a slim meter under the conversation's header.
 */
export type HomeStage = 'first_arrival' | 'returning';

/** The Getting started row that is done once the athlete answers the lead's welcome. */
const MEET_TEAM_ROW = 'meet_team';

/** The Home `data` and `state` call for; `null` while either is unknown. */
export function homeStage(
  data: GettingStartedResponse | undefined,
  state: GettingStartedState | null,
): HomeStage | null {
  if (data === undefined || state === null) return null;
  if (state !== 'open') return 'returning';
  const meetTeam = data.rows.find((row) => row.id === MEET_TEAM_ROW);
  return meetTeam !== undefined && !meetTeam.done ? 'first_arrival' : 'returning';
}

/** The Getting started read, shared by the section, the meter and the Today line. */
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
  const state = gettingStartedState(query.data, hidden);
  return { query, home, state, stage: homeStage(query.data, state) };
}
