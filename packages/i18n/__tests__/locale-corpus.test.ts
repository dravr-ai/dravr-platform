// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Guards the platform's wiring of the string catalogue — the five locales, each bound to its own file
// ABOUTME: The catalogue's content (keys, parity, translation) is authored and tested in dravr-contremaitre

import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, it, expect } from 'vitest';
import { SUPPORTED_LANGUAGES, defaultI18nConfig, DEFAULT_LANGUAGE } from '../src/config';

// The files under src/locales are a byte-identical copy of the pinned
// dravr-contremaitre strings/<locale>.json, where every string is authored;
// its strings_catalogue_test owns key parity, blanks and translation, and
// check-contremaitre-sync.sh fails any byte of difference in the copy. What is
// the platform's own is which file each offered locale reads — pinned here.
const LOCALES_DIR = join(dirname(fileURLToPath(import.meta.url)), '../src/locales');

function bundleFor(language: string): unknown {
  const resources = defaultI18nConfig.resources as Record<string, { translation: unknown }>;
  return resources[language].translation;
}

describe('client locale corpus', () => {
  it('offers exactly the five locales the server accepts, French first', () => {
    expect([...SUPPORTED_LANGUAGES]).toEqual(['fr', 'en', 'es', 'de', 'pt']);
    expect(DEFAULT_LANGUAGE).toBe('fr');
    expect(defaultI18nConfig.lng).toBe('fr');
    expect(defaultI18nConfig.fallbackLng).toBe('fr');
  });

  it('binds every offered locale to its own catalogue file', () => {
    for (const language of SUPPORTED_LANGUAGES) {
      // Why this reads a file: a parity check between two artefacts — the
      // catalogue file on disk and the bundle the config binds to that locale.
      // Both sides are read and compared whole; importing the JSON here would
      // compare the config with the same import it makes.
      const file = JSON.parse(readFileSync(join(LOCALES_DIR, language, 'translation.json'), 'utf8'));
      expect({ language, bundle: bundleFor(language) }).toEqual({ language, bundle: file });
    }
    expect(Object.keys(defaultI18nConfig.resources).sort()).toEqual([...SUPPORTED_LANGUAGES].sort());
  });
});
