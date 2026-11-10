// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: What a provider card offers — how it connects or reconnects, the sign-in its history needs, the move to the official connection
// ABOUTME: The one decision both clients render from, so web and mobile cannot disagree on a tap, a pill or a refused grant

import type { ExtendedProviderStatus, HistoryState, SciotteTarget } from '@pierre/shared-types';
import {
  SCIOTTE_LOGIN_CHALLENGE,
  SCIOTTE_LOGIN_PRESETS,
  cardHistoryState,
  intervalsConnectPath,
  sciotteTargetForBackend,
  syncAuthorizationOwed,
  type IntervalsConnectPath,
} from './providers';

/** The fields of a provider card the connect plan reads. */
export type ConnectPlanCard = Pick<
  ExtendedProviderStatus,
  | 'provider'
  | 'connected'
  | 'connected_backend'
  | 'connection_type'
  | 'needs_reauth'
  | 'requires_oauth'
  | 'own_app_required'
  | 'consent_required'
  | 'recommended_backend'
  | 'seats_left'
  | 'migration'
  | 'history'
>;

/**
 * How a card connects, or reconnects:
 * - `oauth`: the provider's own authorization, named by the provider the
 *   authorize route takes (`garmin`, `strava`, `whoop`…), never a card's
 *   `sciotte_*` key.
 * - `sign_in`: the provider's own account sign-in, for `target`.
 * - `intervals`: the Intervals.icu dialog, along `path`.
 * - `none`: a card that connects without the athlete (the synthetic provider).
 *
 * A notice owed first, an own-app setup and a coach's delegation stay with the
 * caller, which checks them before following the plan.
 */
type ConnectPrimary =
  | { kind: 'oauth'; provider: string }
  | { kind: 'sign_in'; target: SciotteTarget }
  | { kind: 'intervals'; path: IntervalsConnectPath }
  | { kind: 'none' };

/** What a provider card offers. */
interface ConnectPlan {
  primary: ConnectPrimary;
  /**
   * The card is served by a sign-in session alone and the server asks the
   * athlete to move to the provider's official connection
   * (`migration: 'oauth_required'`). The server sends it only while an OAuth
   * seat is free; `primary` is then that authorization.
   */
  migrate: boolean;
  /**
   * The card's history: its state, and the account sign-in that copies it
   * while it is still `needed` or did not finish (`null` otherwise). A
   * Garmin grant receives only what the athlete records after connecting,
   * so that sign-in follows the grant at once (carnet#1472).
   */
  history: { state: HistoryState; signIn: SciotteTarget | null };
}

/** Whether the server asks this card to move to its provider's official connection. */
function migrationAsked(card: Pick<ConnectPlanCard, 'migration'>): boolean {
  return card.migration === 'oauth_required';
}

/**
 * How a card whose provider has both an authorization and an account sign-in
 * (the Strava `sciotte` card and the Garmin `sciotte_garmin` card) connects.
 *
 * Garmin goes through its authorization when it is already held, when the
 * server asks for the move to it, or when the server recommends it (a seat is
 * free); with no seat the server recommends `mirror` and the card signs in.
 * Strava reconnects the way it is connected, and a new connection follows the
 * server's recommendation.
 */
function dualBackendPrimary(card: ConnectPlanCard, target: SciotteTarget): ConnectPrimary {
  const oauth: ConnectPrimary = { kind: 'oauth', provider: target };
  const signIn: ConnectPrimary = { kind: 'sign_in', target };
  if (target === 'garmin') {
    const viaGrant = card.connected && card.connected_backend === target;
    return viaGrant || migrationAsked(card) || card.recommended_backend === 'oauth' ? oauth : signIn;
  }
  if (card.connected && card.needs_reauth) {
    return card.connected_backend === target ? oauth : signIn;
  }
  return card.recommended_backend === 'oauth' ? oauth : signIn;
}

/** The targets whose card has an authorization beside its sign-in. */
const DUAL_BACKEND_TARGETS: ReadonlySet<SciotteTarget> = new Set<SciotteTarget>(['strava', 'garmin']);

function connectPrimary(card: ConnectPlanCard): ConnectPrimary {
  const target = sciotteTargetForBackend(card.provider);
  if (target && DUAL_BACKEND_TARGETS.has(target)) return dualBackendPrimary(card, target);
  if (target) return { kind: 'sign_in', target };
  if (card.provider === 'intervals_icu') return { kind: 'intervals', path: intervalsConnectPath(card) };
  if (card.requires_oauth) return { kind: 'oauth', provider: card.provider };
  return { kind: 'none' };
}

/**
 * The account sign-in that copies a connected card's history: offered while
 * that history is `needed` or did not finish, for the target the server names.
 */
function historySignIn(card: ConnectPlanCard, state: HistoryState): SciotteTarget | null {
  if (state !== 'needed' && state !== 'incomplete') return null;
  const signIn = card.history?.sign_in_target;
  return signIn ? sciotteTargetForBackend(signIn) : null;
}

/** What `card` offers: how it connects or reconnects, whether it should move, and the sign-in its history needs. */
export function connectPlan(card: ConnectPlanCard): ConnectPlan {
  const state = cardHistoryState(card);
  return {
    primary: connectPrimary(card),
    migrate: migrationAsked(card),
    history: { state, signIn: historySignIn(card, state) },
  };
}

