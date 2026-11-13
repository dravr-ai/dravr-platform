// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: What a provider card offers — how it connects or reconnects, and the sign-in its history needs
// ABOUTME: The one decision both clients render from, so web and mobile cannot disagree on a tap, a pill or a refused grant

import type { ExtendedProviderStatus, HistoryState, SciotteLoginResponse, SciotteTarget } from '@pierre/shared-types';
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
   * The card's history: its state, and the account sign-in that brings it
   * in while it is still `needed` (`null` otherwise). A
   * Garmin grant receives only what the athlete records after connecting,
   * so that sign-in follows the grant at once (carnet#1472).
   */
  history: { state: HistoryState; signIn: SciotteTarget | null };
}

/**
 * How a card whose provider has both an authorization and an account sign-in
 * (the Strava `sciotte` card and the Garmin `sciotte_garmin` card) connects.
 *
 * A reconnect signs in, whichever backend the card is connected through: a
 * dead OAuth grant reconnects only through the hosted sign-in, as a dead
 * session does (carnet#1494). A new connection follows the server's
 * recommendation: its authorization only while the operator puts the OAuth
 * first and a seat is free, its sign-in otherwise.
 */
function dualBackendPrimary(card: ConnectPlanCard, target: SciotteTarget): ConnectPrimary {
  if (card.connected && card.needs_reauth) return { kind: 'sign_in', target };
  return card.recommended_backend === 'oauth' ? { kind: 'oauth', provider: target } : { kind: 'sign_in', target };
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
 * The account sign-in that brings a connected card's history in: offered
 * while that history is `needed`, for the target the server names.
 */
function historySignIn(card: ConnectPlanCard, state: HistoryState): SciotteTarget | null {
  if (state !== 'needed') return null;
  const signIn = card.history?.sign_in_target;
  return signIn ? sciotteTargetForBackend(signIn) : null;
}

/** What `card` offers: how it connects or reconnects, and the sign-in its history needs. */
export function connectPlan(card: ConnectPlanCard): ConnectPlan {
  const state = cardHistoryState(card);
  return {
    primary: connectPrimary(card),
    history: { state, signIn: historySignIn(card, state) },
  };
}

/**
 * What a card shows, the first match winning:
 * - `authorize_owed`: connected, but sync waits on a notice (WHOOP's owner authorization).
 * - `reconnect`: connected, and its serving connection is no longer usable.
 * - `history_*`: a connected card's history state, when it has one.
 * - `connected`, else `connect`.
 */
export type CardStatus =
  | 'authorize_owed'
  | 'reconnect'
  | 'history_needed'
  | 'history_imported'
  | 'connected'
  | 'connect';

/** What `card` shows, in the order {@link CardStatus} lists. */
export function cardStatus(card: ConnectPlanCard): CardStatus {
  if (syncAuthorizationOwed(card.provider, card.connected, card.consent_required)) return 'authorize_owed';
  if (card.connected && card.needs_reauth) return 'reconnect';
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
 * Whether a refused connect start means the card's plan changed under the
 * athlete — the seat it was shown went — so the cards are read again and the
 * tap follows the new plan.
 */
export function refusalReloadsCards(reason: string | null | undefined): boolean {
  return reason === OAUTH_NO_SEAT;
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
 * `seats_left` above zero, which the default sign-in-first order produces.
 * With the pool full there is no OAuth to fall back to, and a reconnect has
 * none: a dead grant reconnects only through its sign-in (carnet#1494).
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

/**
 * The provider whose own authorization a completed sign-in goes on to, or
 * `null` (carnet#1494): a provider whose grant writes what its session cannot
 * (Garmin's, which pushes workouts) connects and reconnects by its sign-in
 * first and its grant right after, while the server says the grant is owed.
 */
export function oauthFollowOn(answer: SciotteLoginResponse): string | null {
  return answer.status === 'connected' ? (answer.oauth_follow_on?.provider ?? null) : null;
}
