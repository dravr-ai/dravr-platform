// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: All five locale catalogues compiled in — the phone's catalogue, and the one tests read
// ABOUTME: The phone has no network on first launch to fetch a language from, so it carries every one

import deTranslation from '../locales/de/translation.json';
import enTranslation from '../locales/en/translation.json';
import esTranslation from '../locales/es/translation.json';
import frTranslation from '../locales/fr/translation.json';
import ptTranslation from '../locales/pt/translation.json';
import type { Catalogue, CatalogueResources } from './types';

/** Every supported locale's resources, keyed by locale tag. */
export const EMBEDDED_RESOURCES: CatalogueResources = {
  fr: { translation: frTranslation },
  en: { translation: enTranslation },
  es: { translation: esTranslation },
  de: { translation: deTranslation },
  pt: { translation: ptTranslation },
};

/** A catalogue with nothing left to load. */
export const embeddedCatalogue: Catalogue = { resources: EMBEDDED_RESOURCES };
