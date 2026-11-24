// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the one provider brand table both clients read, spelled as each brand spells itself
// ABOUTME: WHOOP and COROS are all capitals; an id the table does not know answers null, an inherited key included

import { describe, expect, it } from 'vitest';
import { PROVIDER_BRAND, SIGN_IN_BRAND, providerBrand, providerName } from '../src/brands';

describe('PROVIDER_BRAND', () => {
  it('spells each brand as its owner does', () => {
    expect(PROVIDER_BRAND.whoop).toBe('WHOOP');
    expect(PROVIDER_BRAND.coros).toBe('COROS');
    expect(PROVIDER_BRAND.intervals_icu).toBe('Intervals.icu');
    expect(PROVIDER_BRAND.trainingpeaks).toBe('TrainingPeaks');
  });

  it('answers the brand of a known id, and null for any other', () => {
    expect(providerBrand('whoop')).toBe('WHOOP');
    expect(providerBrand('synthetic')).toBeNull();
    expect(providerBrand('constructor')).toBeNull();
  });

  it('names a provider by its brand, whatever the case of the id, and capitalises an id it does not know', () => {
    expect(providerName('whoop')).toBe('WHOOP');
    expect(providerName('Whoop')).toBe('WHOOP');
    expect(providerName('trainingpeaks')).toBe('TrainingPeaks');
    expect(providerName('zwift')).toBe('Zwift');
    expect(providerName('')).toBe('');
  });
});

describe('SIGN_IN_BRAND', () => {
  it('names the identity providers a sign-in goes through, apart from the data sources', () => {
    expect(SIGN_IN_BRAND).toEqual({ google: 'Google', apple: 'Apple' });
    expect(Object.values(PROVIDER_BRAND)).not.toContain('Apple');
  });
});
