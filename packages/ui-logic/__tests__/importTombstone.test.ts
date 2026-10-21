// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Proves both clients read an import-tombstone refusal off a 409 or an OAuth return, and nothing else
// ABOUTME: Pins the provider and email sentences in every shipped locale, each naming the deletion date

import { describe, it, expect } from 'vitest';
import i18next from 'i18next';
import de from '../../i18n/src/locales/de/translation.json';
import en from '../../i18n/src/locales/en/translation.json';
import es from '../../i18n/src/locales/es/translation.json';
import fr from '../../i18n/src/locales/fr/translation.json';
import pt from '../../i18n/src/locales/pt/translation.json';
import {
  describeEmailTombstone,
  describeProviderTombstone,
  importTombstoneFromReturn,
  importTombstoneRefusal,
} from '../src/importTombstone';

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

const DELETED_AT = '2026-10-20T08:00:00+00:00';

describe('reading a tombstone refusal', () => {
  const details = {
    error: 'import_tombstoned',
    tombstone_id: 'tomb-1',
    kind: 'provider_account',
    provider: 'intervals_icu',
    deleted_at: DELETED_AT,
  };

  it('reads the tombstone off a 409', () => {
    expect(importTombstoneRefusal({ response: { status: 409, data: { details } } })).toEqual({
      tombstoneId: 'tomb-1',
      deletedAt: DELETED_AT,
    });
  });

  it('reads nothing off any other refusal', () => {
    expect(
      importTombstoneRefusal({ response: { status: 409, data: { details: { reason: 'already_linked' } } } })
    ).toBeUndefined();
    expect(
      importTombstoneRefusal({ response: { status: 409, data: { details: { ...details, tombstone_id: 7 } } } })
    ).toBeUndefined();
    expect(importTombstoneRefusal(new Error('Network Error'))).toBeUndefined();
    expect(importTombstoneRefusal(null)).toBeUndefined();
  });

  it('reads the tombstone off an OAuth return, and nothing off another failure', () => {
    const tombstoned = new URLSearchParams(
      'provider=strava&success=false&error=import_tombstoned&tombstone_id=tomb-1&deleted_at=2026-10-20T08%3A00%3A00%2B00%3A00'
    );
    expect(importTombstoneFromReturn((name) => tombstoned.get(name))).toEqual({
      tombstoneId: 'tomb-1',
      deletedAt: DELETED_AT,
    });

    const denied = new URLSearchParams('provider=strava&success=false&error=access_denied');
    expect(importTombstoneFromReturn((name) => denied.get(name))).toBeUndefined();

    const noId = new URLSearchParams('provider=strava&success=false&error=import_tombstoned');
    expect(importTombstoneFromReturn((name) => noId.get(name))).toBeUndefined();
  });

  it('reads nothing off a deep link that repeats a parameter', () => {
    // A parsed deep link's params hold an array for a repeated name.
    const params: Record<string, string | string[]> = {
      error: 'import_tombstoned',
      tombstone_id: ['tomb-1', 'tomb-2'],
      deleted_at: DELETED_AT,
    };
    expect(importTombstoneFromReturn((name) => params[name])).toBeUndefined();
  });
});

describe('wording a tombstone', () => {
  const refusal = { tombstoneId: 'tomb-1', deletedAt: DELETED_AT };

  it('names the provider and the deletion date in English', () => {
    const t = translatorFor('en');
    expect(describeProviderTombstone(refusal, 'Strava', { language: 'en', t })).toBe(
      'This Strava account belonged to a Dravr account deleted on Oct 20, 2026. Its data stays deleted until you choose to start fresh with it.'
    );
    expect(describeEmailTombstone({ tombstone_id: 'tomb-1', deleted_at: DELETED_AT }, { language: 'en', t })).toBe(
      'This email belonged to a Dravr account deleted on Oct 20, 2026. Confirm you want to start fresh with it to continue.'
    );
  });

  it.each(Object.keys(CATALOGUES) as (keyof typeof CATALOGUES)[])(
    'words both sentences in %s, naming the provider and the year',
    (lng) => {
      const t = translatorFor(lng);
      const provider = describeProviderTombstone(refusal, 'Strava', { language: lng, t });
      const email = describeEmailTombstone({ tombstone_id: 'tomb-1', deleted_at: DELETED_AT }, { language: lng, t });
      expect(provider).toContain('Strava');
      expect(provider).toContain('2026');
      expect(provider).not.toContain('{{');
      expect(email).toContain('2026');
      expect(email).not.toContain('{{');
    }
  );
});
