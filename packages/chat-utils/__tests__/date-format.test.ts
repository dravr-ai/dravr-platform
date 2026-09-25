// ABOUTME: formatDateTime and formatDate are the one stamp every "saved earlier" surface shows, in the reader's locale
// ABOUTME: Pins the exact strings, so a copy that hard-codes en-US or drops the time cannot pass as this

import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { formatDate, formatDateTime } from '../src/date-format';

/** 13 April 2026, 18:05 UTC — a date whose month abbreviates differently per locale. */
const ISO = '2026-04-13T18:05:00Z';

// The stamps render in the process's zone; pin it so the day and the hour are exact.
const savedZone = process.env.TZ;
beforeAll(() => {
  process.env.TZ = 'UTC';
});
afterAll(() => {
  process.env.TZ = savedZone;
});

describe('formatDateTime', () => {
  it('spells the stamp in the reader language', () => {
    expect(formatDateTime(ISO, 'fr')).toBe('13 avr. 2026, 18:05');
    expect(formatDateTime(ISO, 'en-US')).toMatch(/^Apr 13, 2026, 6:05\sPM$/);
  });

  it('carries a time, not only a date', () => {
    // The admin tables it serves showed a time; dropping it would lose the
    // only thing that distinguishes two rows saved on one day.
    expect(formatDateTime(ISO, 'de')).toMatch(/18:05$/);
  });

  it('returns an unparseable stamp verbatim rather than "Invalid Date"', () => {
    expect(formatDateTime('not a date', 'fr')).toBe('not a date');
    expect(formatDateTime('', 'en-US')).toBe('');
  });
});

describe('formatDate', () => {
  it('spells the day in the reader language, with no time', () => {
    expect(formatDate(ISO, 'en-US')).toBe('Apr 13, 2026');
    expect(formatDate(ISO, 'fr')).toBe('13 avr. 2026');
    expect(formatDate(ISO, 'es')).toBe('13 abr 2026');
  });

  it('keeps the abbreviated month where dateStyle medium would go numeric', () => {
    expect(formatDate(ISO, 'de')).toBe('13. Apr. 2026');
  });

  it('returns an unparseable stamp verbatim rather than "Invalid Date"', () => {
    expect(formatDate('not a date', 'fr')).toBe('not a date');
  });
});
