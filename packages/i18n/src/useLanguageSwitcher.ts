// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Web language switcher hook backed by localStorage
// ABOUTME: Changes chrome language and the coach's reply language in one call

import { useSwitcherCore, type LanguageSwitcherOptions, type LanguageSwitcherResult } from './switcherCore';

// Module-level so the identity is stable across renders — the restore effect
// takes the storage object as a dependency.
const webLocaleStorage = {
  read: (key: string): Promise<string | null> => Promise.resolve(localStorage.getItem(key)),
  write: (key: string, value: string): Promise<void> => {
    localStorage.setItem(key, value);
    return Promise.resolve();
  },
};

/**
 * Manage the web app's language.
 *
 * Persists to `localStorage` for the next visit and to `users.locale` so the
 * coach answers in the same language; `syncState` reports the server half.
 */
export function useLanguageSwitcher(options: LanguageSwitcherOptions = {}): LanguageSwitcherResult {
  return useSwitcherCore(webLocaleStorage, options);
}

export type { LanguageSwitcherOptions, LanguageSwitcherResult };
