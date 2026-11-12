// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: What a provider card offers for its past workouts in both clients: its export source, and the sign-in a grant chains into
// ABOUTME: An export is offered only for a source the server put on the card and this build carries the copy of

import { connectPlan, type ConnectPlanCard } from '@pierre/shared-constants';
import type { ExtendedProviderStatus, SciotteTarget } from '@pierre/shared-types';
import { importSourceCopy } from './archiveImport';

/**
 * The card's export source, when the athlete can import it here: the server
 * names it only while it offers that source (`archive_source`), connected or
 * not, and a source this build has no words for is left out.
 */
export function cardExportSource(card: Pick<ExtendedProviderStatus, 'history'>): string | null {
  const source = card.history?.archive_source;
  return source && importSourceCopy(source) ? source : null;
}

/**
 * The card a grant of `grantProvider` (`garmin`, `strava`…) just connected,
 * when its history is still `needed` and an account sign-in copies it: that
 * sign-in opens at once, with nothing to choose (carnet#1472). A grant lands
 * on the card of its provider's sign-in backend (the Garmin grant serves the
 * `sciotte_garmin` card), so the card is matched on what it is connected
 * through as well as on its key.
 */
export function cardAwaitingHistory<T extends ConnectPlanCard>(
  cards: readonly T[],
  grantProvider: string,
): { card: T; target: SciotteTarget } | undefined {
  for (const card of cards) {
    if (card.provider !== grantProvider && card.connected_backend !== grantProvider) continue;
    const { state, signIn } = connectPlan(card).history;
    if (state === 'needed' && signIn) return { card, target: signIn };
  }
  return undefined;
}
