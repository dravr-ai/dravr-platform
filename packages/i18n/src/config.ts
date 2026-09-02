// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: i18next configuration for unified web and mobile internationalization
// ABOUTME: Declares the five locales the backend answers in and makes French the default

import i18n from 'i18next';
import { initReactI18next } from 'react-i18next';
import deTranslation from './locales/de/translation.json';
import enTranslation from './locales/en/translation.json';
import esTranslation from './locales/es/translation.json';
import frTranslation from './locales/fr/translation.json';
import ptTranslation from './locales/pt/translation.json';
import { registerLocalePersister, type LocalePersister } from './localeSync';
import { installLiveOverlay, type BundleFetcher } from './liveBundle';

/**
 * The locales both surfaces offer, in menu order.
 *
 * This list is the client half of one contract: the server accepts exactly
 * `["fr", "en", "es", "de", "pt"]` on `PUT /api/user/locale` (`SUPPORTED_LOCALES`
 * in `pierre-routes-auth`), and every messaging string ships in all five
 * (`entries == keys * 5`, CI-enforced). Offering a sixth here would let a user
 * pick a language the coach cannot answer in.
 */
export const SUPPORTED_LANGUAGES = ['fr', 'en', 'es', 'de', 'pt'] as const;

/** One of the five locales the platform speaks. */
export type SupportedLanguage = typeof SUPPORTED_LANGUAGES[number];

/** Endonym for each supported locale — a language menu names itself in its own language. */
export const LANGUAGE_NAMES: Record<SupportedLanguage, string> = {
  fr: 'Français',
  en: 'English',
  es: 'Español',
  de: 'Deutsch',
  pt: 'Português',
};

/**
 * The locale a client falls back to before it knows anything about the user.
 *
 * French, matching `DEFAULT_LOCALE` in `pierre-contremaitre::messaging_strings`.
 * The user base is majority francophone and the server already answers them in
 * French; a client defaulting to English put the chrome and the coach in two
 * different languages on first paint.
 */
export const DEFAULT_LANGUAGE: SupportedLanguage = 'fr';

/** Narrow an arbitrary locale tag (a server value, a stored preference) to a supported one. */
export function isSupportedLanguage(value: unknown): value is SupportedLanguage {
  return typeof value === 'string' && (SUPPORTED_LANGUAGES as readonly string[]).includes(value);
}

/** Resource bundles keyed by locale, one `translation` namespace each. */
export const defaultI18nConfig = {
  resources: {
    fr: { translation: frTranslation },
    en: { translation: enTranslation },
    es: { translation: esTranslation },
    de: { translation: deTranslation },
    pt: { translation: ptTranslation },
  },
  lng: DEFAULT_LANGUAGE,
  fallbackLng: DEFAULT_LANGUAGE,
  // Resources are compiled in, so there is nothing to wait for: initialize on
  // the spot rather than deferring to a timer. Without this the first paint can
  // render raw keys, and a test process is left holding i18next's timeout. The
  // live catalogue arrives later, as an overlay (see `installLiveOverlay`).
  initImmediate: false,
  interpolation: {
    escapeValue: false, // React already escapes values
  },
  react: {
    useSuspense: false,
    // Repaint mounted chrome when a resource bundle is added — the live
    // catalogue overlay lands through `addResourceBundle`, which emits the
    // store's `added` event and, without this, nothing until navigation.
    bindI18nStore: 'added',
  },
};

/** Arguments to [`initI18n`]. */
export interface I18nInitOptions {
  /**
   * Writes the chosen locale back to `users.locale` so the coach answers in
   * the same language the chrome is rendered in.
   *
   * Required, not optional: the two locale systems disagreed for as long as
   * the client had a way to change its own language without telling the
   * server, and an optional hook would let the next app re-open that gap by
   * omission.
   */
  persistLocale: LocalePersister;
  /**
   * Reads the live catalogue from the server (`GET /api/i18n/{locale}`).
   *
   * Optional: an app passes its api-client's `i18n.bundle`, so a string fixed
   * upstream reaches it on the next open without a deploy; a test process or
   * a tool leaves it out and renders the embedded copy alone. The overlay is
   * fail-open — a fetch that fails changes nothing on screen.
   */
  fetchBundle?: BundleFetcher;
  /** i18next overrides — resource bundles, interpolation, initial `lng`. */
  config?: Partial<typeof defaultI18nConfig>;
}

/**
 * Initialize i18next and wire the server-side half of the locale contract.
 *
 * Call once per app, before the first render.
 */
export function initI18n(options: I18nInitOptions) {
  registerLocalePersister(options.persistLocale);
  const ready = i18n.use(initReactI18next).init({
    ...defaultI18nConfig,
    ...options.config,
  });
  if (options.fetchBundle !== undefined) {
    installLiveOverlay(i18n, options.fetchBundle);
  }
  return ready;
}

export { i18n };