/**
 * What a card shows, the first match winning:
 * - `authorize_owed`: connected, but sync waits on a notice (WHOOP's owner authorization).
 * - `reconnect`: connected, and its serving connection is no longer usable.
 * - `migrate`: served by a sign-in session the server asks to move to the official connection.
 * - `history_*`: a connected card's history state, when it has one.
 * - `connected`, else `connect`.
 */
export type CardStatus =
  | 'authorize_owed'
  | 'reconnect'
  | 'migrate'
  | 'history_needed'
  | 'history_incomplete'
  | 'history_importing'
  | 'history_imported'
  | 'connected'
  | 'connect';

/** What `card` shows, in the order {@link CardStatus} lists. */
export function cardStatus(card: ConnectPlanCard): CardStatus {
  if (syncAuthorizationOwed(card.provider, card.connected, card.consent_required)) return 'authorize_owed';
  if (card.connected && card.needs_reauth) return 'reconnect';
  if (migrationAsked(card)) return 'migrate';
  const state = cardHistoryState(card);
  if (state !== 'not_applicable') return `history_${state}`;
  return card.connected ? 'connected' : 'connect';
}

/**
 * The card a user-facing provider slug names: `garmin`, `strava`, `coros` and
 * `trainingpeaks` name the card of their sign-in backend (`sciotte_garmin`,
 * `sciotte`…); any other slug is a card key itself.
 */
export function cardForSlug<T extends Pick<ExtendedProviderStatus, 'provider'>>(
  slug: string,
  cards: readonly T[],
): T | undefined {
  const preset = (SCIOTTE_LOGIN_PRESETS as Record<string, { backend: string } | undefined>)[slug];
  const key = preset ? preset.backend : slug;
  return cards.find((card) => card.provider === key);
}

/**
 * The account sign-in a refused authorization of `provider` opens instead of
 * an error, or `null`. Only Strava falls back to it; a refused Garmin grant
 * returns to its card with a retry ({@link refusalReturnsToCard}).
 */
export function credentialFallbackOnRefusal(provider: string): SciotteTarget | null {
  return provider === 'strava' ? 'strava' : null;
}

/**
 * Whether a refused authorization of `provider`, started from a card, goes
 * straight back to that card: Strava's opens its sign-in, Garmin's shows the
 * card's "try again". Any other refusal stays on the callback page, which
 * gives its reason.
 */
export function refusalReturnsToCard(provider: string): boolean {
  return provider === 'strava' || provider === 'garmin';
}

/**
 * `details.reason` of an authorization start refused because the provider's
 * OAuth seats are gone, and the `error` its return carries (mirrors the
 * server's `NO_SEAT_REASON`).
 */
export const OAUTH_NO_SEAT = 'no_seat' as const;

/**
 * `details.reason` of a Garmin sign-in the server refused because a seat is
 * free and a new connection goes through Garmin's authorization (mirrors the
 * server's `OAUTH_REQUIRED_REASON`).
 */
export const SIGN_IN_OAUTH_REQUIRED = 'oauth_required' as const;

/**
 * Whether a refused connect start means the card's plan changed under the
 * athlete — a seat went, or a seat came free — so the cards are read again
 * and the tap follows the new plan.
 */
export function refusalReloadsCards(reason: string | null | undefined): boolean {
  return reason === OAUTH_NO_SEAT || reason === SIGN_IN_OAUTH_REQUIRED;
}

/**
 * `details.reason` of a sign-in that could not complete for a reason that is
 * not the athlete's credentials — the service failed, timed out or shed the
 * login (mirrors the server's `SIGN_IN_UNAVAILABLE_REASON`).
 */
export const SIGN_IN_UNAVAILABLE = 'sign_in_unavailable' as const;

/**
 * The provider whose OAuth a failed account sign-in on `card` falls back to,
 * or `null` (carnet#1477): a card not yet connected whose account sign-in goes
 * first while an OAuth seat is free — `recommended_backend` `mirror` with
 * `seats_left` above zero, which only the operator's sign-in-first order
 * produces. With the pool full there is no OAuth to fall back to.
 */
export function oauthFallbackTarget(
  card: Pick<ConnectPlanCard, 'provider' | 'connected' | 'recommended_backend' | 'seats_left'>,
): SciotteTarget | null {
  if (card.connected || card.recommended_backend !== 'mirror' || !card.seats_left || card.seats_left <= 0) {
    return null;
  }
  const target = sciotteTargetForBackend(card.provider);
  return target && DUAL_BACKEND_TARGETS.has(target) ? target : null;
}

/**
 * Whether a refused sign-in falls back to its provider's OAuth when its card
 * offers one ({@link oauthFallbackTarget}): the sign-in could not complete
 * ({@link SIGN_IN_UNAVAILABLE}) or the provider stopped it at a challenge.
 * Wrong credentials answer `status: failed`, never a refusal, so the athlete
 * types them again instead.
 */
export function signInFallsBackToOAuth(reason: string | null | undefined): boolean {
  return reason === SIGN_IN_UNAVAILABLE || reason === SCIOTTE_LOGIN_CHALLENGE;
}
