// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins how a turned-down sign-in is worded: the provider's reason quoted, then the way on
// ABOUTME: A window offering Google and Apple points the athlete at them; a reason-less answer keeps the advice

import { describe, it, expect } from 'vitest';
import { signInTurnedDown } from '../src/signInTurnedDown';

// Echoes each key with its params, so the test reads which words were chosen.
const t = (key: string, params?: Record<string, string>): string =>
  params ? `${key}(${Object.entries(params).map(([k, v]) => `${k}=${v}`).join(',')})` : key;

describe('signInTurnedDown', () => {
  it("quotes the provider's reason and points an email sign-in at Google and Apple", () => {
    expect(signInTurnedDown(t, 'Strava', "  Couldn't find this account ", true)).toBe(
      "shell.sciotteTurnedDownReason(provider=Strava,reason=Couldn't find this account) "
        + 'shell.sciotteTurnedDownAdviceMethods(provider=Strava)',
    );
  });

  it('gives the plain advice where the window offers no method choice', () => {
    expect(signInTurnedDown(t, 'Garmin', 'Invalid password', false)).toBe(
      'shell.sciotteTurnedDownReason(provider=Garmin,reason=Invalid password) shell.sciotteTurnedDownAdvice',
    );
  });

  it('keeps the advice alone when the provider gave no reason', () => {
    expect(signInTurnedDown(t, 'Strava', '   ', true)).toBe('shell.sciotteTurnedDownAdviceMethods(provider=Strava)');
    expect(signInTurnedDown(t, 'Garmin', null, false)).toBe('shell.sciotteTurnedDownAdvice');
  });
});
