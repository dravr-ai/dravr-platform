// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: What a provider connect on the phone says and opens next — an export's route, a refused connect's line on its row
// ABOUTME: Pure helpers shared by Connections, onboarding's connect step and the sign-in window; the card logic is @pierre/ui-logic's

import { connectPlan } from '@pierre/shared-constants';
import type { SciotteTarget } from '@pierre/shared-types';
import type { ApiErrorTranslate } from '@pierre/ui-logic';
import type { ExtendedProviderStatus } from '../types';
import { IMPORT_ROUTE, ONBOARDING_IMPORT_ROUTE } from '../navigation/routes';

/** Where the import of `source` opens: inside onboarding's connect step, or over the app. */
export function importRouteFor(source: string, onboarding: boolean) {
  return { pathname: onboarding ? ONBOARDING_IMPORT_ROUTE : IMPORT_ROUTE, params: { source } } as const;
}

/** A row's connect that did not go through. */
export interface RefusedConnect {
  card: string;
}

/** The line a refused row shows, its tap starting the connect again: it did not connect. */
export function refusedConnectLine(t: ApiErrorTranslate, platform: string): string {
  return t('providers.connectRefused', { provider: platform });
}

/** What a screen hands {@link showConnectRefusal}: its read of the rows, its sign-in window, its refused row. */
export interface ConnectRefusalSurface {
  /** Read the rows again; resolves to the rows read, none when the read failed. */
  reloadCards: () => Promise<ExtendedProviderStatus[]>;
  /**
   * Open the sign-in to `target` for `card`, carrying the `consents` the
   * athlete ticked before the connect that was refused.
   */
  openSignIn: (target: SciotteTarget, card: ExtendedProviderStatus, consents?: readonly string[]) => void;
  /** Mark the row whose connect did not go through. */
  setRefusedCard: (refused: RefusedConnect) => void;
}

/**
 * Tell the row `cardKey` its connect did not go through. When the refusal
 * says the row's plan changed under the athlete (the seat it was shown
 * went), `reload` reads the rows again first, and a row that now signs in
 * opens its sign-in at once, with the `consents` the athlete ticked on the
 * connect's consent sheet (Garmin's) so the sign-in records the same answer.
 */
export async function showConnectRefusal(
  cardKey: string,
  { reload, consents }: { reload: boolean; consents?: readonly string[] },
  surface: ConnectRefusalSurface,
): Promise<void> {
  if (reload) {
    const served = await surface.reloadCards();
    const card = served.find((p) => p.provider === cardKey);
    const primary = card ? connectPlan(card).primary : null;
    if (card && primary?.kind === 'sign_in') {
      surface.openSignIn(primary.target, card, consents);
      return;
    }
  }
  surface.setRefusedCard({ card: cardKey });
}
