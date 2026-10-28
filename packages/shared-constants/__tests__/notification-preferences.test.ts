// ABOUTME: The preference-surface vocabulary both clients read — blurbs, quiet-hour values, cap labels, device zone
// ABOUTME: Red if the two surfaces could describe the same switch, hour or cap in different words

import { afterEach, describe, expect, it, vi } from 'vitest';
import fr from '../../i18n/src/locales/fr/translation.json';
import pt from '../../i18n/src/locales/pt/translation.json';
import {
  NOTIFICATION_CATEGORIES,
  NOTIFICATION_CATEGORY_BLURB_KEYS,
  NOTIFICATION_CATEGORY_META,
  formatCollapsedCount,
  NOTIFICATION_MAX_PER_DAY_CHOICES,
  NOTIFICATION_QUIET_HOUR_VALUES,
  localTimezone,
  notificationCapLabel,
} from '../src/notifications';
import { COLLAPSED_SIMILAR_KEYS } from '../src/count-keys';

/** A translator that shows which key and params it was handed. */
const t = (key: string, params?: Record<string, string | number>) =>
  params ? `${key}(${JSON.stringify(params)})` : key;

/** A `t()` that resolves keys against one shipped locale and fills `{{name}}`, as both clients do. */
function translatorFor(corpus: unknown) {
  return (key: string, params: Record<string, string | number> = {}): string => {
    const template = key.split('.').reduce<unknown>((node, part) => {
      if (node && typeof node === 'object' && part in node) return (node as Record<string, unknown>)[part];
      return undefined;
    }, corpus);
    if (typeof template !== 'string') return key;
    return template.replace(/\{\{(\w+)\}\}/g, (_, name: string) => String(params[name] ?? ''));
  };
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe('NOTIFICATION_CATEGORY_BLURB_KEYS', () => {
  it('describes every category, the coach category as its agent and its human coach', () => {
    expect(Object.keys(NOTIFICATION_CATEGORY_BLURB_KEYS).sort()).toEqual([...NOTIFICATION_CATEGORIES].sort());
    expect(NOTIFICATION_CATEGORY_BLURB_KEYS.coach).toBe('notifPrefs.blurbCoach');
    expect(NOTIFICATION_CATEGORY_BLURB_KEYS.reminders).toBe('notifPrefs.blurbReminders');
  });
});

describe('NOTIFICATION_CATEGORY_META', () => {
  // « Plan publié par <coach> » sat under « Agent » (carnet#1191): the coach
  // category carries the human coach's notices as well as the agent's, so its
  // name is the activity both share, never the persona alone.
  it('names the coach category for coaching, not for the agent', () => {
    expect(NOTIFICATION_CATEGORY_META.coach.labelKey).toBe('notifPrefs.catCoach');
    expect(fr.notifPrefs.catCoach).toBe('Coaching');
    expect(fr.notifPrefs.blurbCoach).toContain('coach humain');
  });

  it('labels every category with a key the catalogue holds', () => {
    const prefs = fr.notifPrefs as Record<string, string>;
    for (const category of NOTIFICATION_CATEGORIES) {
      const [, key] = NOTIFICATION_CATEGORY_META[category].labelKey.split('.');
      expect({ category, label: prefs[key] }).toEqual({ category, label: expect.any(String) });
      const [, blurb] = NOTIFICATION_CATEGORY_BLURB_KEYS[category].split('.');
      expect({ category, blurb: prefs[blurb] }).toEqual({ category, blurb: expect.any(String) });
    }
  });
});

describe('formatCollapsedCount', () => {
  it('says nothing for a notification that collapsed nothing', () => {
    expect(formatCollapsedCount(undefined, t, 'en')).toBeNull();
    expect(formatCollapsedCount(1, t, 'en')).toBeNull();
  });

  // It printed « +1 similar » under French chrome (carnet#1191).
  it('counts the others in the catalogue, one and many', () => {
    expect(formatCollapsedCount(2, t, 'en')).toBe('notifications.collapsedSimilarOne({"count":1})');
    expect(formatCollapsedCount(4, t, 'en')).toBe('notifications.collapsedSimilarN({"count":3})');
    expect(fr.notifications.collapsedSimilarOne).toBe('+{{count}} similaire');
    expect(fr.notifications.collapsedSimilarN).toBe('+{{count}} similaires');
  });

  // carnet#1212: the form went through an inline `=== 1`; it now goes through
  // countKey with COLLAPSED_SIMILAR_KEYS, the one rule every count follows.
  it('picks the form through countKey in the athlete language', () => {
    const frT = translatorFor(fr);
    expect(formatCollapsedCount(2, frT, 'fr')).toBe('+1 similaire');
    expect(formatCollapsedCount(3, frT, 'fr-CA')).toBe('+2 similaires');
    expect(formatCollapsedCount(2, t, 'pt')).toBe(`${COLLAPSED_SIMILAR_KEYS.one}({"count":1})`);
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
    expect(notificationCapLabel(null, t, 'en')).toBe('frag.noLimit');
    expect(notificationCapLabel(1, t, 'en')).toBe('frag.perDayOne({"count":1})');
    expect(notificationCapLabel(5, t, 'en')).toBe('frag.perDayN({"count":5})');
  });

  // The form is countKey's choice, as for every other count (carnet#1212):
  // French and Portuguese read 0 in the singular where English does not.
  it('picks the form through countKey in the athlete language', () => {
    expect(notificationCapLabel(0, t, 'fr')).toBe('frag.perDayOne({"count":0})');
    expect(notificationCapLabel(0, t, 'pt')).toBe('frag.perDayOne({"count":0})');
    expect(notificationCapLabel(0, t, 'en')).toBe('frag.perDayN({"count":0})');
    expect(notificationCapLabel(3, t, 'fr')).toBe('frag.perDayN({"count":3})');
  });

  // The singular spelled « 1 par jour », so the French and Portuguese 0 that
  // countKey sends to it printed 1: the rendered text pins the count it shows.
  it('prints the count it was given, 0 and 1 alike, in French and Portuguese', () => {
    expect(notificationCapLabel(0, translatorFor(fr), 'fr')).toBe('0 par jour');
    expect(notificationCapLabel(1, translatorFor(fr), 'fr')).toBe('1 par jour');
    expect(notificationCapLabel(0, translatorFor(pt), 'pt')).toBe('0 por dia');
    expect(notificationCapLabel(1, translatorFor(pt), 'pt')).toBe('1 por dia');
  });

  it('has a label for every choice the surfaces offer', () => {
    expect(NOTIFICATION_MAX_PER_DAY_CHOICES.map((choice) => notificationCapLabel(choice, t, 'en'))).toEqual([
      'frag.noLimit',
      'frag.perDayOne({"count":1})',
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
