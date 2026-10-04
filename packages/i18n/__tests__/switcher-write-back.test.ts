// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Covers the restore write-back — a locale chosen on this device reaches users.locale
// ABOUTME: Asserts it writes only for a signed-in user whose record disagrees, and reports a failed write

/** @vitest-environment jsdom */

import { createElement, StrictMode, type ReactNode } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';

import { initI18n, type SupportedLanguage } from '../src/config';
import { registerLocalePersister } from '../src/localeSync';
import { LANGUAGE_STORAGE_KEY, useSwitcherCore, type LocaleStorage } from '../src/switcherCore';

/** In-memory storage holding `stored` under the switcher's key, or nothing at all. */
function storageWith(stored: string | null): LocaleStorage {
  const values = new Map<string, string>();
  if (stored !== null) {
    values.set(LANGUAGE_STORAGE_KEY, stored);
  }
  return {
    read: (key) => Promise.resolve(values.get(key) ?? null),
    write: (key, value) => {
      values.set(key, value);
      return Promise.resolve();
    },
  };
}

const strictWrapper = ({ children }: { children: ReactNode }) => createElement(StrictMode, null, children);

const persist = vi.fn<(language: SupportedLanguage) => Promise<void>>();

beforeEach(async () => {
  persist.mockReset();
  persist.mockResolvedValue(undefined);
  await initI18n({ persistLocale: persist, config: { lng: 'fr' } });
  registerLocalePersister(persist);
});

// Vitest runs without globals here, so Testing Library cannot register its
// own unmount; a hook left mounted would keep reacting to the next test's
// i18next instance.
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe('restore write-back — the device choice reaches users.locale', () => {
  it('writes the stored locale when the signed-in account disagrees', async () => {
    const storage = storageWith('es');
    const { result } = renderHook(() => useSwitcherCore(storage, { serverLocale: 'de' }), {
      wrapper: strictWrapper,
    });

    await waitFor(() => {
      expect(persist).toHaveBeenCalledWith('es');
    });
    await waitFor(() => {
      expect(result.current.syncState).toBe('idle');
    });
    // StrictMode re-runs the effect; the repeat must not send a second PUT.
    expect(persist).toHaveBeenCalledTimes(1);
    expect(result.current.currentLanguage).toBe('es');
  });

  it('joins one write when two switchers restore the same locale at once', async () => {
    const storage = storageWith('pt');
    renderHook(() => {
      useSwitcherCore(storage, { serverLocale: 'en' });
      useSwitcherCore(storage, { serverLocale: 'en' });
    });

    await waitFor(() => {
      expect(persist).toHaveBeenCalledWith('pt');
    });
    expect(persist).toHaveBeenCalledTimes(1);
  });

  it('does not write when the stored locale already matches the account', async () => {
    const storage = storageWith('de');
    const { result } = renderHook(() => useSwitcherCore(storage, { serverLocale: 'de' }));

    await waitFor(() => {
      expect(result.current.currentLanguage).toBe('de');
    });
    expect(persist).not.toHaveBeenCalled();
    expect(result.current.syncState).toBe('idle');
  });

  it('does not write when this device has no stored choice', async () => {
    const storage = storageWith(null);
    const { result } = renderHook(() => useSwitcherCore(storage, { serverLocale: 'en' }));

    await waitFor(() => {
      expect(result.current.currentLanguage).toBe('en');
    });
    expect(persist).not.toHaveBeenCalled();
  });

  it('does not write when nobody is signed in', async () => {
    const storage = storageWith('es');
    const { result } = renderHook(() => useSwitcherCore(storage, {}));

    await waitFor(() => {
      expect(result.current.currentLanguage).toBe('es');
    });
    expect(persist).not.toHaveBeenCalled();
    expect(result.current.syncState).toBe('idle');
  });

  it('reports a failed write as a sync error', async () => {
    persist.mockRejectedValue(new Error('offline'));
    const storage = storageWith('es');
    const { result } = renderHook(() => useSwitcherCore(storage, { serverLocale: 'fr' }));

    await waitFor(() => {
      expect(result.current.syncState).toBe('error');
    });
    expect(persist).toHaveBeenCalledWith('es');
    // The chrome still follows the device choice; only the server half failed.
    expect(result.current.currentLanguage).toBe('es');
  });

  it('shows saving while the write is in flight', async () => {
    let resolveWrite: () => void = () => undefined;
    persist.mockImplementation(
      () =>
        new Promise<void>((resolve) => {
          resolveWrite = resolve;
        }),
    );
    const storage = storageWith('es');
    const { result } = renderHook(() => useSwitcherCore(storage, { serverLocale: 'en' }));

    await waitFor(() => {
      expect(result.current.syncState).toBe('saving');
    });
    resolveWrite();
    await waitFor(() => {
      expect(result.current.syncState).toBe('idle');
    });
  });

  it('settles to idle when the account catches up while the write is in flight', async () => {
    let resolveWrite: () => void = () => undefined;
    persist.mockImplementation(
      () =>
        new Promise<void>((resolve) => {
          resolveWrite = resolve;
        }),
    );
    const storage = storageWith('es');
    const { result, rerender } = renderHook(
      ({ serverLocale }: { serverLocale: string }) => useSwitcherCore(storage, { serverLocale }),
      { initialProps: { serverLocale: 'en' } },
    );

    await waitFor(() => {
      expect(result.current.syncState).toBe('saving');
    });
    // A user refetch lands the new users.locale before the PUT's response does;
    // the re-run finds nothing to write, and the first pass must still report.
    rerender({ serverLocale: 'es' });
    resolveWrite();
    await waitFor(() => {
      expect(result.current.syncState).toBe('idle');
    });
    expect(persist).toHaveBeenCalledTimes(1);
  });

  it('sends one write when the user switches while the account still holds the old locale', async () => {
    const storage = storageWith('de');
    const { result } = renderHook(() => useSwitcherCore(storage, { serverLocale: 'de' }));
    await waitFor(() => {
      expect(result.current.currentLanguage).toBe('de');
    });

    // react-i18next hands out a fresh i18n wrapper on every language change;
    // a restore pass re-run by it would find the new choice disagreeing with
    // the not-yet-refetched account and send a second PUT beside the user's.
    await act(async () => {
      await result.current.changeLanguage('pt');
    });
    await waitFor(() => {
      expect(result.current.syncState).toBe('idle');
    });
    expect(persist).toHaveBeenCalledTimes(1);
    expect(persist).toHaveBeenCalledWith('pt');
  });
});
