// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the check-in cadence parser — the four cadences and null read through, anything else rejects the body
// ABOUTME: Red if a body the server never sends is read as a cadence, which would show the athlete a choice they never made

import { describe, expect, it } from 'vitest';
import { CHECK_IN_CADENCES, isCheckInCadence, parseCheckInPreference } from '../src/check-in';

describe('parseCheckInPreference', () => {
  it('reads every cadence, and null for an athlete who never chose', () => {
    for (const cadence of CHECK_IN_CADENCES) {
      expect(parseCheckInPreference({ cadence })).toEqual({ cadence });
    }
    expect(parseCheckInPreference({ cadence: null })).toEqual({ cadence: null });
  });

  it('rejects an unknown cadence, a missing key or a body that is not an object', () => {
    expect(parseCheckInPreference({ cadence: 'monthly' })).toBeNull();
    expect(parseCheckInPreference({ cadence: 'Daily' })).toBeNull();
    expect(parseCheckInPreference({})).toBeNull();
    expect(parseCheckInPreference(null)).toBeNull();
    expect(parseCheckInPreference([{ cadence: 'daily' }])).toBeNull();
  });

  it('takes off as a cadence and nothing the server does not store', () => {
    expect(isCheckInCadence('off')).toBe(true);
    expect(isCheckInCadence('never')).toBe(false);
    expect(isCheckInCadence(3)).toBe(false);
  });
});
