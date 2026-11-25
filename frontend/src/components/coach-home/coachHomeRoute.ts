// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The hash route of a coach's Home — `home`, `home/athlete/<id>`, `home/athlete/<id>/plan`, `home/team`, `home/season`, `home/me` — and its parse back
// ABOUTME: Home's conversation (`home/chat/<id>`) and activity view (`home/activity/…`) are the personal Home's and never read as these

/**
 * The views of a coach's Home: À traiter, one athlete in it, one athlete's
 * workspace — their plan with the coach's threads beside it (carnet#1816) —,
 * the team table, the season, and the coach's own Home.
 */
export type CoachHomeRoute =
  | { view: 'queue'; athleteId: string | null }
  | { view: 'workspace'; athleteId: string }
  | { view: 'team' }
  | { view: 'season' }
  | { view: 'me' };

const ATHLETE_SEGMENT = 'athlete';
const PLAN_SEGMENT = 'plan';

/** The athlete id a route segment carries, or `null` for one that does not decode. */
function decodeAthlete(segment: string): string | null {
  try {
    return decodeURIComponent(segment);
  } catch (error) {
    if (error instanceof URIError) return null;
    throw error;
  }
}

/**
 * The coach view a Home subview names, or `null` for a subview that belongs
 * to the personal Home (a conversation, an activity) or that nothing names.
 * An empty subview is À traiter.
 */
export function parseCoachHomeRoute(sub: string): CoachHomeRoute | null {
  if (sub === '') return { view: 'queue', athleteId: null };
  if (sub === 'team') return { view: 'team' };
  if (sub === 'season') return { view: 'season' };
  if (sub === 'me') return { view: 'me' };
  const parts = sub.split('/');
  if (parts[0] !== ATHLETE_SEGMENT || parts[1] === undefined || parts[1] === '') return null;
  const athleteId = decodeAthlete(parts[1]);
  if (athleteId === null) return null;
  if (parts.length === 2) return { view: 'queue', athleteId };
  if (parts.length === 3 && parts[2] === PLAN_SEGMENT) return { view: 'workspace', athleteId };
  return null;
}

/** The full hash route (`home[/…]`) of a coach view. */
export function coachHomeRoute(route: CoachHomeRoute): string {
  switch (route.view) {
    case 'queue':
      return route.athleteId === null ? 'home' : `home/${ATHLETE_SEGMENT}/${encodeURIComponent(route.athleteId)}`;
    case 'workspace':
      return `home/${ATHLETE_SEGMENT}/${encodeURIComponent(route.athleteId)}/${PLAN_SEGMENT}`;
    case 'team':
      return 'home/team';
    case 'season':
      return 'home/season';
    case 'me':
      return 'home/me';
  }
}
