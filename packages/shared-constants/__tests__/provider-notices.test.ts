// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Proves each provider notice both clients show reads as keys the en catalogue carries, in five locales
// ABOUTME: And that a notice is asked for only when the card says so and a notice exists for that provider

import { describe, it, expect } from 'vitest';
import en from '../../i18n/src/locales/en/translation.json';
import fr from '../../i18n/src/locales/fr/translation.json';
import es from '../../i18n/src/locales/es/translation.json';
import de from '../../i18n/src/locales/de/translation.json';
import pt from '../../i18n/src/locales/pt/translation.json';
import { PROVIDER_NOTICES, noticeRequired, syncAuthorizationOwed } from '../src/providers';

function lookup(catalogue: unknown, key: string): unknown {
  return key.split('.').reduce<unknown>(
    (node, part) => (node && typeof node === 'object' ? (node as Record<string, unknown>)[part] : undefined),
    catalogue,
  );
}

describe('provider notices', () => {
  it('names the three providers the server asks a notice for', () => {
    expect(Object.keys(PROVIDER_NOTICES).sort()).toEqual(['sciotte_coros', 'sciotte_trainingpeaks', 'whoop']);
  });

  it('reads every notice as text in all five locales', () => {
    for (const [provider, notice] of Object.entries(PROVIDER_NOTICES)) {
      for (const [locale, catalogue] of Object.entries({ en, fr, es, de, pt })) {
        for (const key of [notice.titleKey, notice.bodyKey, notice.consentKey]) {
          const text = lookup(catalogue, key);
          expect(typeof text, `${provider} ${locale} ${key}`).toBe('string');
          expect((text as string).length, `${provider} ${locale} ${key}`).toBeGreaterThan(10);
        }
      }
    }
  });

  it("states WHOOP's owner authorization and the EU basis", () => {
    expect(lookup(en, PROVIDER_NOTICES.whoop.consentKey)).toBe(
      'As the owner of this data, I authorize Dravr to store my WHOOP measurements, compute training metrics from them, and provide them to my AI agent. I can revoke this at any time.',
    );
    expect(lookup(en, PROVIDER_NOTICES.whoop.bodyKey)).toContain('GDPR Article 20 and the EU Data Act');
  });

  it('asks only when the card says so and a notice exists', () => {
    expect(noticeRequired('whoop', true)).toBe(true);
    expect(noticeRequired('whoop', false)).toBe(false);
    expect(noticeRequired('whoop', undefined)).toBe(false);
    expect(noticeRequired('strava', true)).toBe(false);
  });

  it('flags a connected WHOOP that owes its notice, and nothing else', () => {
    expect(syncAuthorizationOwed('whoop', true, true)).toBe(true);
    expect(syncAuthorizationOwed('whoop', false, true)).toBe(false);
    expect(syncAuthorizationOwed('whoop', true, false)).toBe(false);
    // TrainingPeaks' notice guards the login, not what sync keeps.
    expect(syncAuthorizationOwed('sciotte_trainingpeaks', true, true)).toBe(false);
  });

  it('names the authorization a connected WHOOP asks for', () => {
    expect(lookup(en, 'providers.authorizeToKeepSyncing')).toBe('Authorize {{provider}} to keep syncing');
    for (const catalogue of [fr, es, de, pt]) {
      expect(lookup(catalogue, 'providers.authorizeToKeepSyncing')).toContain('{{provider}}');
      expect(typeof lookup(catalogue, 'providers.authorizeAction')).toBe('string');
    }
  });
});
