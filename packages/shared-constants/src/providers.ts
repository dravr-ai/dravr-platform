// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The provider capability-scope vocabulary — one word per wire slug, in the athlete's language
// ABOUTME: Also maps each scrape-mirror provider card to the credential-login target it opens

import type { SciotteTarget } from '@pierre/shared-types';

/**
 * Every capability slug `GET /api/oauth/providers` can put on a provider card,
 * in the order the route builds them (`crates/pierre-routes-auth/src/oauth.rs`).
 * The strings are the wire values, so they stay English whatever the athlete
 * reads.
 */
export const PROVIDER_SCOPES = [
  'activities',
  'sleep',
  'recovery',
  'health',
  'planned_workouts',
] as const;

export type ProviderScope = (typeof PROVIDER_SCOPES)[number];

/**
 * The corpus key naming each scope. Module scope cannot hold a hook, so the
 * table carries the key and each client resolves it with its own `t` — the
 * card's capability line then reads as words in the athlete's language
 * instead of as the wire slugs under otherwise translated chrome.
 */
export const PROVIDER_SCOPE_LABEL_KEY: Record<ProviderScope, string> = {
  activities: 'providers.scope.activities',
  sleep: 'providers.scope.sleep',
  recovery: 'providers.scope.recovery',
  health: 'providers.scope.health',
  planned_workouts: 'providers.scope.planned_workouts',
};

/**
 * The label key for `scope`, or `null` for a slug the catalogue has no word
 * for. A caller prints such a slug as itself: a provider that starts
 * advertising a new capability shows its wire name, never a missing-key
 * string.
 */
export function providerScopeLabelKey(scope: string): string | null {
  return PROVIDER_SCOPE_LABEL_KEY[scope as ProviderScope] ?? null;
}

/**
 * Each scrape-mirror backend's login target — the server's
 * `backend_resolver::hosted_login_target`, which the hosted pages read, so
 * the web and mobile cards open the same login the channel link does.
 */
const SCIOTTE_TARGET_BY_BACKEND: Record<string, SciotteTarget> = {
  sciotte: 'strava',
  sciotte_garmin: 'garmin',
  sciotte_trainingpeaks: 'trainingpeaks',
};

/**
 * The credential-login target for a provider card, or `null` for a provider
 * that connects some other way (OAuth, an API key).
 */
export function sciotteTargetForBackend(provider: string): SciotteTarget | null {
  return SCIOTTE_TARGET_BY_BACKEND[provider] ?? null;
}
