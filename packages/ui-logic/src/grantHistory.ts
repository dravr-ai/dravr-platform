// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: How Connected apps reads a grant receipt and a standing policy — the outcome a receipt records, its app, who is always allowed
// ABOUTME: One reading for web and mobile, so both name each consent event the same way (carnet#1544)

import type { OAuthGrant, OAuthGrantReceipt, OAuthGrantStepUp, OAuthStandingPolicy } from '@pierre/shared-types';

/**
 * What a receipt records, as Connected apps names it:
 * - `allowed`: the athlete approved the grant on the consent screen, or a
 *   one-off question after a fresh sign-in (`step_up`), or the grant predates
 *   receipts and was recorded later (a baseline);
 * - `renewed`: a standing policy renewed it without the screen;
 * - `endedByYou`: the athlete revoked it, or approving the app again replaced it;
 * - `endedByApp`: the app revoked it (RFC 7009), its owner deleted it, or the
 *   app's one read used up a single-use grant (`consume`);
 * - `endedByDravr`: Dravr ended it (the app's registration was swept, the account closed);
 * - `expired`: it reached its end date.
 */
export type GrantReceiptOutcome = 'allowed' | 'renewed' | 'endedByYou' | 'endedByApp' | 'endedByDravr' | 'expired';

/** The corpus key each outcome reads as; each client resolves it with its own t(). */
export const GRANT_RECEIPT_OUTCOME_KEYS: Readonly<Record<GrantReceiptOutcome, string>> = {
  allowed: 'tokens.grantEventAllowed',
  renewed: 'tokens.grantEventRenewed',
  endedByYou: 'tokens.grantEventEndedByYou',
  endedByApp: 'tokens.grantEventEndedByApp',
  endedByDravr: 'tokens.grantEventEndedByDravr',
  expired: 'tokens.grantEventExpired',
};

/** The corpus key naming each preset a grant or a standing policy can carry. */
export const CONNECTED_APP_PRESET_KEYS: Readonly<Record<string, string>> = {
  training_coach: 'tokens.connectedAppsPresetTrainingCoach',
  sleep_coach: 'tokens.connectedAppsPresetSleepCoach',
};

/**
 * The name of `preset` in the reader's language, or `null` for a plain grant
 * and for a preset this build does not know yet: the caller then shows the
 * grant's scope or the preset's own slug. Own keys only, so `toString` is no preset.
 */
export function connectedAppPresetLabel(preset: string | null, t: (key: string) => string): string | null {
  return preset !== null && Object.prototype.hasOwnProperty.call(CONNECTED_APP_PRESET_KEYS, preset)
    ? t(CONNECTED_APP_PRESET_KEYS[preset])
    : null;
}

/** The corpus key naming each stream a one-off question can read, as the step-up screen names it. */
export const STEP_UP_STREAM_KEYS: Readonly<Record<string, string>> = {
  'activity.daily.summary': 'oauth.stepUp.stream.activityDaily',
  'workout.summary': 'oauth.stepUp.stream.workout',
  'training.load.summary': 'oauth.stepUp.stream.trainingLoad',
  'sleep.summary': 'oauth.stepUp.stream.sleep',
  'cardio.daily.summary': 'oauth.stepUp.stream.cardioDaily',
  'body.summary': 'oauth.stepUp.stream.body',
  'fitness.summary': 'oauth.stepUp.stream.fitness',
  'recovery.daily.summary': 'oauth.stepUp.stream.recoveryDaily',
  'profile.summary': 'oauth.stepUp.stream.profile',
  'labs.summary': 'oauth.stepUp.stream.labs',
};

/** The corpus key saying how an app declared it handles health records (its Host Profile basis). */
export const STEP_UP_BASIS_KEYS: Readonly<Record<string, string>> = {
  declared_cimd: 'tokens.connectedAppsStepUpDeclaredCimd',
  declared_request: 'tokens.connectedAppsStepUpDeclaredRequest',
};

