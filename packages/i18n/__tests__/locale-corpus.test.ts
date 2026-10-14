// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Guards the platform's wiring of the string catalogue — the five locales, each bound to its own file
// ABOUTME: The catalogue's content (keys, parity, translation) is authored and tested in dravr-contremaitre

import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, it, expect } from 'vitest';
import { SUPPORTED_LANGUAGES, defaultI18nConfig, DEFAULT_LANGUAGE } from '../src/config';
import { EMBEDDED_RESOURCES } from '../src/catalogue/embedded';
import { lazyCatalogue } from '../src/catalogue/lazy';

// The files under src/locales are a byte-identical copy of the pinned
// dravr-contremaitre strings/<locale>.json, where every string is authored;
// its strings_catalogue_test owns key parity, blanks and translation, and
// check-contremaitre-sync.sh fails any byte of difference in the copy. What is
// the platform's own is which file each offered locale reads — pinned here.
const LOCALES_DIR = join(dirname(fileURLToPath(import.meta.url)), '../src/locales');

function fileFor(language: string): unknown {
  return JSON.parse(readFileSync(join(LOCALES_DIR, language, 'translation.json'), 'utf8'));
}

describe('client locale corpus', () => {
  it('offers exactly the five locales the server accepts, French first', () => {
    expect([...SUPPORTED_LANGUAGES]).toEqual(['fr', 'en', 'es', 'de', 'pt']);
    expect(DEFAULT_LANGUAGE).toBe('fr');
    expect(defaultI18nConfig.lng).toBe('fr');
    expect(defaultI18nConfig.fallbackLng).toBe('fr');
  });

  // Why these read a file: a parity check between two artefacts — the
  // catalogue file on disk and the bundle a catalogue binds to that locale.
  // Both sides are read and compared whole; importing the JSON here would
  // compare the catalogue with the same import it makes.
  it('binds every offered locale to its own catalogue file in the embedded catalogue', () => {
    for (const language of SUPPORTED_LANGUAGES) {
      expect({ language, bundle: EMBEDDED_RESOURCES[language].translation }).toEqual({ language, bundle: fileFor(language) });
    }
    expect(Object.keys(EMBEDDED_RESOURCES).sort()).toEqual([...SUPPORTED_LANGUAGES].sort());
  });

  it('embeds only the default locale in the lazy catalogue and loads every other from its own file', async () => {
    expect(Object.keys(lazyCatalogue.resources)).toEqual([DEFAULT_LANGUAGE]);
    expect(lazyCatalogue.resources[DEFAULT_LANGUAGE].translation).toEqual(fileFor(DEFAULT_LANGUAGE));
    const load = lazyCatalogue.load;
    expect(load).toBeDefined();
    for (const language of SUPPORTED_LANGUAGES) {
      expect({ language, bundle: await load?.(language) }).toEqual({ language, bundle: fileFor(language) });
    }
  });
});
