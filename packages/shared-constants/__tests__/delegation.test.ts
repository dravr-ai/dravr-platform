// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Proves every coaching-platform link refusal reads as a corpus key the en catalogue carries
// ABOUTME: An unknown reason reads as the generic failure; every platform the copy names reads as its brand

import { describe, it, expect } from 'vitest';
import en from '../../i18n/src/locales/en/translation.json';
import {
  DELEGATION_REFUSAL_KEY,
  DELEGATION_CONNECTION_REFUSALS,
  coachPlatformName,
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
    expect(reasons).toHaveLength(19);
    for (const reason of reasons) {
      const key = delegationRefusalKey(reason);
      expect(typeof lookup(key), `${reason} -> ${key}`).toBe('string');
    }
  });

  it('words the conflicts a member can meet on confirm', () => {
    expect(lookup(delegationRefusalKey('own_connection'))).toBe(
      'You already connect {{platform}} with your own account, and Dravr keeps reading that one.',
    );
    expect(lookup(delegationRefusalKey('already_linked'))).toBe(
      'Your {{platform}} is already linked through another group.',
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
      'coach_platform_api_key_required',
      'coach_platform_email_mismatch',
      'coach_platform_email_missing',
      'coach_platform_not_connected',
      'coach_platform_reconnect_needed',
      'coach_platform_terms_outdated',
    ]);
  });

  it('asks an Intervals.icu coach linked by OAuth for their API key, naming the platform', () => {
    expect(delegationRefusalKey('coach_platform_api_key_required')).toBe('delegation.apiKeyRequired');
    expect(lookup('delegation.apiKeyRequired')).toContain('{{platform}}');
    expect(lookup('delegation.apiKeyRequired')).toContain('API key');
  });
});

describe('coaching-platform names', () => {
  it('names a platform by its brand, from its own name or its connection backend', () => {
    expect(coachPlatformName('trainingpeaks')).toBe('TrainingPeaks');
    expect(coachPlatformName('sciotte_trainingpeaks')).toBe('TrainingPeaks');
    expect(coachPlatformName('intervals_icu')).toBe('Intervals.icu');
  });

  it('names every platform a coach can connect while none is known', () => {
    for (const unknown of [undefined, null, 'strava', 'toString', 42]) {
      expect(coachPlatformName(unknown), String(unknown)).toBe('TrainingPeaks / Intervals.icu');
    }
  });

  it('gives every delegation string that names a platform the placeholder it fills', () => {
    const delegation = (en as { delegation: Record<string, string> }).delegation;
    for (const [key, text] of Object.entries(delegation)) {
      expect(text, key).not.toContain('TrainingPeaks');
    }
  });
});
