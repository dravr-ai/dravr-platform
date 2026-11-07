// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins how a provider card reads its history — the state it shows, its action's words, and when it is read again
// ABOUTME: The choices the action opens are connectPlan's, pinned in connect-plan.test.ts

import { describe, expect, it } from 'vitest';
import type { HistoryState } from '@pierre/shared-types';
import en from '../../i18n/src/locales/en/translation.json';
import fr from '../../i18n/src/locales/fr/translation.json';
import es from '../../i18n/src/locales/es/translation.json';
import de from '../../i18n/src/locales/de/translation.json';
import pt from '../../i18n/src/locales/pt/translation.json';
import { HISTORY_ACTION_KEYS, HISTORY_LINE_KEYS, cardHistoryState, historyActionKey, historyImporting } from '../src/providers';

function lookup(catalogue: unknown, key: string): unknown {
  return key.split('.').reduce<unknown>(
    (node, part) => (node && typeof node === 'object' ? (node as Record<string, unknown>)[part] : undefined),
    catalogue,
  );
}

const garmin = (state: HistoryState, connected = true) => ({
  connected,
  history: { state, sign_in_target: 'sciotte_garmin', archive_source: null },
});

describe('card history (carnet#1386)', () => {
  it('reads the served state on a connected card', () => {
    for (const state of ['needed', 'importing', 'incomplete', 'imported', 'not_applicable'] as const) {
      expect(cardHistoryState(garmin(state))).toBe(state);
    }
  });

  it('reads nothing on a card that is not connected or carries no history', () => {
    expect(cardHistoryState(garmin('needed', false))).toBe('not_applicable');
    expect(cardHistoryState({ connected: true })).toBe('not_applicable');
  });

  it('offers to try again after a copy that did not finish, and to add the history otherwise', () => {
    expect(historyActionKey(garmin('incomplete'))).toBe(HISTORY_ACTION_KEYS.retry);
    expect(historyActionKey(garmin('needed'))).toBe(HISTORY_ACTION_KEYS.add);
  });

  it('names only keys every locale carries', () => {
    for (const catalogue of [en, fr, es, de, pt]) {
      for (const key of [...Object.values(HISTORY_ACTION_KEYS), ...Object.values(HISTORY_LINE_KEYS)]) {
        expect(typeof lookup(catalogue, key), key).toBe('string');
      }
    }
  });

  it('is worth reading again only while a card is copying', () => {
    expect(historyImporting([garmin('imported'), garmin('importing')])).toBe(true);
    expect(historyImporting([garmin('imported'), garmin('needed'), garmin('importing', false)])).toBe(false);
  });
});
