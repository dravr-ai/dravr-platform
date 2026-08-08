// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Builds the fitness-provider cards the UI offers from the raw /api/providers list.
// ABOUTME: Hides the native strava/garmin rows and folds a native Strava OAuth grant onto the sciotte card.

import type { ExtendedProviderStatus } from '@pierre/shared-types';

/**
 * A rendered provider card plus the provider id whose stored tokens back it.
 *
 * The two differ for Strava. The card the user sees is `sciotte`, but the
 * connection behind it may be a native `strava` OAuth grant, and
 * `DELETE /api/oauth/providers/{provider}/disconnect` deletes the tokens for
 * exactly the id it is handed. A disconnect aimed at `sciotte` therefore
 * leaves a `strava` grant in place, so the card carries the id to act on.
 */
export interface FitnessProviderCard extends ExtendedProviderStatus {
  /** Provider id to pass to connect and disconnect calls for this card. */
  connectionProvider: string;
}

/**
 * Reduce the raw `/api/providers` list to the cards the UI offers.
 *
 * Two rows are hidden. Native `strava` is reachable only through the Sciotte
 * modal's "Use my own Strava OAuth app" button and would otherwise render a
 * second, duplicate Strava card. `garmin` ("Garmin Connect") has an
 * uncredentialed OAuth API and must not be offered; the `sciotte_garmin`
 * scrape card stays.
 *
 * A connected native `strava` row is folded onto the `sciotte` card so the one
 * Strava card the user sees reports the connection that exists, carries its
 * `needs_reauth` state, and disconnects the grant that is actually stored.
 */
export function buildFitnessProviderCards(
  providers: ExtendedProviderStatus[] | undefined
): FitnessProviderCard[] {
  const stravaOAuth = providers?.find((p) => p.provider === 'strava' && p.connected);

  return (providers ?? [])
    .filter((p) => p.provider !== 'strava' && p.provider !== 'garmin')
    .map((p) =>
      p.provider === 'sciotte' && stravaOAuth && !p.connected
        ? {
            ...p,
            connected: true,
            needs_reauth: stravaOAuth.needs_reauth,
            connectionProvider: 'strava',
          }
        : { ...p, connectionProvider: p.provider }
    );
}
