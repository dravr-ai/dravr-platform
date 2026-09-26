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
    }, 'midnight UTC');

    expect(banner.level).toBe('warning');
    expect(banner.text.params?.percent).toBe(90);
    expect(banner.text.params).toMatchObject({ current: 45, limit: 50 });
    expect(banner.resetsAt).toBe('2026-08-26T00:00:00Z');
  });

  it('names the burst zone with the same counters', () => {
    const banner = quotaNoticeBanner({
      kind: 'quota_warning',
      level: 'burst',
      current: 56,
      limit: 50,
      resets_at: '2026-08-26T00:00:00Z',
    }, 'midnight UTC');

    expect(banner.level).toBe('burst');
    expect(banner.text.key).toBe('usage.burstZone');
    expect(banner.text.params).toMatchObject({ current: 56, limit: 50 });
  });

  it('does not divide by a zero cap', () => {
    const banner = quotaNoticeBanner({
      kind: 'quota_warning',
      level: 'approaching',
      current: 3,
      limit: 0,
      resets_at: '2026-08-26T00:00:00Z',
    }, 'midnight UTC');

    expect(banner.text.params?.percent).toBe(0);
    expect(String(banner.text.params?.percent)).not.toContain('NaN');
    expect(String(banner.text.params?.percent)).not.toContain('Infinity');
  });
});

describe('formatResetTime', () => {
  it('names the reset instant with its hour and its zone', () => {
    // The default locale decides 12- or 24-hour; the zone name is always printed.
    expect(formatResetTime('2026-08-26T00:00:00Z', 'midnight UTC')).toMatch(/^(12:00\sAM|0?0:00) UTC$/);
  });

  it('hands back the caller wording for an instant it cannot parse', () => {
    expect(formatResetTime('not a date', 'minuit UTC')).toBe('minuit UTC');
  });

  it('feeds the same phrase into the notice banner', () => {
    const banner = quotaNoticeBanner({
      kind: 'quota_warning',
      level: 'approaching',
      current: 45,
      limit: 50,
      resets_at: 'garbled',
    }, 'minuit UTC');

    expect(banner.text.params?.time).toBe('minuit UTC');
  });
});

describe('formatCompactNumber', () => {
  it('compacts millions and thousands to one decimal', () => {
    expect(formatCompactNumber(2_000_000)).toBe('2.0M');
    expect(formatCompactNumber(145_000)).toBe('145.0K');
    expect(formatCompactNumber(1_000)).toBe('1.0K');
  });

  it('prints a figure under a thousand as it is', () => {
    expect(formatCompactNumber(999)).toBe('999');
    expect(formatCompactNumber(0)).toBe('0');
  });
});
