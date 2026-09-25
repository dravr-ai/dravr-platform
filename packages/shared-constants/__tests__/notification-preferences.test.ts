// ABOUTME: The preference-surface vocabulary both clients read — blurbs, quiet-hour values, cap labels, device zone
// ABOUTME: Red if the two surfaces could describe the same switch, hour or cap in different words

import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  NOTIFICATION_CATEGORIES,
  NOTIFICATION_CATEGORY_BLURB_KEYS,
  NOTIFICATION_MAX_PER_DAY_CHOICES,
  NOTIFICATION_QUIET_HOUR_VALUES,
  localTimezone,
  notificationCapLabel,
} from '../src/notifications';

/** A translator that shows which key and params it was handed. */
const t = (key: string, params?: Record<string, string | number>) =>
  params ? `${key}(${JSON.stringify(params)})` : key;

afterEach(() => {
  vi.restoreAllMocks();
});

describe('NOTIFICATION_CATEGORY_BLURB_KEYS', () => {
  it('describes every category, the coach category as an agent', () => {
    expect(Object.keys(NOTIFICATION_CATEGORY_BLURB_KEYS).sort()).toEqual([...NOTIFICATION_CATEGORIES].sort());
    expect(NOTIFICATION_CATEGORY_BLURB_KEYS.coach).toBe('notifPrefs.blurbAgent');
    expect(NOTIFICATION_CATEGORY_BLURB_KEYS.reminders).toBe('notifPrefs.blurbReminders');
  });
});

describe('NOTIFICATION_QUIET_HOUR_VALUES', () => {
  it('offers "off" first, then every hour in the HH:MM the server compares', () => {
    expect(NOTIFICATION_QUIET_HOUR_VALUES).toHaveLength(25);
    expect(NOTIFICATION_QUIET_HOUR_VALUES[0]).toBe('');
    expect(NOTIFICATION_QUIET_HOUR_VALUES[1]).toBe('00:00');
    expect(NOTIFICATION_QUIET_HOUR_VALUES[10]).toBe('09:00');
    expect(NOTIFICATION_QUIET_HOUR_VALUES[24]).toBe('23:00');
  });
});

describe('notificationCapLabel', () => {
  it('labels no cap, one a day and N a day with their own keys', () => {
    expect(notificationCapLabel(null, t)).toBe('frag.noLimit');
    expect(notificationCapLabel(1, t)).toBe('frag.perDayOne');
    expect(notificationCapLabel(5, t)).toBe('frag.perDayN({"count":5})');
  });

  it('has a label for every choice the surfaces offer', () => {
    expect(NOTIFICATION_MAX_PER_DAY_CHOICES.map((choice) => notificationCapLabel(choice, t))).toEqual([
      'frag.noLimit',
      'frag.perDayOne',
      'frag.perDayN({"count":3})',
      'frag.perDayN({"count":5})',
      'frag.perDayN({"count":10})',
      'frag.perDayN({"count":20})',
    ]);
  });
});

describe('localTimezone', () => {
  it('names the zone the runtime resolves', () => {
    vi.spyOn(Intl, 'DateTimeFormat').mockReturnValue({
      resolvedOptions: () => ({ timeZone: 'America/Montreal' }),
    } as unknown as Intl.DateTimeFormat);
    expect(localTimezone()).toBe('America/Montreal');
  });

  it('falls to UTC when the runtime cannot name one', () => {
    vi.spyOn(Intl, 'DateTimeFormat').mockImplementation(() => {
      throw new RangeError('no zone');
    });
    expect(localTimezone()).toBe('UTC');
  });
});
