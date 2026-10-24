// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the line Garmin's brand guidelines owe a derived figure — both contributions resolve in all five catalogues
// ABOUTME: Garmin's name is never translated, and no line names one of our figures as Garmin's own (carnet#1154)

import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { GARMIN_CONTRIBUTION_KEY, garminContributionLine } from '../src/garmin-attribution';

const LOCALES = ['fr', 'en', 'es', 'de', 'pt'] as const;
const LOCALES_DIR = join(dirname(fileURLToPath(import.meta.url)), '../../i18n/src/locales');

// The catalogue sits above this package, so it is read as data, as the
// training-status test reads it.
function translator(locale: string) {
  const bundle: Record<string, unknown> = JSON.parse(
    readFileSync(join(LOCALES_DIR, locale, 'translation.json'), 'utf8'),
  );
  return (key: string) => {
    const text = key
      .split('.')
      .reduce<unknown>(
        (node, part) => (typeof node === 'object' && node !== null ? (node as Record<string, unknown>)[part] : undefined),
        bundle,
      );
    if (typeof text !== 'string' || text === '') throw new Error(`${locale} has no string at ${key}`);
    return text;
  };
}

describe('Garmin contribution line', () => {
  it('reads as Garmin’s acceptable samples in English', () => {
    const t = translator('en');
    expect(garminContributionLine(t, 'sole')).toBe('Created using data provided by Garmin devices.');
    expect(garminContributionLine(t, 'part')).toBe('Created in part using data provided by Garmin devices.');
  });

  it('is absent when no Garmin data went into the figure', () => {
    const t = translator('en');
    expect(garminContributionLine(t, null)).toBeUndefined();
    expect(garminContributionLine(t, undefined)).toBeUndefined();
  });

  it('names Garmin untranslated in every locale, never as the maker of our figure', () => {
    for (const locale of LOCALES) {
      const t = translator(locale);
      for (const key of Object.values(GARMIN_CONTRIBUTION_KEY)) {
        const line = t(key);
        expect(line, `${locale} ${key}`).toContain('Garmin');
        // "Garmin speed model" is the guidelines' unacceptable example: the
        // brand never leads a line, where it would read as the figure's name.
        expect(line.startsWith('Garmin'), `${locale} ${key}: ${line}`).toBe(false);
      }
    }
  });
});
