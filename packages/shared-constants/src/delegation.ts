// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The TrainingPeaks link vocabulary both group-info surfaces print — each refusal reason as a corpus key
// ABOUTME: A constants module cannot translate, so it names the keys and each client resolves them with its own t()

import type { DelegationRefusalReason } from '@pierre/shared-types';

/**
 * The corpus key each link refusal reads as. The server's message is English
 * and addressed to an API caller; the clients branch on `details.reason`
 * instead and say it in the athlete's language.
 */
export const DELEGATION_REFUSAL_KEY: Record<DelegationRefusalReason, string> = {
  trainingpeaks_not_connected: 'delegation.connectFirst',
  trainingpeaks_not_coach_account: 'delegation.notCoachAccount',
  trainingpeaks_email_missing: 'delegation.coachEmailMissing',
  trainingpeaks_email_mismatch: 'delegation.coachEmailMismatch',
  dravr_email_unverified: 'delegation.emailUnverified',
  trainingpeaks_reconnect_needed: 'delegation.reconnectFirst',
  trainingpeaks_terms_outdated: 'delegation.termsOutdated',
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
 * The roster refusals the coach resolves on their own TrainingPeaks
 * connection — connecting it, reconnecting it, or connecting the account
 * registered with their Dravr email — so the section offers a way to the
 * connections screen.
 */
export const DELEGATION_CONNECTION_REFUSALS: ReadonlySet<DelegationRefusalReason> = new Set([
  'trainingpeaks_not_connected',
  'trainingpeaks_email_missing',
  'trainingpeaks_email_mismatch',
  'trainingpeaks_reconnect_needed',
  'trainingpeaks_terms_outdated',
]);

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
