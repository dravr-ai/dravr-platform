// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins which of a count's two catalogue keys each locale reads at 0, 1 and 2
// ABOUTME: Red if French or Portuguese print « 0 signalées », or English, Spanish or German print « 0 flagged reply »

import { describe, expect, it } from 'vitest';
import { countKey } from '../src/plural';

const KEYS = { one: 'one', many: 'many' };

describe('countKey', () => {
  it('reads 0 as singular in French and Portuguese, and 1 as singular everywhere', () => {
    for (const language of ['fr', 'pt']) {
      expect(countKey(0, language, KEYS), language).toBe('one');
      expect(countKey(1, language, KEYS), language).toBe('one');
      expect(countKey(2, language, KEYS), language).toBe('many');
    }
  });

  it('reads 0 as plural in English, Spanish and German', () => {
    for (const language of ['en', 'es', 'de']) {
      expect(countKey(0, language, KEYS), language).toBe('many');
      expect(countKey(1, language, KEYS), language).toBe('one');
      expect(countKey(2, language, KEYS), language).toBe('many');
    }
  });

  it('matches a regional tag by its base language', () => {
    expect(countKey(0, 'fr-CA', KEYS)).toBe('one');
    expect(countKey(0, 'pt_BR', KEYS)).toBe('one');
    expect(countKey(0, 'en-GB', KEYS)).toBe('many');
  });

  it('falls back to the English rule for a missing or unknown language', () => {
    expect(countKey(0, undefined, KEYS)).toBe('many');
    expect(countKey(0, 'xx', KEYS)).toBe('many');
    expect(countKey(1, undefined, KEYS)).toBe('one');
  });

  it('reads a large count plural, never singular', () => {
    expect(countKey(21, 'fr', KEYS)).toBe('many');
    expect(countKey(100, 'pt', KEYS)).toBe('many');
  });
});
