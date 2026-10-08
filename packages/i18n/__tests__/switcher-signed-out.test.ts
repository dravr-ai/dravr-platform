// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Covers the signed-out restore — the login screen speaks the last account's language
// ABOUTME: Asserts the remembered account language never outranks a device choice or the next account

/** @vitest-environment jsdom */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';

import { i18n, initI18n, type SupportedLanguage } from '../src/config';
import { registerLocalePersister } from '../src/localeSync';
import {
  ACCOUNT_LANGUAGE_STORAGE_KEY,
  LANGUAGE_STORAGE_KEY,
  useSwitcherCore,
  type LocaleStorage,
} from '../src/switcherCore';
import { useLanguageSwitcher } from '../src/useLanguageSwitcher';

/** In-memory storage that outlives a mounted hook, as `localStorage` outlives a reload. */
function memoryStorage(initial: Record<string, string> = {}): LocaleStorage & { values: Map<string, string> } {
  const values = new Map(Object.entries(initial));
  return {
    values,
    read: (key) => Promise.resolve(values.get(key) ?? null),
    write: (key, value) => {
      values.set(key, value);
      return Promise.resolve();
    },
  };
}

const persist = vi.fn<(language: SupportedLanguage) => Promise<void>>();

beforeEach(async () => {
  persist.mockReset();
  persist.mockResolvedValue(undefined);
  await initI18n({ persistLocale: persist, config: { lng: 'fr' } });
  registerLocalePersister(persist);
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe('signed-out restore — the login screen keeps the account language', () => {
  it('opens the signed-out app in the language the last account used here', async () => {
    const storage = memoryStorage();
    const signedIn = renderHook(() => useSwitcherCore(storage, { serverLocale: 'en' }));
    await waitFor(() => {
      expect(signedIn.result.current.currentLanguage).toBe('en');
    });
    signedIn.unmount();

    // A reload after the session ended: i18next boots on the default again and
    // nobody is signed in, so only this device can say which language to use.
    await i18n.changeLanguage('fr');
    const signedOut = renderHook(() => useSwitcherCore(storage, {}));

    await waitFor(() => {
      expect(signedOut.result.current.currentLanguage).toBe('en');
    });
    expect(persist).not.toHaveBeenCalled();
  });

  it('keeps the account language across a reload through the web hook and localStorage', async () => {
    // The hook the web app mounts, over jsdom's real `localStorage`: the
    // in-memory storage stands in for it everywhere else in this file.
    localStorage.clear();
    const signedIn = renderHook(() => useLanguageSwitcher({ serverLocale: 'en' }));
    await waitFor(() => {
      expect(localStorage.getItem(ACCOUNT_LANGUAGE_STORAGE_KEY)).toBe('en');
    });
    signedIn.unmount();

    await i18n.changeLanguage('fr');
    const signedOut = renderHook(() => useLanguageSwitcher());

    await waitFor(() => {
      expect(signedOut.result.current.currentLanguage).toBe('en');
    });
    expect(localStorage.getItem(LANGUAGE_STORAGE_KEY)).toBeNull();
    expect(persist).not.toHaveBeenCalled();
  });

  it('lets a language chosen on this device outrank the remembered account', async () => {
    const storage = memoryStorage({
      [LANGUAGE_STORAGE_KEY]: 'de',
      [ACCOUNT_LANGUAGE_STORAGE_KEY]: 'en',
    });
    const { result } = renderHook(() => useSwitcherCore(storage, {}));

    await waitFor(() => {
      expect(result.current.currentLanguage).toBe('de');
    });
  });

  it('never carries the remembered account language into the next account', async () => {
    const storage = memoryStorage({ [ACCOUNT_LANGUAGE_STORAGE_KEY]: 'en' });
    const { result } = renderHook(() => useSwitcherCore(storage, { serverLocale: 'es' }));

    await waitFor(() => {
      expect(result.current.currentLanguage).toBe('es');
    });
    await waitFor(() => {
      expect(storage.values.get(ACCOUNT_LANGUAGE_STORAGE_KEY)).toBe('es');
    });
    // Remembering an account is not a device choice: nothing is written back.
    expect(persist).not.toHaveBeenCalled();
    expect(storage.values.has(LANGUAGE_STORAGE_KEY)).toBe(false);
  });

  it('stays on the default for a first visit with nothing remembered', async () => {
    const storage = memoryStorage();
    const read = vi.spyOn(storage, 'read');
    const { result } = renderHook(() => useSwitcherCore(storage, {}));

    // Both keys read means the restore pass has run to its decision.
    await waitFor(() => {
      expect(read).toHaveBeenCalledWith(ACCOUNT_LANGUAGE_STORAGE_KEY);
    });
    await act(async () => {
      await Promise.resolve();
    });
    expect(result.current.currentLanguage).toBe('fr');
    expect(storage.values.size).toBe(0);
  });
});
