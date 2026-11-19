// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The express-consent words shown where an MCP token or an API key is created name distinct keys
// ABOUTME: Every one resolves in all five catalogues, so neither client prints a raw key beside its create action

import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import { CREDENTIAL_CONSENT_KEYS } from '../src/api-keys';

const LOCALES = ['en', 'fr', 'es', 'de', 'pt'] as const;

/** The value at a dotted key of a nested catalogue, or `undefined`. */
function lookup(catalogue: unknown, key: string): unknown {
  return key
    .split('.')
    .reduce<unknown>(
      (node, part) => (node && typeof node === 'object' ? (node as Record<string, unknown>)[part] : undefined),
      catalogue,
    );
}

describe('CREDENTIAL_CONSENT_KEYS', () => {
  // Each key's words are pinned where they render (UserSettings, ApiKeysSettings,
  // TokensScreen, ApiKeysScreen); here only that a token and a key read apart.
  it('names one consent for a token and another for a key', () => {
    expect(CREDENTIAL_CONSENT_KEYS.mcpToken, 'bug: a token and a key share one consent').not.toBe(
      CREDENTIAL_CONSENT_KEYS.apiKey,
    );
  });

  it.each(LOCALES)('resolves in the %s catalogue', (locale) => {
    // Authored in dravr-contremaitre and copied here by the bump lane.
    const path = resolve(__dirname, '../../i18n/src/locales', locale, 'translation.json');
    const catalogue: unknown = JSON.parse(readFileSync(path, 'utf8'));
    const missing = Object.values(CREDENTIAL_CONSENT_KEYS).filter((key) => typeof lookup(catalogue, key) !== 'string');
    expect(missing).toEqual([]);
  });
});
