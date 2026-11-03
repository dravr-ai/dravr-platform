// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins which of a count's two catalogue keys each locale reads at 0, 1 and 2
// ABOUTME: Red if French or Portuguese print « 0 signalées », or English, Spanish or German print « 0 flagged reply »

import { describe, expect, it } from 'vitest';
import { countKey, ordinalCategory, ordinalKey } from '../src/plural';

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

describe('ordinalKey', () => {
  it('writes English ordinals as 1st, 2nd, 3rd, 4th, with 11th to 13th and 21st', () => {
    const suffix = (n: number) => ordinalCategory(n, 'en');
    expect([1, 2, 3, 4, 11, 12, 13, 21, 22, 23, 101, 111].map(suffix)).toEqual([
      'one', 'two', 'few', 'other', 'other', 'other', 'other', 'one', 'two', 'few', 'one', 'other',
    ]);
    expect(ordinalKey(4, 'en-GB', 'k')).toBe('k_ordinal_other');
    expect(ordinalKey(3, undefined, 'k')).toBe('k_ordinal_few');
  });

  it('writes French 1er then 2e, and one mark everywhere in Spanish, German and Portuguese', () => {
    expect(ordinalKey(1, 'fr-CA', 'k')).toBe('k_ordinal_one');
    expect(ordinalKey(2, 'fr', 'k')).toBe('k_ordinal_other');
    expect(ordinalKey(21, 'fr', 'k')).toBe('k_ordinal_other');
    for (const language of ['es', 'de', 'pt']) {
      for (const n of [1, 2, 3, 4]) expect(ordinalKey(n, language, 'k'), `${language} ${n}`).toBe('k_ordinal_other');
    }
  });
});
