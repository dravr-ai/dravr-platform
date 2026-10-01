// ABOUTME: Unit tests for the usage banner a turn's own `notice` block produces
// ABOUTME: Red if the banner stops naming the counter it measured and goes back to scraped prose

import { afterAll, beforeAll, describe, it, expect } from 'vitest';
import { formatCompactNumber, formatResetTime, quotaNoticeBanner } from '../src/quota';

// The reset instant renders in the process's zone; pin it so the phrase is exact.
const savedZone = process.env.TZ;
beforeAll(() => {
  process.env.TZ = 'UTC';
});
afterAll(() => {
  process.env.TZ = savedZone;
});

describe('quotaNoticeBanner', () => {
  it('states the counter, the cap and the percentage for an approaching quota', () => {
    const banner = quotaNoticeBanner({
      kind: 'quota_warning',
      level: 'approaching',
      current: 45,
      limit: 50,
      resets_at: '2026-08-26T00:00:00Z',
    }, 'midnight UTC', 'en');

    expect(banner.level).toBe('warning');
    expect(banner.text.params?.percent).toBe(90);
    expect(banner.text.params).toMatchObject({ current: '45', limit: '50' });
    expect(banner.resetsAt).toBe('2026-08-26T00:00:00Z');
  });

  it('names the burst zone with the same counters', () => {
    const banner = quotaNoticeBanner({
      kind: 'quota_warning',
      level: 'burst',
      current: 56,
      limit: 50,
      resets_at: '2026-08-26T00:00:00Z',
    }, 'midnight UTC', 'en');

    expect(banner.level).toBe('burst');
    expect(banner.text.key).toBe('usage.burstZone');
    expect(banner.text.params).toMatchObject({ current: '56', limit: '50' });
  });

  it('groups the counter and the cap as the athlete reads numbers', () => {
    const notice = {
      kind: 'quota_warning',
      level: 'approaching',
      current: 456_792,
      limit: 500_000,
      resets_at: '2026-08-26T00:00:00Z',
    } as const;

    expect(quotaNoticeBanner(notice, 'minuit UTC', 'fr').text.params).toMatchObject({
      current: '456\u202f792',
      limit: '500\u202f000',
    });
    expect(quotaNoticeBanner(notice, 'midnight UTC', 'de').text.params).toMatchObject({
      current: '456.792',
      limit: '500.000',
    });
  });

  it('does not divide by a zero cap', () => {
    const banner = quotaNoticeBanner({
      kind: 'quota_warning',
      level: 'approaching',
      current: 3,
      limit: 0,
      resets_at: '2026-08-26T00:00:00Z',
    }, 'midnight UTC', 'en');

    expect(banner.text.params?.percent).toBe(0);
    expect(String(banner.text.params?.percent)).not.toContain('NaN');
    expect(String(banner.text.params?.percent)).not.toContain('Infinity');
  });
});

describe('formatResetTime', () => {
  it('names the reset instant with its hour and its zone', () => {
    // The default locale decides 12- or 24-hour and how it spells the
    // meridiem (`AM`, or `a.m.` in Canadian English); the zone name is always
    // printed.
    expect(formatResetTime('2026-08-26T00:00:00Z', 'midnight UTC', 'en')).toMatch(/^12:00\sAM UTC$/);
  });

  it('hands back the caller wording for an instant it cannot parse', () => {
    expect(formatResetTime('not a date', 'minuit UTC', 'fr')).toBe('minuit UTC');
  });

  it('spells the instant in the language the athlete chose, not the device default', () => {
    expect(formatResetTime('2026-08-26T00:00:00Z', 'minuit UTC', 'fr')).toBe('00:00 UTC');
  });

  it('feeds the same phrase into the notice banner', () => {
    const banner = quotaNoticeBanner({
      kind: 'quota_warning',
      level: 'approaching',
      current: 45,
      limit: 50,
      resets_at: 'garbled',
    }, 'minuit UTC', 'fr');

    expect(banner.text.params?.time).toBe('minuit UTC');
  });
});

describe('formatCompactNumber', () => {
  // The catalogue's suffixes, as `t` would hand them back. The keys are
  // spelled out: they are the contract with the catalogue.
  const SUFFIXES: Record<string, Record<string, string>> = {
    en: { 'common.compact.thousands': '{{value}}K', 'common.compact.millions': '{{value}}M' },
    fr: { 'common.compact.thousands': '{{value}} k', 'common.compact.millions': '{{value}} M' },
  };
  const translator = (language: string) => (key: string, params?: Record<string, string | number>) =>
    (SUFFIXES[language][key] ?? key).replace('{{value}}', String(params?.value));

  it('compacts millions and thousands to one decimal', () => {
    expect(formatCompactNumber(2_000_000, translator('en'), 'en')).toBe('2.0M');
    expect(formatCompactNumber(145_000, translator('en'), 'en')).toBe('145.0K');
    expect(formatCompactNumber(1_000, translator('en'), 'en')).toBe('1.0K');
  });

  it('writes the decimal comma and the French suffix in French', () => {
    expect(formatCompactNumber(1_500, translator('fr'), 'fr')).toBe('1,5 k');
    expect(formatCompactNumber(2_000_000, translator('fr'), 'fr')).toBe('2,0 M');
  });

  it('prints a figure under a thousand as it is', () => {
    expect(formatCompactNumber(999, translator('en'), 'en')).toBe('999');
    expect(formatCompactNumber(0, translator('en'), 'en')).toBe('0');
  });
});
