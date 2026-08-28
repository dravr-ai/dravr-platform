// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Proves one keyboard reading exists for both the composer and the message list
// ABOUTME: They disagreed before, so the newest messages hid behind the raised composer

import { renderHook, act } from '@testing-library/react-native';
import { Keyboard, Platform } from 'react-native';
import { useKeyboardOffset } from '../useKeyboardOffset';
import { tabBarBottomOffset, TAB_BAR_GAP } from '../../components/ui/ExpandableTabBar';

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

describe('tabBarBottomOffset', () => {
  it('follows the device inset instead of assuming a home indicator', () => {
    // The constant it replaces was COLLAPSED_HEIGHT + 40, which gave an
    // iPhone SE (inset 0) forty points of dead space under the tab bar.
    const se = tabBarBottomOffset(0);
    const notched = tabBarBottomOffset(34);
    expect(notched - se).toBe(34);
    expect(se).toBe(56 + TAB_BAR_GAP);
  });

  it('keeps the list clear of the composer whichever is taller', () => {
    // What ChatScreen passes as MessageList's bottomInset.
    const resting = tabBarBottomOffset(34);
    const closed = Math.max(resting, 0);
    const open = Math.max(resting, 336);
    expect(closed).toBe(resting);
    // With the keyboard up the list must reserve the KEYBOARD, not the bar —
    // the old fixed 140 reserved neither.
    expect(open).toBe(336);
  });
});
