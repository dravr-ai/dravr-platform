// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Platform-neutral body of the language switcher shared by web and React Native
// ABOUTME: Restores a remembered preference on mount, then moves chrome and server locale together

import { useCallback, useEffect, useRef, useState } from 'react';
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

/** The restore write-back currently in flight, shared by every mounted switcher. */
let pendingWriteBack: { language: SupportedLanguage; done: Promise<void> } | null = null;

/**
 * Push a restored device locale to `users.locale`, joining an identical write
 * already in flight instead of issuing a second one.
 *
 * The web app mounts the switcher hook twice while Settings is open (app root
 * and the picker), and StrictMode re-runs effects; both would otherwise send
 * the same `PUT` side by side. A write for a different language is never
 * joined — the latest choice always reaches the server.
 */
function writeBackRestoredLocale(language: SupportedLanguage): Promise<void> {
  if (pendingWriteBack !== null && pendingWriteBack.language === language) {
    return pendingWriteBack.done;
  }
  const entry = { language, done: persistLocaleToServer(language) };
  pendingWriteBack = entry;
  const settle = (): void => {
    if (pendingWriteBack === entry) {
      pendingWriteBack = null;
    }
  };
  entry.done.then(settle, settle);
  return entry.done;
}

/** Options accepted by both language-switcher hooks. */
export interface LanguageSwitcherOptions {
  /** Override the storage key. Defaults to [`LANGUAGE_STORAGE_KEY`]. */
  storageKey?: string;
  /**
   * The locale the server has on record for the signed-in user
   * (`User.locale`), `undefined` while nobody is signed in. Adopted when this
   * device has no stored choice, so a user who picked German on the web does
   * not land back in French on their phone; overwritten with the stored choice
   * when the two disagree, so the agent answers in the language on screen.
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
 * what the agent answers in. The server write is awaited and its failure is
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
  // Which write owns `syncState`: the restore pass that started a write-back,
  // a user change, or nothing once the hook is gone. A restore pass that a
  // dependency change superseded still reports its write unless a newer write
  // took over — otherwise a `serverLocale` that catches up mid-write (a user
  // refetch landing before the PUT's response) would leave `'saving'` behind,
  // and the pickers stay disabled on it.
  const syncOwner = useRef<object | null>(null);

  // react-i18next hands out a fresh `i18n` wrapper on every language change,
  // so the restore effect reads it through a ref instead of depending on it.
  // As a dependency it re-ran the restore after each switch, and that pass
  // found the new choice disagreeing with a `serverLocale` not yet refetched
  // and sent a second `PUT` beside the one `changeLanguage` already made.
  const i18nRef = useRef(i18n);
  useEffect(() => {
    i18nRef.current = i18n;
  }, [i18n]);

  useEffect(
    () => () => {
      syncOwner.current = null;
    },
    [],
  );

  // Deliberately not guarded by a "ran once" ref. StrictMode mounts, tears
  // down and remounts every effect in development: a ref guard would let the
  // teardown cancel the only restore attempt and leave the second mount with
  // no stored preference applied at all. Re-running is safe instead, because
  // `changeLanguage` writes storage before anything can re-read it — a repeat
  // pass finds the viewer's own choice and re-applies the same value, and a
  // repeat write-back joins the one already in flight.
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
      const instance = i18nRef.current;
      if (preferred !== null && preferred !== instance.language) {
        await instance.changeLanguage(preferred);
      }
      // A choice made on this device wins over the account, so the account
      // must hear about it: without this write the chrome renders the stored
      // language while the agent keeps answering in `users.locale`. Only for a
      // signed-in user (a supported `serverLocale`) whose record disagrees.
      if (
        cancelled ||
        !isSupportedLanguage(stored) ||
        !isSupportedLanguage(serverLocale) ||
        stored === serverLocale
      ) {
        return;
      }
      const owner = {};
      syncOwner.current = owner;
      setSyncState('saving');
      let outcome: LocaleSyncState = 'idle';
      try {
        await writeBackRestoredLocale(stored);
      } catch {
        outcome = 'error';
      }
      if (syncOwner.current === owner) {
        setSyncState(outcome);
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [storage, storageKey, serverLocale]);

  const changeLanguage = useCallback(
    async (next: SupportedLanguage): Promise<void> => {
      // The user's choice supersedes a restore write-back still in flight.
      syncOwner.current = {};
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
