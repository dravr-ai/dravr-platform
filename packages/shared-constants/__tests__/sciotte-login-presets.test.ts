// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Proves the one credential-login preset table both clients read names text in all five locales
// ABOUTME: And that each provider card resolves to its login target and back through that same table

import { describe, it, expect } from 'vitest';
import en from '../../i18n/src/locales/en/translation.json';
import fr from '../../i18n/src/locales/fr/translation.json';
import es from '../../i18n/src/locales/es/translation.json';
import de from '../../i18n/src/locales/de/translation.json';
import pt from '../../i18n/src/locales/pt/translation.json';
import {
  PROVIDER_NOTICES,
  SCIOTTE_LOGIN_PRESETS,
  loginIdentifierField,
  sciotteTargetForBackend,
} from '../src/providers';

function lookup(catalogue: unknown, key: string): unknown {
  return key.split('.').reduce<unknown>(
    (node, part) => (node && typeof node === 'object' ? (node as Record<string, unknown>)[part] : undefined),
    catalogue,
  );
}

describe('SCIOTTE_LOGIN_PRESETS', () => {
  it('maps each provider card to its login target and back', () => {
    expect(sciotteTargetForBackend('sciotte')).toBe('strava');
    expect(sciotteTargetForBackend('sciotte_garmin')).toBe('garmin');
    expect(sciotteTargetForBackend('sciotte_trainingpeaks')).toBe('trainingpeaks');
    expect(sciotteTargetForBackend('sciotte_coros')).toBe('coros');
    expect(sciotteTargetForBackend('whoop')).toBeNull();
    expect(sciotteTargetForBackend('strava')).toBeNull();
  });

  it('names Garmin as the athlete knows it, not the OAuth API', () => {
    expect(lookup(en, SCIOTTE_LOGIN_PRESETS.garmin.labelKey)).toBe('Garmin');
  });

  it('signs TrainingPeaks in with a username, Garmin with either, the rest with an email', () => {
    expect(SCIOTTE_LOGIN_PRESETS.trainingpeaks.identifier).toBe('username');
    expect(SCIOTTE_LOGIN_PRESETS.garmin.identifier).toBe('email_or_username');
    for (const target of ['strava', 'coros'] as const) {
      expect(SCIOTTE_LOGIN_PRESETS[target].identifier).toBe('email');
    }
  });

  it('types the account field email only where nothing but an email is accepted', () => {
    expect(loginIdentifierField('email').inputType).toBe('email');
    // A Garmin username such as "jpbsavard" has no @: an email-typed field
    // would refuse it before it is sent.
    expect(loginIdentifierField('email_or_username')).toEqual({
      labelKey: 'common.emailOrUsername',
      inputType: 'text',
      autoComplete: 'username',
    });
    expect(loginIdentifierField('username').inputType).toBe('text');
    for (const kind of ['email', 'username', 'email_or_username'] as const) {
      expect(lookup(en, loginIdentifierField(kind).labelKey), kind).toBeTruthy();
    }
  });

  it('carries the exposure notice of the targets that ask one', () => {
    expect(SCIOTTE_LOGIN_PRESETS.trainingpeaks.notice).toBe(PROVIDER_NOTICES.sciotte_trainingpeaks);
    expect(SCIOTTE_LOGIN_PRESETS.coros.notice).toBe(PROVIDER_NOTICES.sciotte_coros);
    expect(SCIOTTE_LOGIN_PRESETS.strava.notice).toBeUndefined();
    expect(SCIOTTE_LOGIN_PRESETS.garmin.notice).toBeUndefined();
  });

  it('reads every preset key as text in all five locales', () => {
    for (const [target, preset] of Object.entries(SCIOTTE_LOGIN_PRESETS)) {
      for (const [locale, catalogue] of Object.entries({ en, fr, es, de, pt })) {
        for (const key of [preset.labelKey, preset.titleKey, preset.placeholderKey]) {
          const value = lookup(catalogue, key);
          expect(typeof value, `${target} ${key} in ${locale}`).toBe('string');
          expect((value as string).length, `${target} ${key} in ${locale}`).toBeGreaterThan(0);
        }
      }
    }
  });
});
