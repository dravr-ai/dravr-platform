// ABOUTME: formatDateTime is the one stamp every "saved earlier" surface shows, in the reader's locale
// ABOUTME: Fifteen sites spelled it before this, twelve of them hard-coding en-US

import { describe, expect, it } from 'vitest';
import { formatDateTime } from '../src/date-format';

/** 13 April 2026, 18:05 UTC — a date whose month abbreviates differently per locale. */
const ISO = '2026-04-13T18:05:00Z';

describe('formatDateTime', () => {
  it('spells the month in the reader language', () => {
    const french = formatDateTime(ISO, 'fr');
    const english = formatDateTime(ISO, 'en-US');

    expect(french).toContain('avr');
    expect(french).toContain('2026');
    expect(english).toContain('Apr');
    expect(english).toContain('2026');
    expect(french).not.toBe(english);
  });

  it('carries a time, not only a date', () => {
    // The admin tables this replaces showed a time; dropping it would lose the
    // only thing that distinguishes two rows saved on one day.
    expect(formatDateTime(ISO, 'en-US')).toMatch(/\d{1,2}:\d{2}/);
    expect(formatDateTime(ISO, 'fr')).toMatch(/\d{1,2}:\d{2}/);
  });

  it('returns an unparseable stamp verbatim rather than "Invalid Date"', () => {
    expect(formatDateTime('not a date', 'fr')).toBe('not a date');
    expect(formatDateTime('', 'en-US')).toBe('');
  });
});
