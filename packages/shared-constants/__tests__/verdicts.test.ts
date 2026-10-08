// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The verdict category vocabulary — one corpus key per category, and a fallback for an unknown one
// ABOUTME: Red if a category reaches the athlete as its raw enum, or a key is missing from any locale

import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import {
  VERDICT_CATEGORY_LABEL_KEY,
  VERDICT_CATEGORY_OTHER_KEY,
  verdictCategoryLabelKey,
} from '../src/verdicts';

const LOCALES = ['en', 'fr', 'es', 'de', 'pt'] as const;

function lookup(locale: string, dotted: string): unknown {
  // Reads the locale corpus because it is the data under test — a catalogue
  // scan: every category key must resolve in every shipped locale.
  const path = resolve(__dirname, '../../i18n/src/locales', locale, 'translation.json');
  const corpus: unknown = JSON.parse(readFileSync(path, 'utf8'));
  return dotted.split('.').reduce<unknown>((node, part) => {
    if (node && typeof node === 'object' && part in node) {
      return (node as Record<string, unknown>)[part];
    }
    return undefined;
  }, corpus);
}

describe('verdictCategoryLabelKey', () => {
  it('maps every known category to its own key', () => {
    for (const [category, key] of Object.entries(VERDICT_CATEGORY_LABEL_KEY)) {
      expect(verdictCategoryLabelKey(category)).toBe(key);
    }
    expect(verdictCategoryLabelKey('training_prescription')).toBe('chat.verdictCategoryTrainingPrescription');
  });

  it('falls back to the generic word for a category this client does not know', () => {
    expect(verdictCategoryLabelKey('sleep_hygiene')).toBe(VERDICT_CATEGORY_OTHER_KEY);
    expect(verdictCategoryLabelKey('')).toBe(VERDICT_CATEGORY_OTHER_KEY);
    // An inherited property name is not a category.
    expect(verdictCategoryLabelKey('toString')).toBe(VERDICT_CATEGORY_OTHER_KEY);
  });

  it('names every key in all five locales', () => {
    const keys = [...Object.values(VERDICT_CATEGORY_LABEL_KEY), VERDICT_CATEGORY_OTHER_KEY];
    for (const locale of LOCALES) {
      for (const key of keys) {
        const word = lookup(locale, key);
        expect(typeof word, `${locale}:${key}`).toBe('string');
        expect((word as string).length, `${locale}:${key}`).toBeGreaterThan(0);
      }
    }
  });
});
