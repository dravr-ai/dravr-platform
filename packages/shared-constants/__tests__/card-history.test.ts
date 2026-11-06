// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins how a provider card reads its history — the state it shows, the sign-in it offers, and when it is read again
// ABOUTME: The sign-in target is the server's backend id resolved to the login's target, never passed through raw

import { describe, expect, it } from 'vitest';
import type { HistoryState } from '@pierre/shared-types';
import { HISTORY_ACTION_KEYS, cardHistoryState, historyActionKey, historyImporting, historyLoginTarget } from '../src/providers';

const garmin = (state: HistoryState, connected = true) => ({
  connected,
  history: { state, sign_in_target: 'sciotte_garmin', archive_source: null },
});

describe('card history (carnet#1386)', () => {
  it('offers the Garmin login while the history is needed or did not finish', () => {
    expect(historyLoginTarget(garmin('needed'))).toBe('garmin');
    expect(historyLoginTarget(garmin('incomplete'))).toBe('garmin');
  });

  it('offers nothing while it copies, once it is in, or when it does not apply', () => {
    for (const state of ['importing', 'imported', 'not_applicable'] as const) {
      expect(historyLoginTarget(garmin(state))).toBeNull();
    }
  });

  it('reads nothing on a card that is not connected or carries no history', () => {
    expect(cardHistoryState(garmin('needed', false))).toBe('not_applicable');
    expect(historyLoginTarget(garmin('needed', false))).toBeNull();
    expect(cardHistoryState({ connected: true })).toBe('not_applicable');
    expect(historyLoginTarget({ connected: true, history: { state: 'needed', sign_in_target: null, archive_source: null } })).toBeNull();
  });

  it('offers to try again after a copy that did not finish, and to sign in otherwise', () => {
    expect(historyActionKey(garmin('incomplete'))).toBe(HISTORY_ACTION_KEYS.retry);
    expect(historyActionKey(garmin('needed'))).toBe(HISTORY_ACTION_KEYS.signIn);
  });

  it('is worth reading again only while a card is copying', () => {
    expect(historyImporting([garmin('imported'), garmin('importing')])).toBe(true);
    expect(historyImporting([garmin('imported'), garmin('needed'), garmin('importing', false)])).toBe(false);
  });
});
