// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A provider connect runs in the athlete's own tab and comes back to the page it started from
// ABOUTME: Keeps the starting route across the provider round trip and settles the callback's result on return

import {
  OAUTH_NO_SEAT,
  cardForSlug,
  connectPlan,
  credentialFallbackOnRefusal,
  refusalReturnsToCard,
  type ConnectPlanCard,
} from '@pierre/shared-constants';
import type { SciotteTarget } from '@pierre/shared-types';
import { cardAwaitingHistory, type ImportTombstoneRefusal } from '@pierre/ui-logic';
import { writeSessionRoute } from './sessionRoute';

/** The `sessionStorage` key holding the dashboard route a connect started from. */
const RETURN_ROUTE_KEY = 'pierre_oauth_return';

/**
 * The `localStorage` key the settled result is left under, for whichever
 * surface the athlete returns to: the open thread (`useOAuthCompletion`), the
 * provider cards and Settings each read it when they mount.
 */
export const OAUTH_RESULT_KEY = 'pierre_oauth_result';

/**
 * How long a result left under [`OAUTH_RESULT_KEY`] still describes the
 * connect the athlete just made. A refusal that keeps the callback page waits
 * there for the athlete to read it, so this outlasts a read, not a redirect.
 */
export const OAUTH_RESULT_FRESH_MS = 5 * 60 * 1000;

/** What the server's return to the app root says about a finished connect. */
export interface OAuthCallbackParams {
  provider: string;
  success: boolean;
  error?: string;
  tombstone?: ImportTombstoneRefusal;
}

/** The result left under [`OAUTH_RESULT_KEY`]. */
export interface OAuthResult {
  type: 'oauth_completed';
  provider: string;
  success: boolean;
  /** Raw error messages may carry sensitive detail, so only that one occurred is kept. */
  hasError: boolean;
  /**
   * Why the grant was refused, kept only for a reason on the allow-list: the
   * provider's OAuth seats ran out (`no_seat`). Any other error is dropped.
   */
  reason?: typeof OAUTH_NO_SEAT;
  /** A refusal resolved on the callback page: no other way in may be offered. */
  tombstoned: boolean;
  timestamp: number;
}

/**
 * Start a provider connect in this tab: remember the dashboard route the
 * athlete is on, then go to the server's launch route, which redirects to the
 * provider. The server returns to the app root, where [`settleOAuthReturn`]
 * puts the route back.
 *
 * It never opens a second tab: a return landing there would be a second copy
 * of the app, while the one the athlete started from waited behind it.
 */
