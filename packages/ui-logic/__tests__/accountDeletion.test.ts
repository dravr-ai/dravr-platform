// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Proves a refused self-serve account deletion reads as a catalogue sentence in every shipped locale
// ABOUTME: Keyed on the server's `error` code, blockers kept and worded, the confirmation matched as the server does

import { describe, it, expect } from 'vitest';
import i18next from 'i18next';
import de from '../../i18n/src/locales/de/translation.json';
import en from '../../i18n/src/locales/en/translation.json';
import es from '../../i18n/src/locales/es/translation.json';
import fr from '../../i18n/src/locales/fr/translation.json';
import pt from '../../i18n/src/locales/pt/translation.json';
import {
  describeAccountDeletionBlocker,
  describeAccountDeletionFailure,
  emailConfirms,
} from '../src/accountDeletion';

const CATALOGUES = { de, en, es, fr, pt } as const;

/** A strict translator over one real catalogue: a missing key throws. */
function translatorFor(lng: keyof typeof CATALOGUES) {
  const instance = i18next.createInstance();
  void instance.init({
    lng,
    resources: { [lng]: { translation: CATALOGUES[lng] } },
    initAsync: false,
    showSupportNotice: false,
    interpolation: { escapeValue: false },
  });
  return (key: string, params?: Record<string, string | number>): string => {
    if (!instance.exists(key)) throw new Error(`missing ${lng} catalogue key ${key}`);
    return instance.t(key, params ?? {});
  };
}

function refused(status: number, data: Record<string, unknown>) {
  return { response: { status, data } };
}

describe('emailConfirms', () => {
  it('matches the account email the way the server normalizes it', () => {
    expect(emailConfirms('  Athlete@Example.COM ', 'athlete@example.com')).toBe(true);
    expect(emailConfirms('athlete@example.co', 'athlete@example.com')).toBe(false);
    expect(emailConfirms('   ', '')).toBe(false);
  });
});

describe('describeAccountDeletionFailure', () => {
  for (const lng of Object.keys(CATALOGUES) as (keyof typeof CATALOGUES)[]) {
    const t = translatorFor(lng);

    it(`words every refusal code from the ${lng} catalogue`, () => {
      const mismatch = describeAccountDeletionFailure(refused(400, { error: 'email_mismatch' }), t);
      const required = describeAccountDeletionFailure(refused(403, { error: 'password_required' }), t);
      const incorrect = describeAccountDeletionFailure(refused(403, { error: 'password_incorrect' }), t);
      const tooMany = describeAccountDeletionFailure(refused(429, { error: 'too_many_attempts' }), t);
      const failed = describeAccountDeletionFailure(new Error('network'), t);
      const messages = [mismatch, required, incorrect, tooMany, failed].map((f) => f.message);
      expect(new Set(messages).size).toBe(5);
      expect(messages.every((m) => !m.startsWith('accountDeletion.'))).toBe(true);
      expect(failed.blockers).toEqual([]);
    });

    it(`keeps the blockers of a 409 and words each from the ${lng} catalogue`, () => {
      const failure = describeAccountDeletionFailure(
        refused(409, {
          error: 'blocked',
          blockers: [
            { kind: 'owns_coaching_group', detail: 'Track Tuesdays' },
            { kind: 'not_a_kind', detail: 'dropped' },
          ],
        }),
        t,
      );
      expect(failure.blockers).toEqual([{ kind: 'owns_coaching_group', detail: 'Track Tuesdays' }]);
      expect(describeAccountDeletionBlocker(failure.blockers[0], t)).toContain('Track Tuesdays');
    });
  }
});
