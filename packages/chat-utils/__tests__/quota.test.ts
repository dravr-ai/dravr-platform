// ABOUTME: Unit tests for the usage banner a turn's own `notice` block produces
// ABOUTME: Red if the banner stops naming the counter it measured and goes back to scraped prose

import { describe, it, expect } from 'vitest';
import { quotaNoticeBanner } from '../src/quota';

describe('quotaNoticeBanner', () => {
  it('states the counter, the cap and the percentage for an approaching quota', () => {
    const banner = quotaNoticeBanner({
      kind: 'quota_warning',
      level: 'approaching',
      current: 45,
      limit: 50,
      resets_at: '2026-08-26T00:00:00Z',
    });

    expect(banner.level).toBe('warning');
    expect(banner.message).toContain('90%');
    expect(banner.message).toContain('(45/50)');
    expect(banner.resetsAt).toBe('2026-08-26T00:00:00Z');
  });

  it('names the burst zone with the same counters', () => {
    const banner = quotaNoticeBanner({
      kind: 'quota_warning',
      level: 'burst',
      current: 56,
      limit: 50,
      resets_at: '2026-08-26T00:00:00Z',
    });

    expect(banner.level).toBe('burst');
    expect(banner.message).toContain('burst zone');
    expect(banner.message).toContain('(56/50)');
  });

  it('does not divide by a zero cap', () => {
    const banner = quotaNoticeBanner({
      kind: 'quota_warning',
      level: 'approaching',
      current: 3,
      limit: 0,
      resets_at: '2026-08-26T00:00:00Z',
    });

    expect(banner.message).toContain('0%');
    expect(banner.message).not.toContain('NaN');
    expect(banner.message).not.toContain('Infinity');
  });
});
