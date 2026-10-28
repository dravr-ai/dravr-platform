// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The verdict category vocabulary — one corpus key per category, and a fallback for an unknown one
// ABOUTME: Red if a category reaches the athlete as its raw enum, a key is missing from a locale, or an abstention hides the supported count

import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { summarizeVerdicts } from '@pierre/shared-types';
import type { ClaimVerdictStatus, VerdictSummary } from '@pierre/shared-types';
import {
  VERDICT_CATEGORY_LABEL_KEY,
  VERDICT_CATEGORY_OTHER_KEY,
  VERDICT_COUNT_LABEL_KEY,
  VERDICT_SPOKEN_SEPARATOR,
  verdictCategoryLabelKey,
  verdictChipLabel,
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

/** A `t()` that resolves keys against one shipped locale, as both clients do. */
function translatorFor(locale: string) {
  return (key: string, values: Record<string, string | number> = {}): string => {
    const template = lookup(locale, key);
    if (typeof template !== 'string') return key;
    return template.replace(/\{\{(\w+)\}\}/g, (_, name: string) => String(values[name] ?? ''));
  };
}

function summaryOf(...statuses: ClaimVerdictStatus[]): VerdictSummary {
  const summary = summarizeVerdicts(statuses.map((status) => ({ status })));
  if (summary === null) throw new Error('a non-empty set always summarizes');
  return summary;
}

describe('verdictChipLabel', () => {
  it('states the supported count beside an abstention, in French', () => {
    const t = translatorFor('fr');
    expect(verdictChipLabel(t, summaryOf('supported', 'unverifiable', 'supported'), 'fr')).toBe(
      '2 appuyées · 1 invérifiable',
    );
  });

  it('leads with a finding, then the supported claims, then the abstentions', () => {
    const t = translatorFor('en');
    expect(
      verdictChipLabel(t, summaryOf('rhetorical', 'supported', 'unverifiable', 'contradicted', 'supported'), 'en'),
    ).toBe('1 contradicted · 2 supported · 1 unverifiable · 1 rhetorical');
  });

  it('names one status alone when every claim landed on it', () => {
    expect(verdictChipLabel(translatorFor('fr'), summaryOf('supported', 'supported', 'supported'), 'fr')).toBe(
      '3 appuyées',
    );
    // An all-abstention reply never prints « 0 appuyée ».
    expect(verdictChipLabel(translatorFor('fr'), summaryOf('unverifiable'), 'fr')).toBe('1 invérifiable');
  });

  it('labels the turn chips the server sends before the rows land', () => {
    // A `verdicts` reply block carries contradicted / unsupported flags alone.
    expect(verdictChipLabel(translatorFor('fr'), summaryOf('unsupported', 'unsupported'), 'fr')).toBe(
      '2 non appuyées',
    );
  });

  it('agrees the status word with its count', () => {
    const t = translatorFor('fr');
    expect(verdictChipLabel(t, summaryOf('contradicted'), 'fr')).toBe('1 contredite');
    expect(verdictChipLabel(t, summaryOf('contradicted', 'contradicted'), 'fr')).toBe('2 contredites');
    expect(verdictChipLabel(translatorFor('es'), summaryOf('supported', 'supported'), 'es')).toBe('2 respaldadas');
    expect(verdictChipLabel(translatorFor('pt'), summaryOf('unverifiable', 'unverifiable'), 'pt')).toBe(
      '2 não verificáveis',
    );
  });

  it('names every count key in all five locales, each carrying its count', () => {
    for (const locale of LOCALES) {
      for (const { one, many } of Object.values(VERDICT_COUNT_LABEL_KEY)) {
        for (const key of [one, many]) {
          const word = lookup(locale, key);
          expect(typeof word, `${locale}:${key}`).toBe('string');
          expect(word as string, `${locale}:${key}`).toContain('{{count}}');
        }
      }
    }
  });
});

describe('the spoken verdict label', () => {
  // The mobile chip's accessibility label puts the rollup in
  // app.claimVerdictsStatus's {{status}} slot; a screen reader reads it with
  // commas, never a middle dot it would name aloud.
  const EXPECTED: Record<(typeof LOCALES)[number], string> = {
    en: 'Claim verdicts: 2 supported, 1 unverifiable',
    fr: 'Verdicts sur les affirmations : 2 appuyées, 1 invérifiable',
    es: 'Veredictos sobre las afirmaciones: 2 respaldadas, 1 no verificable',
    de: 'Urteile zu den Aussagen: 2 gestützt, 1 nicht überprüfbar',
    pt: 'Vereditos sobre as afirmações: 2 sustentadas, 1 não verificável',
  };

  it('reads as one sentence in all five locales', () => {
    for (const locale of LOCALES) {
      const t = translatorFor(locale);
      const status = verdictChipLabel(t, summaryOf('supported', 'unverifiable', 'supported'), locale, VERDICT_SPOKEN_SEPARATOR);
      expect(t('app.claimVerdictsStatus', { status }), locale).toBe(EXPECTED[locale]);
    }
  });
});