export function startOAuthInThisTab(url: string): void {
  try {
    sessionStorage.setItem(RETURN_ROUTE_KEY, window.location.hash.replace(/^#/, ''));
  } catch {
    // Storage refused (private mode): the return lands on the role default.
  }
  window.location.assign(url);
}

/**
 * Start a provider connect in a new tab, leaving this one where it is: the
 * chat's reconnect, offered inside a reply the athlete is still reading. The
 * tab is opened by script so it may close itself when the connect comes back
 * ([`returnToConnectStart`]), and its `opener` is cut so the provider's pages
 * cannot reach back into this tab. This tab hears the result through the
 * storage event the return's write fires.
 */
export function startOAuthInNewTab(url: string): void {
  const tab = window.open(url, '_blank');
  if (tab) tab.opener = null;
}

/**
 * Leave a connect's return for where the connect started. A connect started
 * in this tab ([`startOAuthInThisTab`]) gets its route back, as this session's
 * own route, and forgets it. One started elsewhere — a new tab the chat opened
 * ([`startOAuthInNewTab`]) — closes this tab, so the athlete is back on the
 * tab they started from; a tab the browser will not let a script close (a
 * link followed from outside the app) stays, on the app root, where the
 * dashboard opens the role default.
 */
export function returnToConnectStart(): void {
  let route: string | null = null;
  try {
    route = sessionStorage.getItem(RETURN_ROUTE_KEY);
    sessionStorage.removeItem(RETURN_ROUTE_KEY);
  } catch {
    // Storage refused: nothing was kept.
  }
  if (route === null) window.close();
  window.history.replaceState(null, '', '/');
  if (route) writeSessionRoute(route, 'replace');
}

/**
 * The card whose "Bring in your history" sheet a completed grant opens, or
 * `null`: the granted provider's card — named by its key, or by the backend
 * that serves it (the Garmin grant serves the `sciotte_garmin` card) — read
 * after the grant, whose history is still `needed` and which offers a way to
 * bring it in this build can show (`cardAwaitingHistory`). Garmin's API
 * pushes only what the athlete records after connecting, so the athlete
 * chooses how to bring in what came before.
 */
export function historyChoiceAfterGrant<T extends ConnectPlanCard>(
  result: Pick<OAuthResult, 'provider' | 'success'>,
  providers: readonly T[],
): T | null {
  if (!result.success) return null;
  return cardAwaitingHistory(providers, result.provider) ?? null;
}

/** A refused grant's answer on its card: its account sign-in, or a retry on the row ({@link refusalAnswer}). */
export type RefusalAnswer<T> = { kind: 'sign_in'; target: SciotteTarget; card: T | undefined } | { kind: 'retry'; card: T };

/**
 * How a card answers a refused grant that came back to it
 * (`refusalReturnsToCard`), read from the cards as they are after it:
 * - `sign_in`: Strava's account sign-in, its fallback for any refusal; and
 *   Garmin's when the grant was refused because the seats ran out under the
 *   athlete (`reason` `no_seat`) and its plan now signs in — while they are
 *   out a Garmin connection signs in.
 * - `retry`: the card shows "didn't connect, try again" — any other refused
 *   Garmin grant, and one whose plan still authorizes; a refused Garmin grant
 *   never opens the sign-in while a seat is free.
 * - `null`: no card answers it (a tombstone, which the callback page resolved,
 *   or a provider no card is for).
 */
export function refusalAnswer<T extends ConnectPlanCard>(
  result: Pick<OAuthResult, 'provider' | 'success' | 'tombstoned' | 'reason'>,
  providers: readonly T[],
): RefusalAnswer<T> | null {
  if (result.success || result.tombstoned || !refusalReturnsToCard(result.provider)) return null;
  const card = cardForSlug(result.provider, providers);
  const fallback = credentialFallbackOnRefusal(result.provider);
  if (fallback) return { kind: 'sign_in', target: fallback, card };
  if (!card) return null;
  const primary = connectPlan(card).primary;
  if (result.reason === OAUTH_NO_SEAT && primary.kind === 'sign_in') {
    return { kind: 'sign_in', target: primary.target, card };
  }
  return { kind: 'retry', card };
}

/**
 * Whether the athlete goes straight back to the page the connect started
 * from. A grant does; so does a refused Strava or Garmin grant started in
 * this tab (`refusalReturnsToCard`), whose card answers it — Strava's with its
 * sign-in, Garmin's with what its plan now says. Any other refusal stays on
 * the callback page, which gives its reason — as does every refusal in a tab
 * the chat opened, where no card is there to answer it — and a tombstone,
 * which that page resolves.
 */
function returnsAtOnce(params: OAuthCallbackParams): boolean {
  if (params.tombstone !== undefined) return false;
  return params.success || (startedInThisTab() && refusalReturnsToCard(params.provider));
}

/** Whether the connect this tab came back from started here ([`startOAuthInThisTab`]). */
function startedInThisTab(): boolean {
  try {
    return sessionStorage.getItem(RETURN_ROUTE_KEY) !== null;
  } catch {
    return false;
  }
}

/**
 * Settle the server's return from a connect, before the first render: leave
 * the result for the surface the athlete returns to and, when nothing is left
 * to say on the callback page, go back to the route the connect started from.
 *
 * Returns the callback page to show instead, or `null`. Idempotent: once the
 * address bar is put back there is nothing left to settle, so React's
 * development double call of a state initializer finds nothing the second time.
 */
export function settleOAuthReturn(params: OAuthCallbackParams | null): OAuthCallbackParams | null {
  if (!params) return null;
  const result: OAuthResult = {
    type: 'oauth_completed',
    provider: params.provider,
    success: params.success,
    hasError: !params.success && !!params.error,
    reason: params.error === OAUTH_NO_SEAT ? OAUTH_NO_SEAT : undefined,
    tombstoned: params.tombstone !== undefined,
    timestamp: Date.now(),
  };
  try {
    localStorage.setItem(OAUTH_RESULT_KEY, JSON.stringify(result));
  } catch {
    // Storage refused: the surfaces refetch the provider status on mount anyway.
  }
  if (!returnsAtOnce(params)) return params;
  returnToConnectStart();
  return null;
}
