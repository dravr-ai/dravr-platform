// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The provider capability-scope vocabulary — one word per wire slug, in the athlete's language
// ABOUTME: Also maps each scrape-mirror card to its login target, and each provider to the notice it asks for first

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
  sciotte_coros: 'coros',
};

/**
 * The credential-login target for a provider card, or `null` for a provider
 * that connects some other way (OAuth, an API key).
 */
export function sciotteTargetForBackend(provider: string): SciotteTarget | null {
  return SCIOTTE_TARGET_BY_BACKEND[provider] ?? null;
}

/** The catalogue keys of the notice a provider asks for before connecting. */
export interface ProviderNoticeKeys {
  titleKey: string;
  bodyKey: string;
  consentKey: string;
}

/**
 * Every provider notice, by the backend id the server's `consent_required`
 * names it by (`PROVIDER_TERMS_VERSIONS` in
 * `crates/pierre-core/src/constants/oauth/providers.rs`). TrainingPeaks and
 * COROS state the account risk of a signed-in scrape; WHOOP asks for the owner
 * authorization its API terms require before Dravr keeps any WHOOP data.
 */
export const PROVIDER_NOTICES: Record<string, ProviderNoticeKeys> = {
  sciotte_trainingpeaks: {
    titleKey: 'providers.trainingpeaksNotice.title',
    bodyKey: 'providers.trainingpeaksNotice.body',
    consentKey: 'providers.trainingpeaksNotice.consent',
  },
  sciotte_coros: {
    titleKey: 'providers.corosNotice.title',
    bodyKey: 'providers.corosNotice.body',
    consentKey: 'providers.corosNotice.consent',
  },
  whoop: {
    titleKey: 'providers.whoopNotice.title',
    bodyKey: 'providers.whoopNotice.body',
    consentKey: 'providers.whoopNotice.consent',
  },
};

/**
 * Whether connecting `provider` must first show its notice: the card says the
 * account has not accepted it, and the client carries a notice to show.
 */
export function noticeRequired(provider: string, consentRequired: boolean | undefined): boolean {
  return Boolean(consentRequired) && provider in PROVIDER_NOTICES;
}

/**
 * The providers whose notice also gates what health sync keeps: while it is
 * owed, sync stores none of the account's records from that provider (WHOOP's
 * owner authorization). A connected card for one of them that owes its notice
 * is not healthy, whatever `connected` says.
 */
const SYNC_GATING_NOTICES: ReadonlySet<string> = new Set(['whoop']);

/**
 * Whether a connected provider has stopped syncing until the athlete accepts
 * its notice: the card is connected, says the notice is owed, and the notice
 * gates what sync keeps. The card then asks for the authorization, and
 * accepting it restarts the provider's connect with the acceptance.
 */
export function syncAuthorizationOwed(
  provider: string,
  connected: boolean,
  consentRequired: boolean | undefined,
): boolean {
  return connected && Boolean(consentRequired) && SYNC_GATING_NOTICES.has(provider);
}
