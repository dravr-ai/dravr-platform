// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins what every provider card offers — its connect or reconnect, and the sign-in its history needs — one case per row
// ABOUTME: Every reconnect signs in (a dead grant's too), a refused Garmin grant retries on its card, no authorization names a sciotte_* key

import { describe, expect, it } from 'vitest';
import type { HistoryState } from '@pierre/shared-types';
import {
  OAUTH_NO_SEAT,
  cardForSlug,
  cardStatus,
  connectPlan,
  credentialFallbackOnRefusal,
  refusalReloadsCards,
  refusalReturnsToCard,
  type ConnectPlanCard,
  SIGN_IN_UNAVAILABLE,
  oauthFallbackTarget,
  signInFallsBackToOAuth,
} from '../src/connect-plan';

const card = (overrides: Partial<ConnectPlanCard> & Pick<ConnectPlanCard, 'provider'>): ConnectPlanCard => ({
  connected: false,
  needs_reauth: false,
  requires_oauth: true,
  own_app_required: false,
  consent_required: false,
  ...overrides,
});

const GARMIN_EXPORT = 'garmin_archive';
const STRAVA_EXPORT = 'strava_archive';

/** A Garmin card connected through its grant, its history in `state`. */
const garminGrant = (state: HistoryState, archive: string | null = GARMIN_EXPORT): ConnectPlanCard =>
  card({
    provider: 'sciotte_garmin',
    connected: true,
    connected_backend: 'garmin',
    connection_type: 'oauth',
    recommended_backend: 'oauth',
    history: { state, sign_in_target: 'sciotte_garmin', archive_source: archive },
  });

/** A disconnected Garmin card, as the server serves it with `seats` OAuth seats left. */
const garminNew = (seats: number, archive: string | null = GARMIN_EXPORT): ConnectPlanCard =>
  card({
    provider: 'sciotte_garmin',
    recommended_backend: seats > 0 ? 'oauth' : 'mirror',
    history: { state: 'not_applicable', sign_in_target: null, archive_source: archive },
  });

/** A Garmin card served by a sign-in session alone. */
const garminSession = (overrides: Partial<ConnectPlanCard> = {}): ConnectPlanCard =>
  card({
    provider: 'sciotte_garmin',
    connected: true,
    connected_backend: 'sciotte_garmin',
    connection_type: 'oauth',
    recommended_backend: 'mirror',
    history: { state: 'not_applicable', sign_in_target: null, archive_source: GARMIN_EXPORT },
    ...overrides,
  });

describe('connectPlan — Garmin grant holder (carnet#1386)', () => {
  it('names the Garmin sign-in that copies the history while it is needed (carnet#1472)', () => {
    const plan = connectPlan(garminGrant('needed'));
    expect(plan.primary).toEqual({ kind: 'oauth', provider: 'garmin' });
    expect(plan.history).toEqual({ state: 'needed', signIn: 'garmin' });
    expect(cardStatus(garminGrant('needed'))).toBe('history_needed');
  });

  it('names it again after a copy that did not finish', () => {
    expect(connectPlan(garminGrant('incomplete')).history.signIn).toBe('garmin');
    expect(cardStatus(garminGrant('incomplete'))).toBe('history_incomplete');
  });

  it('names no sign-in while the copy runs or once the history is in', () => {
    for (const state of ['importing', 'imported'] as const) {
      expect(connectPlan(garminGrant(state)).history).toEqual({ state, signIn: null });
      expect(cardStatus(garminGrant(state))).toBe(`history_${state}`);
    }
  });

  it('reconnects a flagged grant through the Garmin sign-in, whatever the seats (carnet#1494)', () => {
    for (const recommended of ['mirror', 'oauth'] as const) {
      const flagged = { ...garminGrant('imported'), needs_reauth: true, recommended_backend: recommended };
      expect(connectPlan(flagged).primary).toEqual({ kind: 'sign_in', target: 'garmin' });
      expect(cardStatus(flagged)).toBe('reconnect');
    }
  });
});

describe('connectPlan — a new Garmin connection', () => {
  it('goes through Garmin while a seat is free', () => {
    expect(connectPlan(garminNew(4)).primary).toEqual({ kind: 'oauth', provider: 'garmin' });
    expect(cardStatus(garminNew(4))).toBe('connect');
  });

  it('signs in with the Garmin account once the seats are gone, the card staying connectable', () => {
    const full = garminNew(0);
    expect(connectPlan(full).primary).toEqual({ kind: 'sign_in', target: 'garmin' });
    expect(cardStatus(full)).toBe('connect');
  });

  it('names no history sign-in on a card that is not connected', () => {
    expect(connectPlan(garminNew(0)).history).toEqual({ state: 'not_applicable', signIn: null });
  });

  it('goes through Garmin from a server that serves no history (an older card shape)', () => {
    const old = card({ provider: 'sciotte_garmin', recommended_backend: 'oauth' });
    expect(connectPlan(old)).toEqual({
      primary: { kind: 'oauth', provider: 'garmin' },
      history: { state: 'not_applicable', signIn: null },
    });
  });
});

