// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A count is grouped the way the chosen language groups thousands, not the way the device does
// ABOUTME: Red if a usage figure falls back to the English comma under French or German chrome

import { describe, expect, it } from 'vitest';
import { formatCount, formatMajorCurrency, formatMinorCurrency } from '../src/number-format';

describe('formatCount', () => {
  it('groups thousands as each language writes them', () => {
    expect(formatCount(12_345, 'en')).toBe('12,345');
    // French groups with a narrow no-break space.
    expect(formatCount(12_345, 'fr')).toBe('12 345');
    expect(formatCount(12_345, 'de')).toBe('12.345');
  });

  it('prints a small count as the bare figure', () => {
    expect(formatCount(0, 'fr')).toBe('0');
    expect(formatCount(42, 'fr')).toBe('42');
  });
});

describe('formatMinorCurrency', () => {
  it('writes an amount in minor units as each language writes money', () => {
    expect(formatMinorCurrency(1250, 'usd', 'en')).toBe('$12.50');
    // French puts the symbol after a no-break space and decimals after a comma.
    expect(formatMinorCurrency(1250, 'usd', 'fr')).toBe('12,50\u00a0$US');
    expect(formatMinorCurrency(1250, 'eur', 'fr')).toBe('12,50\u00a0€');
  });

  it('reads a currency without a subdivision as whole units, not hundredths', () => {
    // Stripe sends a yen amount as the yen count itself: 1250 is ¥1,250, never ¥13.
    expect(formatMinorCurrency(1250, 'jpy', 'en')).toBe('¥1,250');
    expect(formatMinorCurrency(1250, 'jpy', 'fr')).toBe('1\u202f250\u00a0JPY');
  });
});

describe('formatMajorCurrency', () => {
  it('writes a whole plan price without decimals, in the language the athlete chose', () => {
    expect(formatMajorCurrency(20, 'usd', 'en')).toBe('$20');
    expect(formatMajorCurrency(20, 'usd', 'fr')).toBe('20\u00a0$US');
    expect(formatMajorCurrency(20, 'usd', 'de')).toBe('20\u00a0$');
  });

  it('keeps the cents of an amount that has them', () => {
    expect(formatMajorCurrency(12.5, 'usd', 'en')).toBe('$12.50');
    expect(formatMajorCurrency(12.5, 'usd', 'fr')).toBe('12,50\u00a0$US');
  });
});
