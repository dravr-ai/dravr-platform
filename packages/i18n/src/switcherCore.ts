// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Platform-neutral body of the language switcher shared by web and React Native
// ABOUTME: Restores a remembered preference on mount, then moves chrome and server locale together

import { useCallback, useEffect, useState } from 'react';
import { DEFAULT_LANGUAGE, isSupportedLanguage, type SupportedLanguage } from './config';
import { persistLocaleToServer } from './localeSync';
import { useTranslation } from './types';

/** Storage key holding the viewer's chosen locale on both platforms. */
export const LANGUAGE_STORAGE_KEY = 'pierre_app_language';

/**
 * Where a chosen locale is remembered between sessions. `localStorage` on the
 * web, `AsyncStorage` on device — both are promise-shaped here so the hook
 * body has one form.
 */
export interface LocaleStorage {
  /** The stored locale tag, or `null` when the viewer has never chosen one. */
  read: (key: string) => Promise<string | null>;
  /** Remember `value` under `key` for the next session. */
  write: (key: string, value: string) => Promise<void>;
}

/** Progress of the write to `users.locale` behind the last language change. */
export type LocaleSyncState = 'idle' | 'saving' | 'error';

/** Options accepted by both language-switcher hooks. */
export interface LanguageSwitcherOptions {
  /** Override the storage key. Defaults to [`LANGUAGE_STORAGE_KEY`]. */
  storageKey?: string;
  /**
   * The locale the server has on record for the signed-in user
   * (`User.locale`). Used only when this device has no stored choice, so a
   * user who picked German on the web does not land back in French on their
   * phone.
   */
  serverLocale?: string;
  /** Notified after a successful change, once chrome and server agree. */
  onLanguageChange?: (language: SupportedLanguage) => void;
}

/** What both language-switcher hooks return. */
export interface LanguageSwitcherResult {
  /** The locale the chrome is currently rendered in. */
  currentLanguage: SupportedLanguage;
  /** Switch chrome and reply language together. Never rejects — read `syncState`. */
  changeLanguage: (language: SupportedLanguage) => Promise<void>;
  /** `'error'` once the chrome moved but `users.locale` did not. */
  syncState: LocaleSyncState;
}

/**
 * The shared switcher body.
 *
 * Chrome language and reply language are one preference wearing two hats, so
 * a change writes both: i18next for what the user reads, `users.locale` for
 * what the coach answers in. The server write is awaited and its failure is
 * reported rather than logged, because a silently-dropped write is exactly the
 * disagreement this hook exists to close.
 */
export function useSwitcherCore(
  storage: LocaleStorage,
  options: LanguageSwitcherOptions,
): LanguageSwitcherResult {
  const { storageKey = LANGUAGE_STORAGE_KEY, serverLocale, onLanguageChange } = options;
  const { i18n, language } = useTranslation();
  const [syncState, setSyncState] = useState<LocaleSyncState>('idle');

  // Deliberately not guarded by a "ran once" ref. StrictMode mounts, tears
  // down and remounts every effect in development: a ref guard would let the
  // teardown cancel the only restore attempt and leave the second mount with
  // no stored preference applied at all. Re-running is safe instead, because
  // `changeLanguage` writes storage before anything can re-read it — a repeat
  // pass finds the viewer's own choice and re-applies the same value.
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      const stored = await storage.read(storageKey).catch(() => null);
      if (cancelled) {
        return;
      }
      const preferred = isSupportedLanguage(stored)
        ? stored
        : isSupportedLanguage(serverLocale)
          ? serverLocale
          : null;
      if (preferred !== null && preferred !== i18n.language) {
        await i18n.changeLanguage(preferred);
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [storage, storageKey, serverLocale, i18n]);

  const changeLanguage = useCallback(
    async (next: SupportedLanguage): Promise<void> => {
      setSyncState('saving');
      // Storage first, so the restore effect can never observe a window where
      // i18next has moved on but the remembered preference has not.
      await storage.write(storageKey, next).catch(() => undefined);
      await i18n.changeLanguage(next);
      try {
        await persistLocaleToServer(next);
      } catch {
        setSyncState('error');
        return;
      }
      setSyncState('idle');
      onLanguageChange?.(next);
    },
    [i18n, storage, storageKey, onLanguageChange],
  );

  return {
    currentLanguage: isSupportedLanguage(language) ? language : DEFAULT_LANGUAGE,
    changeLanguage,
    syncState,
  };
}
