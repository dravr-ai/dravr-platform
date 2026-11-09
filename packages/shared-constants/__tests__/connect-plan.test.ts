// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins what every provider card offers — its connect or reconnect, the sign-in its history needs, the move to OAuth — one case per row
// ABOUTME: Garmin with no seat signs in, a refused Garmin grant retries on its card, and no authorization ever names a sciotte_* key

import { describe, expect, it } from 'vitest';
import type { HistoryState } from '@pierre/shared-types';
import {
  OAUTH_NO_SEAT,
  SIGN_IN_OAUTH_REQUIRED,
  cardForSlug,
  cardStatus,
  connectPlan,
  credentialFallbackOnRefusal,
  refusalReloadsCards,
  refusalReturnsToCard,
  type ConnectPlanCard,
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
    expect(plan.migrate).toBe(false);
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

  it('reconnects a flagged grant through Garmin even with no seat left', () => {
    const flagged = { ...garminGrant('imported'), needs_reauth: true, recommended_backend: 'mirror' as const };
    expect(connectPlan(flagged).primary).toEqual({ kind: 'oauth', provider: 'garmin' });
    expect(cardStatus(flagged)).toBe('reconnect');
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
      migrate: false,
      history: { state: 'not_applicable', signIn: null },
    });
  });
});

describe('connectPlan — a Garmin sign-in session alone', () => {
  it('moves to the official connection when the server asks, reconnecting through Garmin', () => {
    const asked = garminSession({ migration: 'oauth_required', recommended_backend: 'oauth' });
    expect(connectPlan(asked)).toMatchObject({ primary: { kind: 'oauth', provider: 'garmin' }, migrate: true });
    expect(cardStatus(asked)).toBe('migrate');
  });

  it('a dead session the server asks to move reconnects through Garmin, read as a reconnect', () => {
    const dead = garminSession({ migration: 'oauth_required', needs_reauth: true, recommended_backend: 'oauth' });
    expect(connectPlan(dead)).toMatchObject({ primary: { kind: 'oauth', provider: 'garmin' }, migrate: true });
    expect(cardStatus(dead)).toBe('reconnect');
  });

  it('signs in again when no seat is free, so the server asks no move', () => {
    const dead = garminSession({ needs_reauth: true });
    expect(connectPlan(dead)).toMatchObject({ primary: { kind: 'sign_in', target: 'garmin' }, migrate: false });
    expect(cardStatus(dead)).toBe('reconnect');
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

  it('goes through Strava while a seat is free, and signs in once they are gone', () => {
    expect(connectPlan(strava({ recommended_backend: 'oauth' })).primary).toEqual({ kind: 'oauth', provider: 'strava' });
    expect(connectPlan(strava({ recommended_backend: 'mirror' })).primary).toEqual({ kind: 'sign_in', target: 'strava' });
  });

  it('reconnects the way it is connected, whatever the seats', () => {
    const flagged = (backend: string, recommended: 'oauth' | 'mirror') =>
      strava({ connected: true, needs_reauth: true, connected_backend: backend, recommended_backend: recommended });
    expect(connectPlan(flagged('strava', 'mirror')).primary).toEqual({ kind: 'oauth', provider: 'strava' });
    expect(connectPlan(flagged('sciotte', 'oauth')).primary).toEqual({ kind: 'sign_in', target: 'strava' });
    expect(cardStatus(flagged('sciotte', 'oauth'))).toBe('reconnect');
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
        migrate: false,
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
      garminSession({ migration: 'oauth_required', recommended_backend: 'oauth' }),
      card({ provider: 'sciotte', recommended_backend: 'oauth' }),
      card({ provider: 'sciotte', connected: true, needs_reauth: true, connected_backend: 'strava' }),
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

  it('reads the cards again when a seat went or a seat came free', () => {
    for (const reason of [OAUTH_NO_SEAT, SIGN_IN_OAUTH_REQUIRED]) {
      expect(refusalReloadsCards(reason)).toBe(true);
    }
    expect(refusalReloadsCards('login_challenge')).toBe(false);
    expect(refusalReloadsCards(undefined)).toBe(false);
  });
});
