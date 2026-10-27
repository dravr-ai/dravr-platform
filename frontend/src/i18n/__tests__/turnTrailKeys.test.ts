// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the turn trail's vocabulary — every step and indicator key the shared trail returns has words in all five locales
// ABOUTME: A key the catalogue lacks renders as itself, and a test comparing t(key) with t(key) cannot see it

import { describe, expect, it } from 'vitest';
import { TURN_STEP_KEYS, TURN_TRAIL_KEYS } from '@pierre/chat-utils';
import de from '../../../../packages/i18n/src/locales/de/translation.json';
import en from '../../../../packages/i18n/src/locales/en/translation.json';
import es from '../../../../packages/i18n/src/locales/es/translation.json';
import fr from '../../../../packages/i18n/src/locales/fr/translation.json';
import pt from '../../../../packages/i18n/src/locales/pt/translation.json';

const BUNDLES: Record<string, Record<string, unknown>> = {
  fr: fr as Record<string, unknown>,
  en: en as Record<string, unknown>,
  es: es as Record<string, unknown>,
  de: de as Record<string, unknown>,
  pt: pt as Record<string, unknown>,
};

function leaf(bundle: Record<string, unknown>, key: string): unknown {
  return key.split('.').reduce<unknown>((node, part) => {
    return node && typeof node === 'object' ? (node as Record<string, unknown>)[part] : undefined;
  }, bundle);
}

const KEYS = [...Object.values(TURN_STEP_KEYS), ...Object.values(TURN_TRAIL_KEYS)];

describe('turn trail vocabulary', () => {
  it('gives every key words in all five locales', () => {
    const missing = Object.entries(BUNDLES).flatMap(([locale, bundle]) =>
      KEYS.filter(key => {
        const value = leaf(bundle, key);
        return typeof value !== 'string' || value.trim() === '';
      }).map(key => `${locale}:${key}`),
    );
    expect(missing).toEqual([]);
  });

  it('carries the placeholders each key is called with, in every locale', () => {
    const params: Partial<Record<string, string[]>> = {
      [TURN_TRAIL_KEYS.elapsedSeconds]: ['seconds'],
      [TURN_TRAIL_KEYS.elapsedMinutes]: ['minutes', 'seconds'],
      [TURN_TRAIL_KEYS.elapsedHours]: ['hours', 'minutes'],
      [TURN_TRAIL_KEYS.foldedN]: ['duration', 'count'],
      [TURN_TRAIL_KEYS.foldedOne]: ['duration'],
      [TURN_TRAIL_KEYS.foldedNoSteps]: ['duration'],
      [TURN_TRAIL_KEYS.tabTitle]: ['count', 'title'],
    };
    const wrong = Object.entries(BUNDLES).flatMap(([locale, bundle]) =>
      Object.entries(params).flatMap(([key, names]) =>
        (names ?? [])
          .filter(name => !String(leaf(bundle, key)).includes(`{{${name}}}`))
          .map(name => `${locale}:${key} lacks {{${name}}}`),
      ),
    );
    expect(wrong).toEqual([]);
  });

  it('never names the agent a coach: that word is the human coach’s', () => {
    const offenders = Object.entries(BUNDLES).flatMap(([locale, bundle]) =>
      KEYS.filter(key => /coach/i.test(String(leaf(bundle, key)))).map(key => `${locale}:${key}`),
    );
    expect(offenders).toEqual([]);
  });
});
