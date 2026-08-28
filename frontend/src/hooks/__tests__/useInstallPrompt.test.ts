// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Proves beforeinstallprompt is captured and deferred rather than discarded
// ABOUTME: Nothing listened for the event, so the only route to installing was a browser menu

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { renderHook, act, waitFor } from '@testing-library/react';
import { useInstallPrompt } from '../useInstallPrompt';

/** A stand-in for the Chromium event, which jsdom does not define. */
function makeInstallEvent(outcome: 'accepted' | 'dismissed' = 'accepted') {
  const event = new Event('beforeinstallprompt') as Event & {
    prompt: () => Promise<void>;
    userChoice: Promise<{ outcome: 'accepted' | 'dismissed' }>;
  };
  event.prompt = vi.fn().mockResolvedValue(undefined);
  event.userChoice = Promise.resolve({ outcome });
  return event;
}

beforeEach(() => {
  localStorage.clear();
  Object.defineProperty(window, 'matchMedia', {
    configurable: true,
    writable: true,
    value: (query: string) => ({
      matches: false,
      media: query,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    }),
  });
});

afterEach(() => localStorage.clear());

describe('useInstallPrompt', () => {
  it('cannot install until the browser says so', () => {
    const { result } = renderHook(() => useInstallPrompt());
    expect(result.current.canInstall).toBe(false);
  });

  it('captures the deferred event and offers the install', () => {
    const { result } = renderHook(() => useInstallPrompt());

    act(() => {
      window.dispatchEvent(makeInstallEvent());
    });

    expect(result.current.canInstall).toBe(true);
  });

  it('prevents the default so the event stays usable', () => {
    renderHook(() => useInstallPrompt());
    const event = makeInstallEvent();
    const prevented = vi.spyOn(event, 'preventDefault');

    act(() => {
      window.dispatchEvent(event);
    });

    // Without this Chromium shows its own mini-infobar and throws the
    // deferral away, so the app can never prompt.
    expect(prevented).toHaveBeenCalled();
  });

  it('shows the browser dialog and consumes the single-use event', async () => {
    const { result } = renderHook(() => useInstallPrompt());
    const event = makeInstallEvent('accepted');

    act(() => {
      window.dispatchEvent(event);
    });
    await act(async () => {
      await result.current.promptInstall();
    });

    expect(event.prompt).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(result.current.canInstall).toBe(false));
  });

  it('stops asking once the viewer declines the OS dialog', async () => {
    const { result } = renderHook(() => useInstallPrompt());

    act(() => {
      window.dispatchEvent(makeInstallEvent('dismissed'));
    });
    await act(async () => {
      await result.current.promptInstall();
    });

    expect(localStorage.getItem('dravr.installPromptDismissed')).toBe('true');
    // A fresh mount that sees the event again still stays quiet.
    const second = renderHook(() => useInstallPrompt());
    act(() => {
      window.dispatchEvent(makeInstallEvent());
    });
    expect(second.result.current.canInstall).toBe(false);
  });

  it('remembers an explicit dismissal', () => {
    const { result } = renderHook(() => useInstallPrompt());
    act(() => {
      window.dispatchEvent(makeInstallEvent());
    });
    expect(result.current.canInstall).toBe(true);

    act(() => result.current.dismiss());
    expect(result.current.canInstall).toBe(false);
    expect(localStorage.getItem('dravr.installPromptDismissed')).toBe('true');
  });

  it('never offers to install an app that is already installed', () => {
    Object.defineProperty(window, 'matchMedia', {
      configurable: true,
      writable: true,
      value: (query: string) => ({
        matches: query === '(display-mode: standalone)',
        media: query,
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
      }),
    });

    const { result } = renderHook(() => useInstallPrompt());
    act(() => {
      window.dispatchEvent(makeInstallEvent());
    });

    expect(result.current.canInstall).toBe(false);
  });

  it('survives storage that throws, as a locked-down browser does', () => {
    const getItem = vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('SecurityError');
    });
    const setItem = vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('SecurityError');
    });

    const { result } = renderHook(() => useInstallPrompt());
    act(() => {
      window.dispatchEvent(makeInstallEvent());
    });
    // Reads that throw must not suppress the offer for everyone.
    expect(result.current.canInstall).toBe(true);
    act(() => result.current.dismiss());
    expect(result.current.canInstall).toBe(false);

    getItem.mockRestore();
    setItem.mockRestore();
  });
});
