// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the provider capability-scope vocabulary — every wire slug has a word, in all five locales
// ABOUTME: A slug without one printed as itself ("activities, sleep") under French chrome; this keeps the table closed

import { describe, expect, it } from 'vitest';
import { PROVIDER_SCOPES, PROVIDER_SCOPE_LABEL_KEY, providerScopeLabelKey } from '@pierre/shared-constants';
import de from '../../../../packages/i18n/src/locales/de/translation.json';
import en from '../../../../packages/i18n/src/locales/en/translation.json';
import es from '../../../../packages/i18n/src/locales/es/translation.json';
import fr from '../../../../packages/i18n/src/locales/fr/translation.json';
import pt from '../../../../packages/i18n/src/locales/pt/translation.json';

/**
 * The four capability slugs `GET /api/oauth/providers` builds, in the order
 * `crates/pierre-routes-auth/src/oauth.rs` pushes them.
 */
const SERVER_SCOPES = ['activities', 'sleep', 'recovery', 'health'];

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

describe('provider capability scopes', () => {
  it('lists exactly the scopes the server can send', () => {
    expect([...PROVIDER_SCOPES]).toEqual(SERVER_SCOPES);
  });

  it('names a corpus key for every scope, and all five locales carry it', () => {
    for (const scope of PROVIDER_SCOPES) {
      const key = PROVIDER_SCOPE_LABEL_KEY[scope];
      expect(key, scope).toBe(`providers.scope.${scope}`);
      for (const [locale, bundle] of Object.entries(BUNDLES)) {
        expect(typeof leaf(bundle, key), `${locale} ${key}`).toBe('string');
      }
    }
  });

  it('translates the words rather than repeating the slug', () => {
    // The defect was the slug itself showing under translated chrome, so the
    // guard is that a non-English locale says something else.
    for (const locale of ['fr', 'es', 'de', 'pt']) {
      for (const scope of PROVIDER_SCOPES) {
        expect(leaf(BUNDLES[locale], PROVIDER_SCOPE_LABEL_KEY[scope]), `${locale} ${scope}`).not.toBe(
          scope,
        );
      }
    }
    expect(leaf(BUNDLES.fr, PROVIDER_SCOPE_LABEL_KEY.sleep)).toBe('sommeil');
    expect(leaf(BUNDLES.fr, PROVIDER_SCOPE_LABEL_KEY.recovery)).toBe('récupération');
  });

  it('has no key for a slug it does not know, so the caller prints it verbatim', () => {
    expect(providerScopeLabelKey('zzz-unknown')).toBeNull();
    expect(providerScopeLabelKey('sleep')).toBe('providers.scope.sleep');
  });
});
