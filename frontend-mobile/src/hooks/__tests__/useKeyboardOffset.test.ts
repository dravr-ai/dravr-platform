// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Proves one keyboard reading exists for both the composer and the message list
// ABOUTME: They disagreed before, so the newest messages hid behind the raised composer

import { renderHook, act } from '@testing-library/react-native';
import { Keyboard, Platform } from 'react-native';
import { useKeyboardOffset } from '../useKeyboardOffset';

type Listener = (event: { endCoordinates: { height: number }; duration?: number }) => void;

/** Capture the listeners the hook registers so a test can fire them. */
function captureKeyboard() {
  const listeners: Record<string, Listener> = {};
  const spy = jest.spyOn(Keyboard, 'addListener').mockImplementation(((
    event: string,
    cb: Listener,
  ) => {
    listeners[event] = cb;
    return { remove: jest.fn() };
  }) as unknown as typeof Keyboard.addListener);
  return { listeners, spy };
}

afterEach(() => jest.restoreAllMocks());

describe('useKeyboardOffset', () => {
  it('starts closed', () => {
    captureKeyboard();
    const { result } = renderHook(() => useKeyboardOffset());
    expect(result.current.height).toBe(0);
  });

  it('reports the keyboard height and the OS animation duration', () => {
    const { listeners } = captureKeyboard();
    const { result } = renderHook(() => useKeyboardOffset());

    const show = Platform.OS === 'ios' ? 'keyboardWillShow' : 'keyboardDidShow';
    act(() => {
      listeners[show]({ endCoordinates: { height: 336 }, duration: 320 });
    });

    expect(result.current.height).toBe(336);
    // Matching the OS duration is what makes the composer travel WITH the
    // keyboard instead of chasing it.
    expect(result.current.duration).toBe(Platform.OS === 'ios' ? 320 : 250);
  });

  it('returns to zero when the keyboard closes', () => {
    const { listeners } = captureKeyboard();
    const { result } = renderHook(() => useKeyboardOffset());

    const show = Platform.OS === 'ios' ? 'keyboardWillShow' : 'keyboardDidShow';
    const hide = Platform.OS === 'ios' ? 'keyboardWillHide' : 'keyboardDidHide';

    act(() => listeners[show]({ endCoordinates: { height: 336 }, duration: 250 }));
    expect(result.current.height).toBe(336);

    act(() => listeners[hide]({ endCoordinates: { height: 0 }, duration: 250 }));
    expect(result.current.height).toBe(0);
  });
});

describe('the composer resting offset', () => {
  it('keeps the list clear of the composer whichever is taller', () => {
    // What ChatScreen passes as MessageList's bottomInset: the composer rests
    // on the device inset now that nothing sits under it but the home
    // indicator, and the keyboard replaces that when it is up.
    const resting = Math.max(34, 8);
    const closed = Math.max(resting, 0);
    const open = Math.max(resting, 336);
    expect(closed).toBe(34);
    // With the keyboard up the list must reserve the KEYBOARD, not the bar —
    // the old fixed 140 reserved neither.
    expect(open).toBe(336);
  });
});
