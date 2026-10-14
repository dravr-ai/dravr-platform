// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the web's lazy catalogue end to end: a chosen locale's strings are present when the switch lands
// ABOUTME: And the live overlay merges over the loaded file rather than being replaced by it

import { describe, it, expect } from 'vitest';
import type { I18nBundleResult } from '@pierre/shared-types';
import { i18n, initI18n } from '../src/config';
import { lazyCatalogue } from '../src/catalogue/lazy';

describe('lazy catalogue', () => {
  it('switches language only once the locale is loaded, then overlays the live bundle on it', async () => {
    const fetched: string[] = [];
    const fetchBundle = (locale: string): Promise<I18nBundleResult> => {
      fetched.push(locale);
      return Promise.resolve({
        status: 'fresh',
        bundle: { locale, etag: `etag-${locale}`, strings: { 'app.liveOnly': `live ${locale}` } },
      });
    };
    await initI18n({ persistLocale: () => Promise.resolve(), catalogue: lazyCatalogue, fetchBundle });

    expect(i18n.language).toBe('fr');
    expect(i18n.hasResourceBundle('en', 'translation')).toBe(false);

    const english: string[] = [];
    i18n.on('languageChanged', (lng) => {
      // Strings are already in the store when the chrome is told to repaint.
      english.push(`${lng}:${String(i18n.getResourceBundle(lng, 'translation') !== undefined)}`);
    });
    await i18n.changeLanguage('en');

    expect(english).toEqual(['en:true']);
    const loaded = i18n.getResourceBundle('en', 'translation') as Record<string, unknown>;
    expect(Object.keys(loaded).length).toBeGreaterThan(10);
    // The live string landed over the loaded file, and the file's own keys survived it.
    await expect.poll(() => i18n.t('app.liveOnly')).toBe('live en');
    expect(Object.keys(i18n.getResourceBundle('en', 'translation') as object).length).toBe(Object.keys(loaded).length);
    expect(fetched).toEqual(['fr', 'en']);
  });
});