describe('connectPlan — a Garmin sign-in session alone', () => {
  it('signs in again when it dies, whatever the seats', () => {
    for (const recommended of ['mirror', 'oauth'] as const) {
      const dead = garminSession({ needs_reauth: true, recommended_backend: recommended });
      expect(connectPlan(dead).primary).toEqual({ kind: 'sign_in', target: 'garmin' });
      expect(cardStatus(dead)).toBe('reconnect');
    }
    expect(cardStatus(garminSession())).toBe('connected');
  });
});

describe('connectPlan — Strava', () => {
  const strava = (overrides: Partial<ConnectPlanCard>): ConnectPlanCard =>
    card({
      provider: 'sciotte',
      history: { state: 'not_applicable', sign_in_target: null, archive_source: STRAVA_EXPORT },
      ...overrides,
    });

  it('goes through Strava while the server recommends it, and signs in otherwise', () => {
    expect(connectPlan(strava({ recommended_backend: 'oauth' })).primary).toEqual({ kind: 'oauth', provider: 'strava' });
    expect(connectPlan(strava({ recommended_backend: 'mirror' })).primary).toEqual({ kind: 'sign_in', target: 'strava' });
  });

  it('reconnects through the Strava sign-in whichever backend died, whatever the seats (carnet#1494)', () => {
    const flagged = (backend: string, recommended: 'oauth' | 'mirror') =>
      strava({ connected: true, needs_reauth: true, connected_backend: backend, recommended_backend: recommended });
    for (const backend of ['strava', 'sciotte']) {
      for (const recommended of ['oauth', 'mirror'] as const) {
        expect(connectPlan(flagged(backend, recommended)).primary).toEqual({ kind: 'sign_in', target: 'strava' });
        expect(cardStatus(flagged(backend, recommended))).toBe('reconnect');
      }
    }
  });

  it('chains no history sign-in after a Strava grant', () => {
    const granted = strava({ connected: true, connected_backend: 'strava', recommended_backend: 'oauth' });
    expect(connectPlan(granted).history).toEqual({ state: 'not_applicable', signIn: null });
  });
});

describe('connectPlan — other providers', () => {
  it('signs COROS and TrainingPeaks in with no history sign-in beside it', () => {
    for (const [provider, target] of [
      ['sciotte_coros', 'coros'],
      ['sciotte_trainingpeaks', 'trainingpeaks'],
    ] as const) {
      const signIn = card({
        provider,
        consent_required: true,
        history: { state: 'not_applicable', sign_in_target: null, archive_source: null },
      });
      expect(connectPlan(signIn)).toEqual({
        primary: { kind: 'sign_in', target },
        history: { state: 'not_applicable', signIn: null },
      });
    }
  });

  it('asks a connected WHOOP that owes its authorization for it, through WHOOP', () => {
    const whoop = card({ provider: 'whoop', connected: true, consent_required: true });
    expect(cardStatus(whoop)).toBe('authorize_owed');
    expect(connectPlan(whoop).primary).toEqual({ kind: 'oauth', provider: 'whoop' });
  });

  it('opens the Intervals.icu dialog along its path', () => {
    expect(connectPlan(card({ provider: 'intervals_icu' })).primary).toEqual({ kind: 'intervals', path: 'choose' });
    expect(
      connectPlan(card({ provider: 'intervals_icu', connected: true, connection_type: 'manual' })).primary,
    ).toEqual({ kind: 'intervals', path: 'key' });
  });

  it('connects a provider without OAuth with nothing to do', () => {
    expect(connectPlan(card({ provider: 'synthetic', requires_oauth: false })).primary).toEqual({ kind: 'none' });
  });

  it('reads a healthy connection as connected', () => {
    expect(cardStatus(card({ provider: 'wahoo', connected: true }))).toBe('connected');
  });
});

describe('connectPlan — history sign-in', () => {
  it('offers nothing on a card that carries no history', () => {
    expect(connectPlan(card({ provider: 'sciotte_garmin', connected: true, connected_backend: 'garmin' })).history).toEqual({
      state: 'not_applicable',
      signIn: null,
    });
  });

  it('names the sign-in whether or not the server puts an export source on the card', () => {
    expect(connectPlan(garminGrant('needed', null)).history.signIn).toBe('garmin');
  });

  it('offers no sign-in for a target no login exists for', () => {
    const unknown = {
      ...garminGrant('needed', null),
      history: { state: 'needed' as const, sign_in_target: 'sciotte_unknown', archive_source: null },
    };
    expect(connectPlan(unknown).history.signIn).toBeNull();
  });
});

