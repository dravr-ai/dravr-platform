// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Proves a card offers the username-and-password reconnect when that session expired beside a serving grant
// ABOUTME: And that a dead grant, a healthy session or a disconnected card offers none

import { describe, it, expect } from 'vitest';
import { sessionReconnectTarget } from '../src/providers';

const garmin = {
  provider: 'sciotte_garmin',
  connected: true,
  needs_reauth: false,
  session_needs_reauth: true,
};

describe('sessionReconnectTarget', () => {
  it('opens the usr/pwd login of a session expired beside a serving grant', () => {
    expect(sessionReconnectTarget(garmin)).toBe('garmin');
    expect(sessionReconnectTarget({ ...garmin, provider: 'sciotte' })).toBe('strava');
  });

  it('offers nothing while the session is healthy', () => {
    expect(sessionReconnectTarget({ ...garmin, session_needs_reauth: false })).toBeNull();
  });

  it('leaves a dead grant to the card’s own reconnect', () => {
    expect(sessionReconnectTarget({ ...garmin, needs_reauth: true })).toBeNull();
  });

  it('offers nothing on a disconnected card', () => {
    expect(sessionReconnectTarget({ ...garmin, connected: false })).toBeNull();
  });
});
