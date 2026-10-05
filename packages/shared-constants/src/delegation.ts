// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The coaching-platform link vocabulary both group-info surfaces print — each refusal reason as a corpus key
// ABOUTME: A constants module cannot translate, so it names the keys and the platform each client passes to its own t()

import type { CoachPlatformProvider, DelegationRefusalReason } from '@pierre/shared-types';

/**
 * The corpus key each link refusal reads as. The server's message is English
 * and addressed to an API caller; the clients branch on `details.reason`
 * instead and say it in the athlete's language.
 */
export const DELEGATION_REFUSAL_KEY: Record<DelegationRefusalReason, string> = {
  coach_platform_not_connected: 'delegation.connectFirst',
  coach_platform_not_coach_account: 'delegation.notCoachAccount',
  coach_platform_email_missing: 'delegation.coachEmailMissing',
  coach_platform_email_mismatch: 'delegation.coachEmailMismatch',
  dravr_email_unverified: 'delegation.emailUnverified',
  coach_platform_reconnect_needed: 'delegation.reconnectFirst',
  coach_platform_api_key_required: 'delegation.apiKeyRequired',
  coach_platform_terms_outdated: 'delegation.termsOutdated',
  unsupported_provider: 'delegation.actionFailed',
  invalid_athlete: 'delegation.actionFailed',
  athlete_not_on_roster: 'delegation.notOnRoster',
  athlete_email_missing: 'delegation.athleteEmailMissing',
  athlete_email_mismatch: 'delegation.athleteEmailMismatch',
  member_email_unverified: 'delegation.memberEmailUnverified',
  member_is_coach: 'delegation.actionFailed',
  already_proposed: 'delegation.alreadyProposed',
  athlete_already_linked: 'delegation.athleteAlreadyLinked',
  own_connection: 'delegation.ownConnectionConflict',
  already_linked: 'delegation.alreadyLinked',
};

/** Said when a step failed for a reason the table does not name. */
export const DELEGATION_ACTION_FAILED_KEY = 'delegation.actionFailed';

/**
 * The roster refusals the coach resolves on their own coaching-platform
 * connection — connecting it, reconnecting it (with an API key, for
 * Intervals.icu), or connecting the account registered with their Dravr
 * email — so the section offers a way to the connections screen.
 */
export const DELEGATION_CONNECTION_REFUSALS: ReadonlySet<DelegationRefusalReason> = new Set([
  'coach_platform_not_connected',
  'coach_platform_email_missing',
  'coach_platform_email_mismatch',
  'coach_platform_reconnect_needed',
  'coach_platform_api_key_required',
  'coach_platform_terms_outdated',
]);

/**
 * Each coaching platform's brand, the `{{platform}}` every `delegation.*`
 * string names. A brand is a name, not a word, so it reads the same in every
 * language.
 */
const COACH_PLATFORM_NAMES: Record<CoachPlatformProvider, string> = {
  trainingpeaks: 'TrainingPeaks',
  intervals_icu: 'Intervals.icu',
};

/** The backend a platform's own connection is stored under, where it differs from the platform. */
const COACH_PLATFORM_BACKENDS: Record<string, CoachPlatformProvider> = {
  sciotte_trainingpeaks: 'trainingpeaks',
};

/**
 * What `{{platform}}` reads as for `provider` — a roster's or a link's
 * `provider`, a refusal's `details.provider`, or a provider card's backend.
 * Before any platform is known (the roster is loading, or the coach has
 * connected none), it names every platform a coach can connect.
 */
export function coachPlatformName(provider: unknown): string {
  // Own keys only: an index would also find `toString` and the other names a
  // table inherits.
  const own = (table: object, key: string) => Object.prototype.hasOwnProperty.call(table, key);
  if (typeof provider === 'string') {
    if (own(COACH_PLATFORM_NAMES, provider)) {
      return COACH_PLATFORM_NAMES[provider as CoachPlatformProvider];
    }
    if (own(COACH_PLATFORM_BACKENDS, provider)) {
      return COACH_PLATFORM_NAMES[COACH_PLATFORM_BACKENDS[provider]];
    }
  }
  return Object.values(COACH_PLATFORM_NAMES).join(' / ');
}

/** Whether `reason` is one the server sends for a link step. */
export function isDelegationRefusal(reason: unknown): reason is DelegationRefusalReason {
  // Own keys only: `in` would also accept `toString` and the other names the
  // table inherits.
  return typeof reason === 'string' && Object.prototype.hasOwnProperty.call(DELEGATION_REFUSAL_KEY, reason);
}

/** The corpus key a refusal reads as, or the generic failure for anything else. */
export function delegationRefusalKey(reason: unknown): string {
  return isDelegationRefusal(reason) ? DELEGATION_REFUSAL_KEY[reason] : DELEGATION_ACTION_FAILED_KEY;
}
