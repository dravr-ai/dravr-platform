// ABOUTME: Pins the bubble clock, the day pill and the grouping window the messenger thread draws with
// ABOUTME: 24-hour in every locale, today/yesterday as words, older days as a spelled-out date

import { describe, expect, it } from 'vitest';
import {
  dayLabelFor,
  formatMessageTime,
  isSameMessageGroup,
  localDayKey,
} from '../src/message-time';

const NOW = new Date(2026, 8, 1, 16, 18); // Tue 1 Sep 2026 16:18 local

describe('formatMessageTime', () => {
  it('is a 24-hour clock in every locale', () => {
    const stamp = new Date(2026, 8, 1, 16, 18).toISOString();
    expect(formatMessageTime(stamp, 'en-US')).toBe('16:18');
    expect(formatMessageTime(stamp, 'fr')).toBe('16:18');
    expect(formatMessageTime(new Date(2026, 8, 1, 0, 5).toISOString(), 'en-US')).toBe('00:05');
  });

  it('is empty for a stamp it cannot parse', () => {
    expect(formatMessageTime('not a date', 'en-US')).toBe('');
  });
});

describe('dayLabelFor', () => {
  it('names today and yesterday, and spells older days out', () => {
    expect(dayLabelFor(new Date(2026, 8, 1, 9).toISOString(), 'fr', NOW)).toEqual({ kind: 'today' });
    expect(dayLabelFor(new Date(2026, 7, 31, 23).toISOString(), 'fr', NOW)).toEqual({ kind: 'yesterday' });
    const older = dayLabelFor(new Date(2026, 7, 20).toISOString(), 'fr', NOW);
    expect(older.kind).toBe('date');
    expect(older.kind === 'date' && older.label).toMatch(/jeudi 20 août/);
    const lastYear = dayLabelFor(new Date(2025, 11, 31).toISOString(), 'en-US', NOW);
    expect(lastYear.kind === 'date' && lastYear.label).toMatch(/December 31, 2025/);
  });
});

describe('isSameMessageGroup', () => {
  const at = (minute: number) => new Date(2026, 8, 1, 16, minute).toISOString();

  it('groups the same author within five minutes on the same day', () => {
    expect(isSameMessageGroup({ role: 'user', created_at: at(0) }, { role: 'user', created_at: at(4) })).toBe(true);
    expect(isSameMessageGroup({ role: 'user', created_at: at(0) }, { role: 'user', created_at: at(6) })).toBe(false);
    expect(isSameMessageGroup({ role: 'user', created_at: at(0) }, { role: 'assistant', created_at: at(1) })).toBe(false);
    expect(isSameMessageGroup({ role: 'user', created_at: null }, { role: 'user', created_at: at(1) })).toBe(false);
  });

  it('keys the day on local time', () => {
    expect(localDayKey(new Date(2026, 8, 1, 0, 5).toISOString())).toBe('2026-09-01');
  });
});
