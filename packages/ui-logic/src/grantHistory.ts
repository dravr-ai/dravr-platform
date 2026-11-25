// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: How Connected apps reads a grant receipt and a standing policy — the outcome a receipt records, its app, who is always allowed
// ABOUTME: One reading for web and mobile, so both name each consent event the same way (carnet#1544)

import type { OAuthGrant, OAuthGrantReceipt, OAuthStandingPolicy } from '@pierre/shared-types';

/**
 * What a receipt records, as Connected apps names it:
 * - `allowed`: the athlete approved the grant on the consent screen (or the
 *   grant predates receipts and was recorded later, a baseline);
 * - `renewed`: a standing policy renewed it without the screen;
 * - `endedByYou`: the athlete revoked it, or approving the app again replaced it;
 * - `endedByApp`: the app revoked it (RFC 7009) or its owner deleted it;
 * - `endedByDravr`: Dravr ended it (the app's registration was swept, the account closed);
 * - `expired`: it reached its end date.
 */
export type GrantReceiptOutcome = 'allowed' | 'renewed' | 'endedByYou' | 'endedByApp' | 'endedByDravr' | 'expired';

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
  }
  return 'allowed';
}

/** The client a receipt's grant was given to, from its signed payload; null when the payload names none. */
export function grantReceiptClientId(receipt: OAuthGrantReceipt): string | null {
  const clientId = member(receipt.payload, 'grant')?.client_id;
  return typeof clientId === 'string' && clientId.length > 0 ? clientId : null;
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
 * The clients the athlete always allows. Revoking such a client's grant
 * leaves its policy, so the client is renewed the next time it connects:
 * the revoke confirmation says so.
 */
export function alwaysAllowedClients(policies: readonly OAuthStandingPolicy[]): Set<string> {
  return new Set(policies.map((policy) => policy.client_id));
}
