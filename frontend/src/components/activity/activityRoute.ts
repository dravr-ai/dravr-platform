// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The hash route of one activity's view, `home/activity/<provider>/<id>`, built and read in one place
// ABOUTME: A sub-view of Home, so Back returns to Home and a deep link opens the activity on the Home tab

/** The segment under Home that names an activity's view. */
const ACTIVITY_SEGMENT = 'activity';

/** The activity a view shows: the provider's slug and the provider's own id for it. */
export interface ActivityRef {
  provider: string;
  id: string;
}

/** The Dashboard route, `tab/subview`, of one activity's view. */
export function activityViewRoute(provider: string, id: string): string {
  return `home/${ACTIVITY_SEGMENT}/${encodeURIComponent(provider)}/${encodeURIComponent(id)}`;
}

/**
 * The activity a Home sub-view names, or null for any other sub-view — the
 * Home page itself. `activity/<provider>/<id>` with both segments present and
 * nothing after them; anything else is not an activity's view.
 */
export function parseActivitySubview(sub: string): ActivityRef | null {
  const parts = sub.split('/');
  if (parts.length !== 3 || parts[0] !== ACTIVITY_SEGMENT || parts[1] === '' || parts[2] === '') {
    return null;
  }
  try {
    return { provider: decodeURIComponent(parts[1]), id: decodeURIComponent(parts[2]) };
  } catch (error) {
    // A malformed percent escape in a hand-typed hash names no activity.
    if (error instanceof URIError) return null;
    throw error;
  }
}