/**
 * What a connected app's one-off question reads and on what the app's word stands, one line each:
 * the streams the athlete left ticked (a stream this build has no label for is named as the server
 * names it), then the basis of its Host Profile. A line the server sent nothing for is left out.
 * Own keys only, so `toString` is no stream or basis.
 */
export function stepUpLines(
  stepUp: OAuthGrantStepUp,
  t: (key: string, options?: Record<string, unknown>) => string,
): string[] {
  const own = (keys: Readonly<Record<string, string>>, name: string) =>
    Object.prototype.hasOwnProperty.call(keys, name) ? keys[name] : null;
  const lines: string[] = [];
  const streams = (stepUp.streams ?? []).map((stream) => {
    const key = own(STEP_UP_STREAM_KEYS, stream);
    return key ? t(key) : stream;
  });
  if (streams.length > 0) {
    lines.push(t('tokens.connectedAppsStepUpStreams', { streams: streams.join(', ') }));
  }
  const basis = stepUp.host_profile_basis ? own(STEP_UP_BASIS_KEYS, stepUp.host_profile_basis) : null;
  if (basis) {
    lines.push(t(basis));
  }
  return lines;
}

/** The signed payload's member `name`, as an object, or null. */
function member(payload: Record<string, unknown>, name: string): Record<string, unknown> | null {
  const value = payload[name];
  return value !== null && typeof value === 'object' && !Array.isArray(value) ? (value as Record<string, unknown>) : null;
}

/** What `receipt` records. */
export function grantReceiptOutcome(receipt: OAuthGrantReceipt): GrantReceiptOutcome {
  switch (receipt.event) {
    case 'expire':
      return 'expired';
    case 'revoke':
      switch (receipt.actor) {
        case 'user':
          return 'endedByYou';
        case 'client':
          return 'endedByApp';
        case 'auto':
          return 'endedByDravr';
      }
      break;
    case 'issue': {
      const approval = member(receipt.payload, 'approval');
      return receipt.actor === 'auto' && approval?.standing_policy_ref ? 'renewed' : 'allowed';
    }
    case 'step_up':
      // The athlete approved a one-off question after a fresh sign-in.
      return 'allowed';
    case 'consume':
      // The app's one read ended the single-use grant.
      return 'endedByApp';
  }
  return 'allowed';
}

/** The client a receipt's grant was given to, from its signed payload; null when the payload names none. */
export function grantReceiptClientId(receipt: OAuthGrantReceipt): string | null {
  const clientId = member(receipt.payload, 'grant')?.client_id;
  return typeof clientId === 'string' && clientId.length > 0 ? clientId : null;
}

/**
 * The app a receipt names in the consent history: the name its client
 * registered (which the server reads when it lists the receipt, so a revoked
 * app keeps its name), else the name a live grant or policy carries, else the
 * client id; `null` when the receipt names no client.
 */
export function grantReceiptAppName(receipt: OAuthGrantReceipt, names: ReadonlyMap<string, string>): string | null {
  const clientId = grantReceiptClientId(receipt);
  return receipt.client_name || (clientId && names.get(clientId)) || clientId;
}

/**
 * The name to show for each client the athlete's grants and policies name:
 * the name it registered, else none (the caller shows the id). A receipt of a
 * client deleted since names only its id.
 */
export function clientNames(
  grants: readonly OAuthGrant[],
  policies: readonly OAuthStandingPolicy[],
): Map<string, string> {
  const names = new Map<string, string>();
  for (const { client_id, client_name } of [...grants, ...policies]) {
    if (client_name && !names.has(client_id)) {
      names.set(client_id, client_name);
    }
  }
  return names;
}

/**
 * The clients the athlete always allows, whose Connected-apps rows say so.
 * Revoking such a client's grant also ends its policies.
 */
export function alwaysAllowedClients(policies: readonly OAuthStandingPolicy[]): Set<string> {
  return new Set(policies.map((policy) => policy.client_id));
}
