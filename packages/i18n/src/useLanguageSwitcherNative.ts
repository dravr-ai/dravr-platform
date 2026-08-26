// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: React Native language switcher hook backed by AsyncStorage
// ABOUTME: Changes chrome language and the coach's reply language in one call

import AsyncStorage from '@react-native-async-storage/async-storage';
import { useSwitcherCore, type LanguageSwitcherOptions, type LanguageSwitcherResult } from './switcherCore';

// Module-level so the identity is stable across renders — the restore effect
// takes the storage object as a dependency.
const nativeLocaleStorage = {
  read: (key: string): Promise<string | null> => AsyncStorage.getItem(key),
  write: (key: string, value: string): Promise<void> => AsyncStorage.setItem(key, value),
};

/**
 * Manage the mobile app's language.
 *
 * Persists to `AsyncStorage` for the next launch and to `users.locale` so the
 * coach answers in the same language; `syncState` reports the server half.
 */
export function useLanguageSwitcherNative(
  options: LanguageSwitcherOptions = {},
): LanguageSwitcherResult {
  return useSwitcherCore(nativeLocaleStorage, options);
}

export type { LanguageSwitcherOptions, LanguageSwitcherResult };
