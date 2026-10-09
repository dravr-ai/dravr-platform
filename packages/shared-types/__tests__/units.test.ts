// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the units parser — a well-formed body reads through, an unknown system or a missing key rejects it
// ABOUTME: Red if a body the server never sends is read as metric, which would print a mile as a kilometre

import { describe, expect, it } from 'vitest';
import { parseUnitPreferences } from '../src/units';

const BODY = {
  preference: 'automatic',
  units: 'imperial',
  source: 'provider',
  provider: 'strava',
  provider_units: 'imperial',
  device_locale: 'en-US',
};

describe('parseUnitPreferences', () => {
  it('reads a well-formed body through', () => {
    expect(parseUnitPreferences(BODY)).toEqual(BODY);
    expect(
      parseUnitPreferences({ ...BODY, source: 'locale', provider: null, provider_units: null, device_locale: null }),
    ).toEqual({ ...BODY, source: 'locale', provider: null, provider_units: null, device_locale: null });
  });

  it('rejects an unknown system, preference or source', () => {
    expect(parseUnitPreferences({ ...BODY, units: 'nautical' })).toBeNull();
    expect(parseUnitPreferences({ ...BODY, preference: 'km' })).toBeNull();
    expect(parseUnitPreferences({ ...BODY, source: 'guess' })).toBeNull();
    expect(parseUnitPreferences({ ...BODY, provider_units: 'feet' })).toBeNull();
  });

  it('rejects a body missing a key or that is not an object', () => {
    const { device_locale: _dropped, ...partial } = BODY;
    expect(parseUnitPreferences(partial)).toBeNull();
    expect(parseUnitPreferences(null)).toBeNull();
    expect(parseUnitPreferences([BODY])).toBeNull();
  });
});
