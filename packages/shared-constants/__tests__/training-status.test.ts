// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the Home training status wording — every band and line resolves in all five catalogues and reads as written
// ABOUTME: Also pins the framing: no band or line in any locale words form or load as a risk of harm

import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { FORM_BANDS } from '@pierre/shared-types';
import {
  FORM_BAND_DETAIL_KEY,
  FORM_BAND_LABEL_KEY,
  TRAINING_STATUS_KEY,
  formLine,
  loadRatioLine,
  recoveryLine,
  trendDayLine,
  trendLabelLine,
  trendSpanDays,
} from '../src/training-status';

const LOCALES = ['fr', 'en', 'es', 'de', 'pt'] as const;
const LOCALES_DIR = join(dirname(fileURLToPath(import.meta.url)), '../../i18n/src/locales');

// Why this reads files: the locale JSON is a catalogue, and this package sits
// below @pierre/i18n so it cannot import it. The keys the module emits are
// looked up in every locale's file — the data under test, not source.
function catalogue(locale: string): Record<string, unknown> {
  return JSON.parse(readFileSync(join(LOCALES_DIR, locale, 'translation.json'), 'utf8'));
}

function lookup(bundle: Record<string, unknown>, key: string): unknown {
  return key.split('.').reduce<unknown>(
    (node, part) => (typeof node === 'object' && node !== null ? (node as Record<string, unknown>)[part] : undefined),
    bundle,
  );
}

/** `t()` over one real catalogue: the string, its `{{values}}` filled in. */
function translator(locale: string) {
  const bundle = catalogue(locale);
  return (key: string, values: Record<string, string | number> = {}) => {
    const text = lookup(bundle, key);
    if (typeof text !== 'string') throw new Error(`${locale} has no string at ${key}`);
    return text.replace(/\{\{(\w+)\}\}/g, (_, name: string) => {
      if (!(name in values)) throw new Error(`${key} wants {{${name}}}`);
      return String(values[name]);
    });
  };
}

const ALL_KEYS = [
  ...Object.values(FORM_BAND_LABEL_KEY),
  ...Object.values(FORM_BAND_DETAIL_KEY),
  ...Object.values(TRAINING_STATUS_KEY),
];

describe('training status wording', () => {
  it('names every band the server can send, and only those', () => {
    expect(Object.keys(FORM_BAND_LABEL_KEY).sort()).toEqual([...FORM_BANDS].sort());
    expect(Object.keys(FORM_BAND_DETAIL_KEY).sort()).toEqual([...FORM_BANDS].sort());
  });

  it('resolves every key to a non-empty string in all five locales', () => {
    expect(ALL_KEYS).toHaveLength(27);
    for (const locale of LOCALES) {
      const bundle = catalogue(locale);
      for (const key of ALL_KEYS) {
        const text = lookup(bundle, key);
        expect({ locale, key, ok: typeof text === 'string' && text.trim().length > 0 }).toEqual({
          locale,
          key,
          ok: true,
        });
      }
    }
  });

  it('never words form or load as a risk of harm, in any locale', () => {
    const banned = /risk|injur|danger|overtrain|risque|blessure|surentra|riesgo|lesi[oó]n|sobreentren|risiko|verletz|übertrain|risco|lesão|sobretreino/i;
    for (const locale of LOCALES) {
      const bundle = catalogue(locale);
      for (const key of ALL_KEYS) {
        expect({ locale, key, banned: banned.test(String(lookup(bundle, key))) }).toEqual({
          locale,
          key,
          banned: false,
        });
      }
    }
  });

  it('prints form as a signed share of fitness, and nothing without a chronic base', () => {
    const en = translator('en');
    expect(formLine(en, { band: 'productive', pct_of_fitness: -12 })).toBe('Form -12% of your fitness');
    expect(formLine(en, { band: 'fresh', pct_of_fitness: 6 })).toBe('Form +6% of your fitness');
    expect(formLine(en, { band: 'insufficient_history', pct_of_fitness: null })).toBeNull();
    expect(formLine(translator('fr'), { band: 'productive', pct_of_fitness: -12 })).toBe(
      'Forme à -12 % de ta condition physique',
    );
  });

  it('states the load ratio as a multiple of the athlete own average, over the windows it was given', () => {
    const load = { ratio: 1.37, acute_days: 7, chronic_days: 28 };
    expect(loadRatioLine(translator('en'), load, '1.4')).toBe('Last 7 days: 1.4× your 28-day average');
    expect(loadRatioLine(translator('fr'), load, '1,4')).toBe('7 derniers jours : 1,4 × ta moyenne sur 28 jours');
  });

  it('words the recovery days for none, one and several', () => {
    const en = translator('en');
    expect(recoveryLine(en, 0)).toBe('Your form calls for no extra lighter day.');
    expect(recoveryLine(en, 1)).toBe('Your form calls for 1 lighter day.');
    expect(recoveryLine(en, 3)).toBe('Your form calls for 3 lighter days.');
  });

  it('reads one day of the trend with its band, and without a figure when it has none', () => {
    const en = translator('en');
    expect(trendDayLine(en, { band: 'heavy_block', pct_of_fitness: -22 }, '20 Sep')).toBe(
      '20 Sep · -22% · Heavy block',
    );
    expect(trendDayLine(en, { band: 'insufficient_history', pct_of_fitness: null }, '20 Sep')).toBe(
      '20 Sep · Not enough history',
    );
  });

  it('labels the trend with the days the served series covers, not the window it aims for', () => {
    const days = (count: number) =>
      Array.from({ length: count }, (_, index) => ({
        date: new Date(Date.UTC(2026, 7, 28 + index)).toISOString().slice(0, 10),
      }));
    // 43 points are 42 days end to end; 9 points are 8, across a month boundary.
    expect(trendSpanDays(days(43))).toBe(42);
    expect(trendSpanDays(days(9))).toBe(8);
    expect(trendLabelLine(translator('en'), days(9))).toBe('Your form over the last 8 days');
    expect(trendLabelLine(translator('fr'), days(9))).toBe('Ta forme sur les 8 derniers jours');
    expect(trendSpanDays(days(1))).toBeNull();
    expect(trendSpanDays([])).toBeNull();
    expect(trendLabelLine(translator('en'), [{ date: 'soon' }, { date: '2026-09-02' }])).toBeNull();
  });
});
