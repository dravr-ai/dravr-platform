// ABOUTME: Unit tests for reading an RFC 3339 instant — no fraction, three, six and nine digits, offsets, and what is not an instant
// ABOUTME: Also runs every case against a Date.parse that reads only the ECMA-262 format, the floor the phone's Hermes is held to

import { afterEach, describe, expect, it, vi } from 'vitest';
import { instantMs } from '../src/instant';

/** 2026-10-16 14:05:00 UTC, the whole second every case below builds on. */
const SECOND = Date.UTC(2026, 9, 16, 14, 5, 0);

/** The one format ECMA-262 obliges every engine's `Date.parse` to read. */
const ECMA_DATE_TIME = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d{3})?(Z|[+-]\d{2}:\d{2})$/;

const READS: Array<[string, string, number]> = [
  ['no fraction', '2026-10-16T14:05:00Z', SECOND],
  ['three fractional digits', '2026-10-16T14:05:00.482Z', SECOND + 482],
  ['six fractional digits, as Postgres stores them', '2026-10-16T14:05:00.482999Z', SECOND + 482],
  ['nine fractional digits, as the clock reads them', '2026-10-16T14:05:00.482999731Z', SECOND + 482],
  ['one fractional digit', '2026-10-16T14:05:00.5Z', SECOND + 500],
  ['a negative offset with six digits', '2026-10-16T10:05:00.482913-04:00', SECOND + 482],
  ['a positive offset with no fraction', '2026-10-16T16:05:00+02:00', SECOND],
  ['a zero offset', '2026-10-16T14:05:00.482913+00:00', SECOND + 482],
  ['lower-case separators', '2026-10-16t14:05:00.482913z', SECOND + 482],
];

const REFUSED: Array<[string, string]> = [
  ['a word', 'yesterday'],
  ['an empty string', ''],
  ['a civil date, which names no instant', '2026-10-16'],
  ['a time with no offset, which names no instant', '2026-10-16T14:05:00.482913'],
  ['a dot with no digits', '2026-10-16T14:05:00.Z'],
  ['a month that does not exist', '2026-13-01T00:00:00Z'],
  ['a written-out date', 'Fri, 16 Oct 2026 14:05:00 GMT'],
];

describe('instantMs', () => {
  it.each(READS)('reads %s', (_case, value, expected) => {
    expect(instantMs(value)).toBe(expected);
  });

  it.each(REFUSED)('refuses %s', (_case, value) => {
    expect(instantMs(value)).toBeNaN();
  });

  it('cuts the fraction, so a later save never reads earlier than one it follows', () => {
    expect(instantMs('2026-10-16T14:05:00.482999Z')).toBeLessThan(instantMs('2026-10-16T14:05:00.483000Z'));
    expect(instantMs('2026-10-16T14:05:00.482913Z')).toBe(instantMs('2026-10-16T14:05:00.482Z'));
  });

  describe('on an engine whose Date.parse reads only the ECMA-262 format', () => {
    // Node's Date.parse reads six and nine fractional digits, so a test on it
    // cannot see an engine that does not. This one stands in for the floor the
    // standard sets: the spec format parses, every other string is NaN.
    afterEach(() => {
      vi.restoreAllMocks();
    });

    function strictDateParse() {
      const lenient = Date.parse.bind(Date);
      vi.spyOn(Date, 'parse').mockImplementation((text: string) =>
        ECMA_DATE_TIME.test(text) ? lenient(text) : Number.NaN,
      );
    }

    it('refuses the six-digit stamp Node reads, so the cases below do not lean on Node', () => {
      strictDateParse();
      expect(Date.parse('2026-10-16T14:05:00.482913Z')).toBeNaN();
    });

    it.each(READS)('still reads %s', (_case, value, expected) => {
      strictDateParse();
      expect(instantMs(value)).toBe(expected);
    });
  });
});
