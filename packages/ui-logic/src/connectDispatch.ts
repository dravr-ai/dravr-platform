// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The step a provider card's connect takes on every surface — Settings and onboarding, web and phone
// ABOUTME: One reading of the card's connectPlan, so no surface sends a card the athlete authorized to the key form while the authorization is offered

import { connectPlan, intervalsKeyReplacesAuthorization, type ConnectPlanCard } from '@pierre/shared-constants';
import type { SciotteTarget } from '@pierre/shared-types';

/**
 * How the Intervals.icu dialog opens:
 * - `offersAuthorize`: it leads with the authorization, the API-key form one
 *   tap away; the key form alone otherwise.
 * - `replacesAuthorization`: a key linked there replaces the athlete's
 *   authorization, whose grant the server revokes at Intervals.icu, so the
 *   key form says so before it is submitted.
 */
export interface IntervalsDialog {
  offersAuthorize: boolean;
  replacesAuthorization: boolean;
}

/**
 * The step a card's connect, or reconnect, takes:
 * - `sign_in`: the provider's own account sign-in, for `target`.
 * - `authorize`: the authorization of `provider`, the name the authorize
 *   route takes. An Intervals.icu card the athlete authorized reconnects this
 *   way, like any other authorization.
 * - `intervals_dialog`: the Intervals.icu dialog, opened as `dialog` says.
 * - `none`: a card that connects without the athlete.
 */
export type ConnectAction =
  | { kind: 'sign_in'; target: SciotteTarget }
  | { kind: 'authorize'; provider: string }
  | { kind: 'intervals_dialog'; dialog: IntervalsDialog }
  | { kind: 'none' };

/** The step `card`'s connect takes, as its {@link connectPlan} decides. */
export function connectAction(card: ConnectPlanCard): ConnectAction {
  const primary = connectPlan(card).primary;
  switch (primary.kind) {
    case 'sign_in':
      return { kind: 'sign_in', target: primary.target };
    case 'oauth':
      return { kind: 'authorize', provider: primary.provider };
    case 'intervals':
      return primary.path === 'authorize'
        ? { kind: 'authorize', provider: card.provider }
        : {
            kind: 'intervals_dialog',
            dialog: {
              offersAuthorize: primary.path === 'choose',
              replacesAuthorization: intervalsKeyReplacesAuthorization(card),
            },
          };
    case 'none':
      return { kind: 'none' };
  }
}

/**
 * What a surface does for each step of {@link ConnectAction}. A notice owed
 * first and an own-app setup stay with the surface, inside its `authorize`.
 */
export interface ConnectHandlers {
  /** Open the account sign-in for `target`. */
  signIn: (target: SciotteTarget) => void;
  /** Start the authorization of `provider`. */
  authorize: (provider: string) => void;
  /** Open the Intervals.icu dialog as `dialog` says. */
  intervalsDialog: (dialog: IntervalsDialog) => void;
  /** The card connects without the athlete; a surface with nothing to do omits it. */
  none?: () => void;
}

/** Take the step `card`'s connect calls for, through the surface's `handlers`. */
export function followConnectPlan(card: ConnectPlanCard, handlers: ConnectHandlers): void {
  const action = connectAction(card);
  switch (action.kind) {
    case 'sign_in':
      handlers.signIn(action.target);
      return;
    case 'authorize':
      handlers.authorize(action.provider);
      return;
    case 'intervals_dialog':
      handlers.intervalsDialog(action.dialog);
      return;
    case 'none':
      handlers.none?.();
      return;
  }
}
