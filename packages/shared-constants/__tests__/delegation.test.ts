// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Proves every TrainingPeaks link refusal reads as a corpus key the en catalogue carries
// ABOUTME: An unknown reason reads as the generic failure, never as a raw key string

import { describe, it, expect } from 'vitest';
import en from '../../i18n/src/locales/en/translation.json';
import {
  DELEGATION_REFUSAL_KEY,
  DELEGATION_CONNECTION_REFUSALS,
  delegationRefusalKey,
  isDelegationRefusal,
} from '../src/delegation';

function lookup(key: string): unknown {
  return key.split('.').reduce<unknown>(
    (node, part) => (node && typeof node === 'object' ? (node as Record<string, unknown>)[part] : undefined),
    en,
  );
}

describe('delegation refusal vocabulary', () => {
  it('names a key the catalogue carries for every reason', () => {
    const reasons = Object.keys(DELEGATION_REFUSAL_KEY);
    expect(reasons).toHaveLength(12);
    for (const reason of reasons) {
      const key = delegationRefusalKey(reason);
      expect(typeof lookup(key), `${reason} -> ${key}`).toBe('string');
    }
  });

  it('words the conflicts a member can meet on confirm', () => {
    expect(lookup(delegationRefusalKey('own_connection'))).toBe(
      'You already connect TrainingPeaks with your own account, and Dravr keeps reading that one.',
    );
    expect(lookup(delegationRefusalKey('already_linked'))).toBe(
      'Your TrainingPeaks is already linked through another group.',
    );
  });

  it('reads an unknown or missing reason as the generic failure', () => {
    expect(isDelegationRefusal('session_expired')).toBe(false);
    expect(isDelegationRefusal(undefined)).toBe(false);
    expect(delegationRefusalKey('session_expired')).toBe('delegation.actionFailed');
    expect(delegationRefusalKey(undefined)).toBe('delegation.actionFailed');
  });

  it('never reads a name the table inherits as a refusal', () => {
    for (const inherited of ['toString', 'constructor', 'hasOwnProperty', '__proto__']) {
      expect(isDelegationRefusal(inherited), inherited).toBe(false);
      expect(delegationRefusalKey(inherited), inherited).toBe('delegation.actionFailed');
    }
  });

  it('sends the coach to connections only for their own connection gaps', () => {
    expect([...DELEGATION_CONNECTION_REFUSALS].sort()).toEqual([
      'trainingpeaks_not_connected',
      'trainingpeaks_reconnect_needed',
      'trainingpeaks_terms_outdated',
    ]);
  });
});
