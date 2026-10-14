// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The web's catalogue: French compiled in, every other locale its own chunk fetched when chosen
// ABOUTME: Embedding all five would add ~1 MB of strings to the bundle each visitor downloads before first paint

import frTranslation from '../locales/fr/translation.json';
import type { Catalogue } from './types';

/**
 * One literal `import()` per locale, so the bundler emits each file as its
 * own chunk. A template path would leave the bundler to guess the set.
 */
async function loadLocale(locale: string): Promise<Record<string, unknown>> {
  switch (locale) {
    case 'fr':
      return frTranslation;
    case 'en':
      return (await import('../locales/en/translation.json')).default;
    case 'es':
      return (await import('../locales/es/translation.json')).default;
    case 'de':
      return (await import('../locales/de/translation.json')).default;
    case 'pt':
      return (await import('../locales/pt/translation.json')).default;
    default:
      throw new Error(`no catalogue for locale ${locale}`);
  }
}

/**
 * French — the default and the fallback — is embedded so first paint never
 * waits on a fetch; another locale loads before i18next switches to it.
 */
export const lazyCatalogue: Catalogue = {
  resources: { fr: { translation: frTranslation } },
  load: loadLocale,
};