describe('connectPlan — an authorization never names a sign-in backend', () => {
  it('names the provider the authorize route takes on every card', () => {
    const cards = [
      garminGrant('needed'),
      garminNew(2),
      card({ provider: 'sciotte', recommended_backend: 'oauth' }),
      card({ provider: 'whoop' }),
    ];
    for (const each of cards) {
      const { primary } = connectPlan(each);
      expect(primary.kind).toBe('oauth');
      if (primary.kind === 'oauth') expect(primary.provider.startsWith('sciotte')).toBe(false);
    }
  });
});

describe('cardForSlug', () => {
  const cards = [
    card({ provider: 'sciotte_garmin' }),
    card({ provider: 'sciotte' }),
    card({ provider: 'sciotte_coros' }),
    card({ provider: 'whoop' }),
  ];

  it('reads a user-facing slug as the card of its sign-in backend', () => {
    expect(cardForSlug('garmin', cards)?.provider).toBe('sciotte_garmin');
    expect(cardForSlug('strava', cards)?.provider).toBe('sciotte');
    expect(cardForSlug('coros', cards)?.provider).toBe('sciotte_coros');
  });

  it('reads any other slug as a card key', () => {
    expect(cardForSlug('sciotte_coros', cards)?.provider).toBe('sciotte_coros');
    expect(cardForSlug('whoop', cards)?.provider).toBe('whoop');
    expect(cardForSlug('polar', cards)).toBeUndefined();
  });
});

describe('a refused connect', () => {
  it('opens the sign-in for Strava alone', () => {
    expect(credentialFallbackOnRefusal('strava')).toBe('strava');
    expect(credentialFallbackOnRefusal('garmin')).toBeNull();
    expect(credentialFallbackOnRefusal('whoop')).toBeNull();
  });

  it('returns a refused Strava or Garmin grant to its card, any other to the callback page', () => {
    expect(refusalReturnsToCard('strava')).toBe(true);
    expect(refusalReturnsToCard('garmin')).toBe(true);
    expect(refusalReturnsToCard('whoop')).toBe(false);
  });

  it('reads the cards again when the seat it was shown went', () => {
    expect(refusalReloadsCards(OAUTH_NO_SEAT)).toBe(true);
    expect(refusalReloadsCards('oauth_required')).toBe(false);
    expect(refusalReloadsCards('login_challenge')).toBe(false);
    expect(refusalReloadsCards(undefined)).toBe(false);
  });
});

describe('the OAuth a failed sign-in falls back to (carnet#1477)', () => {
  const signInFirst = (provider: string, seats: number, connected = false) =>
    card({ provider, connected, recommended_backend: 'mirror', seats_left: seats });

  it('falls back to the provider OAuth on a card whose sign-in goes first with a seat free', () => {
    expect(oauthFallbackTarget(signInFirst('sciotte', 3))).toBe('strava');
    expect(oauthFallbackTarget(signInFirst('sciotte_garmin', 1))).toBe('garmin');
  });

  it('has no OAuth to fall back to with the pool full, on OAuth first, once connected (a reconnect too), or for COROS', () => {
    expect(oauthFallbackTarget(signInFirst('sciotte', 0))).toBeNull();
    expect(oauthFallbackTarget(card({ provider: 'sciotte', recommended_backend: 'oauth', seats_left: 3 }))).toBeNull();
    expect(oauthFallbackTarget(signInFirst('sciotte', 3, true))).toBeNull();
    const deadGrant: ConnectPlanCard = {
      ...signInFirst('sciotte_garmin', 3, true),
      connected_backend: 'garmin',
      needs_reauth: true,
    };
    expect(oauthFallbackTarget(deadGrant)).toBeNull();
    expect(oauthFallbackTarget(signInFirst('sciotte_coros', 3))).toBeNull();
    expect(oauthFallbackTarget(card({ provider: 'sciotte', recommended_backend: 'mirror' }))).toBeNull();
  });

  it('falls back on a sign-in that could not complete or met a challenge, never on any other refusal', () => {
    expect(signInFallsBackToOAuth(SIGN_IN_UNAVAILABLE)).toBe(true);
    expect(signInFallsBackToOAuth('login_challenge')).toBe(true);
    for (const reason of ['oauth_required', 'login_flow_expired', 'login_cancelled', 'no_seat', null, undefined]) {
      expect(signInFallsBackToOAuth(reason)).toBe(false);
    }
  });
});
