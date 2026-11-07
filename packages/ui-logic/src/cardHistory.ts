// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: What a provider card offers for its past workouts in both clients: the choices shown, its export source, a grant's sheet
// ABOUTME: An export is offered only for a source the server put on the card and this build carries the copy of

import { cardHistoryState, connectPlan, type ConnectPlanCard, type HistoryChoice } from '@pierre/shared-constants';
import type { ExtendedProviderStatus } from '@pierre/shared-types';
import { importSourceCopy } from './archiveImport';

/**
 * The history choices `card` offers that this build can show: the sign-in
 * whenever the plan offers it, an export only while this build carries that
 * source's copy. An empty list means the card offers no way to bring its
 * history in here: no "Add history" action and no sheet.
 */
export function shownHistoryChoices(card: ConnectPlanCard): HistoryChoice[] {
  return connectPlan(card).history.choices.filter(
    (choice) => choice.kind === 'sign_in' || importSourceCopy(choice.source) !== undefined,
  );
}

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
 * when its history is still `needed` and it offers a way to bring it in that
 * this build can show: the history sheet opens on it. A grant lands on the
 * card of its provider's sign-in backend (the Garmin grant serves the
 * `sciotte_garmin` card), so the card is matched on what it is connected
 * through as well as on its key.
 */
export function cardAwaitingHistory<T extends ConnectPlanCard>(
  cards: readonly T[],
  grantProvider: string,
): T | undefined {
  return cards.find(
    (card) =>
      (card.provider === grantProvider || card.connected_backend === grantProvider) &&
      cardHistoryState(card) === 'needed' &&
      shownHistoryChoices(card).length > 0,
  );
}
