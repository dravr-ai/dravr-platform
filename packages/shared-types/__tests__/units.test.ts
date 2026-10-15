// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the units parser — a well-formed body reads through, an unknown system or a missing key rejects it
// ABOUTME: Red if a body the server never sends is read as metric, which would print a mile as a kilometre

import { describe, expect, it } from 'vitest';
import { parseUnitPreferences } from '../src/units';

describe('parseUnitPreferences', () => {
  it('reads either system', () => {
    expect(parseUnitPreferences({ units: 'imperial' })).toEqual({ units: 'imperial' });
    expect(parseUnitPreferences({ units: 'metric' })).toEqual({ units: 'metric' });
  });

  it('rejects an unknown system, a missing key or a non-object', () => {
    expect(parseUnitPreferences({ units: 'nautical' })).toBeNull();
    expect(parseUnitPreferences({ units: 'automatic' })).toBeNull();
    expect(parseUnitPreferences({})).toBeNull();
    expect(parseUnitPreferences(null)).toBeNull();
    expect(parseUnitPreferences([{ units: 'metric' }])).toBeNull();
  });
});
