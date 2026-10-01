// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Every account role names a key every locale carries, and the French labels are French
// ABOUTME: Red if a role is added without a label, or a label key is missing from any catalogue

import { describe, expect, it } from 'vitest';
import de from '../../i18n/src/locales/de/translation.json';
import en from '../../i18n/src/locales/en/translation.json';
import es from '../../i18n/src/locales/es/translation.json';
import fr from '../../i18n/src/locales/fr/translation.json';
import pt from '../../i18n/src/locales/pt/translation.json';
import { ACCOUNT_ROLE_LABEL_KEY } from '../src/roles';

const LOCALES = { de, en, es, fr, pt } as const;

function lookup(catalogue: unknown, key: string): unknown {
  return key.split('.').reduce<unknown>(
    (node, part) => (node && typeof node === 'object' ? (node as Record<string, unknown>)[part] : undefined),
    catalogue,
  );
}

describe('ACCOUNT_ROLE_LABEL_KEY', () => {
  it('names a key every locale translates', () => {
    for (const [locale, catalogue] of Object.entries(LOCALES)) {
      for (const key of Object.values(ACCOUNT_ROLE_LABEL_KEY)) {
        expect(typeof lookup(catalogue, key), `${locale}:${key}`).toBe('string');
      }
    }
  });

  it('reads the plain role in French, not the wire value', () => {
    expect(lookup(fr, ACCOUNT_ROLE_LABEL_KEY.user)).toBe('Utilisateur');
  });
});
