// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Every count the clients print has a singular and a plural key in all five locales, each carrying {{count}}
// ABOUTME: Red if a count could read « 1 non lues » or « 0 activités » again: a missing form, or a form that spells its digit

import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import * as COUNT_KEYS from '../src/count-keys';
import { activitiesLine } from '../src/training-volume';
import { TURN_FOLDED_KEYS, type CountKeys } from '@pierre/chat-utils';

const LOCALES = ['en', 'fr', 'es', 'de', 'pt'] as const;

/** Pairs whose forms interpolate something other than the count: the names themselves. */
const NAMED: Record<string, { pair: CountKeys; slot: string }> = {
  PROVIDERS_CONNECTED_KEYS: { pair: COUNT_KEYS.PROVIDERS_CONNECTED_KEYS, slot: '{{providers}}' },
};

/**
 * The pairs whose two forms both interpolate the count: every pair
 * count-keys exports, so a new one is checked the day it lands, plus
 * chat-utils' folded-turn pair, which lives below this package.
 */
const COUNTED: Record<string, CountKeys> = {
  ...Object.fromEntries(Object.entries(COUNT_KEYS).filter(([name]) => !(name in NAMED))),
  TURN_FOLDED_KEYS,
};

function lookup(locale: string, dotted: string): unknown {
  // Reads the locale corpus because it is the data under test: every key a
  // count can choose must resolve in every shipped locale.
  const path = resolve(__dirname, '../../i18n/src/locales', locale, 'translation.json');
  const corpus: unknown = JSON.parse(readFileSync(path, 'utf8'));
  return dotted.split('.').reduce<unknown>((node, part) => {
    if (node && typeof node === 'object' && part in node) {
      return (node as Record<string, unknown>)[part];
    }
    return undefined;
  }, corpus);
}

describe('the count key pairs', () => {
  it('carry {{count}} in both forms in all five locales', () => {
    for (const locale of LOCALES) {
      for (const [name, { one, many }] of Object.entries(COUNTED)) {
        for (const key of [one, many]) {
          const word = lookup(locale, key);
          expect(typeof word, `${locale}:${name}:${key}`).toBe('string');
          // A singular that spells « 1 » would print it for a French 0.
          expect(word as string, `${locale}:${name}:${key}`).toContain('{{count}}');
        }
      }
    }
  });

  it('resolve both forms of every named pair in all five locales, each carrying its slot', () => {
    for (const locale of LOCALES) {
      for (const [name, { pair, slot }] of Object.entries(NAMED)) {
        for (const key of [pair.one, pair.many]) {
          const word = lookup(locale, key);
          expect(typeof word, `${locale}:${name}:${key}`).toBe('string');
          expect(word as string, `${locale}:${name}:${key}`).toContain(slot);
        }
      }
    }
  });

  it('cover every count the move named, so none slipped back out of count-keys', () => {
    for (const name of ['PER_DAY_KEYS', 'PROVIDERS_CONNECTED_KEYS', 'COLLAPSED_SIMILAR_KEYS', 'WEEK_ACTIVITY_COUNT_KEYS']) {
      expect(Object.keys(COUNT_KEYS), name).toContain(name);
    }
  });

  it('read the singular for one in French, never the plural', () => {
    expect(lookup('fr', COUNT_KEYS.NOTIFICATIONS_BELL_UNREAD_KEYS.one)).toBe('Notifications, {{count}} non lue');
    expect(lookup('fr', COUNT_KEYS.CLAIM_VERDICT_SUMMARY_STATUS_ONLY_KEYS.one)).toBe(
      "{{count}} verdict sur les affirmations — dans l'ensemble : {{status}}",
    );
  });
});

describe('activitiesLine', () => {
  const t = (key: string, values?: Record<string, string | number>) => `${key}(${JSON.stringify(values)})`;

  it('reads an empty week in the singular in French and Portuguese, in the plural in English', () => {
    expect(activitiesLine(t, 0, 'fr')).toBe('home.volume.activityCountOne({"count":0})');
    expect(activitiesLine(t, 0, 'pt')).toBe('home.volume.activityCountOne({"count":0})');
    expect(activitiesLine(t, 0, 'en')).toBe('home.volume.activityCountN({"count":0})');
    expect(activitiesLine(t, 1, 'de')).toBe('home.volume.activityCountOne({"count":1})');
    expect(activitiesLine(t, 4, 'fr')).toBe('home.volume.activityCountN({"count":4})');
  });
});
