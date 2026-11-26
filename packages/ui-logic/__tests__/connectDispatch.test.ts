// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the step a provider card's connect takes on every surface, Intervals.icu's three paths above all
// ABOUTME: An authorized Intervals.icu card goes straight to its authorization; only a key or a new link opens the dialog, which says what a key replaces

import { describe, expect, it, vi } from 'vitest';
import type { ConnectPlanCard } from '@pierre/shared-constants';
import { connectAction, followConnectPlan, type ConnectHandlers } from '../src/connectDispatch';

const card = (overrides: Partial<ConnectPlanCard>): ConnectPlanCard => ({
  provider: 'intervals_icu',
  connected: false,
  requires_oauth: true,
  own_app_required: false,
  consent_required: false,
  ...overrides,
});

const handlers = (): Required<ConnectHandlers> => ({
  signIn: vi.fn(),
  authorize: vi.fn(),
  intervalsDialog: vi.fn(),
  none: vi.fn(),
});

describe('connectAction for Intervals.icu', () => {
  it("opens the key form alone on path 'key'", () => {
    const keyLinked = card({ connected: true, connection_type: 'manual' });
    expect(connectAction(keyLinked)).toEqual({
      kind: 'intervals_dialog',
      dialog: { offersAuthorize: false, replacesAuthorization: false },
    });
    expect(connectAction(card({ own_app_required: true }))).toEqual({
      kind: 'intervals_dialog',
      dialog: { offersAuthorize: false, replacesAuthorization: false },
    });
  });

  it('says the key form replaces an authorization only over an authorized card', () => {
    // With no client to authorize under, an authorized card reconnects
    // through the key form, and the key it links replaces the authorization.
    const authorizedNoClient = card({ connected: true, connection_type: 'oauth', own_app_required: true });
    expect(connectAction(authorizedNoClient)).toEqual({
      kind: 'intervals_dialog',
      dialog: { offersAuthorize: false, replacesAuthorization: true },
    });
    // A card read through a group coach's account has no authorization of
    // the athlete's own to replace.
    expect(connectAction(card({ connected: true, connection_type: 'delegated' }))).toEqual({
      kind: 'intervals_dialog',
      dialog: { offersAuthorize: true, replacesAuthorization: false },
    });
  });

  it("goes straight to the authorization on path 'authorize'", () => {
    const authorized = card({ connected: true, connection_type: 'oauth', needs_reauth: true });
    expect(connectAction(authorized)).toEqual({ kind: 'authorize', provider: 'intervals_icu' });
  });

  it("opens the dialog led by the authorization on path 'choose'", () => {
    expect(connectAction(card({}))).toEqual({
      kind: 'intervals_dialog',
      dialog: { offersAuthorize: true, replacesAuthorization: false },
    });
  });
});

describe('connectAction for the other cards', () => {
  it('signs in on a sign-in card', () => {
    expect(connectAction(card({ provider: 'sciotte_coros', requires_oauth: false }))).toEqual({
      kind: 'sign_in',
      target: 'coros',
    });
  });

  it('authorizes the provider the plan names, never the card key', () => {
    expect(connectAction(card({ provider: 'sciotte', recommended_backend: 'oauth' }))).toEqual({
      kind: 'authorize',
      provider: 'strava',
    });
    expect(connectAction(card({ provider: 'whoop' }))).toEqual({ kind: 'authorize', provider: 'whoop' });
  });

  it('takes no step for a card that connects without the athlete', () => {
    expect(connectAction(card({ provider: 'synthetic', requires_oauth: false }))).toEqual({ kind: 'none' });
  });
});

describe('followConnectPlan', () => {
  it('calls exactly the handler of the step, with its argument', () => {
    const authorized = handlers();
    followConnectPlan(card({ connected: true, connection_type: 'oauth' }), authorized);
    expect(authorized.authorize).toHaveBeenCalledExactlyOnceWith('intervals_icu');
    expect(authorized.intervalsDialog).not.toHaveBeenCalled();

    const choose = handlers();
    followConnectPlan(card({}), choose);
    expect(choose.intervalsDialog).toHaveBeenCalledExactlyOnceWith({ offersAuthorize: true, replacesAuthorization: false });
    expect(choose.authorize).not.toHaveBeenCalled();

    const key = handlers();
    followConnectPlan(card({ connected: true, connection_type: 'manual' }), key);
    expect(key.intervalsDialog).toHaveBeenCalledExactlyOnceWith({ offersAuthorize: false, replacesAuthorization: false });

    const signIn = handlers();
    followConnectPlan(card({ provider: 'sciotte_garmin', recommended_backend: 'mirror' }), signIn);
    expect(signIn.signIn).toHaveBeenCalledExactlyOnceWith('garmin');

    const none = handlers();
    followConnectPlan(card({ provider: 'synthetic', requires_oauth: false }), none);
    expect(none.none).toHaveBeenCalledOnce();
  });

  it('does nothing for a card with no step when the surface gave no handler for it', () => {
    const { signIn, authorize, intervalsDialog } = handlers();
    const rest: ConnectHandlers = { signIn, authorize, intervalsDialog };
    expect(() => followConnectPlan(card({ provider: 'synthetic', requires_oauth: false }), rest)).not.toThrow();
    expect(rest.authorize).not.toHaveBeenCalled();
  });
});
