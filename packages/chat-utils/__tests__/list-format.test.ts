// ABOUTME: Unit tests for joining items into one phrase in the reader's language
// ABOUTME: Pins the browser's list grammar per locale, and the comma join on a runtime without Intl.ListFormat

import { afterEach, describe, expect, it, vi } from 'vitest';
import { formatList } from '../src/list-format';

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('formatList', () => {
  it('joins items the way the language does', () => {
    expect(formatList(['Garmin'], 'en')).toBe('Garmin');
    expect(formatList(['Garmin', 'Strava'], 'en')).toBe('Garmin and Strava');
    expect(formatList(['Garmin', 'Strava', 'COROS'], 'en')).toBe('Garmin, Strava, and COROS');
    expect(formatList(['Garmin', 'Strava'], 'fr')).toBe('Garmin et Strava');
    expect(formatList(['Garmin', 'Strava', 'COROS'], 'de')).toBe('Garmin, Strava und COROS');
  });

  it('joins with a comma on a runtime whose Intl has no ListFormat, as the phone has none', () => {
    vi.stubGlobal('Intl', { ...Intl, ListFormat: undefined });
    expect(formatList(['2026-10-19', '2026-10-26'], 'fr')).toBe('2026-10-19, 2026-10-26');
  });
});
