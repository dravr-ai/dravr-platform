// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: One source of truth for how far the keyboard has pushed the chat surface up
// ABOUTME: The composer listened alone, so the message list never moved and hid behind it

import { useEffect, useState } from 'react';
import { Keyboard, Platform } from 'react-native';

export interface KeyboardOffset {
  /** Keyboard height in dp, 0 when it is closed. */
  height: number;
  /** The OS animation duration, so the composer can match the keyboard exactly. */
  duration: number;
}

/**
 * Track the keyboard.
 *
 * This lived inside ChatInputBar, which meant the composer was the only thing
 * that knew the keyboard had opened. MessageList reserved a fixed
 * `paddingBottom: 140` sized for the composer AT REST, so once the keyboard
 * lifted the composer the padding no longer matched and the newest messages
 * sat behind it — while you were typing a reply to them.
 *
 * `keyboardWillShow` on iOS gives the height before the animation starts, so
 * the composer can move with the keyboard rather than after it. Android has no
 * `will` event and reports on `didShow`.
 */
export function useKeyboardOffset(): KeyboardOffset {
  const [offset, setOffset] = useState<KeyboardOffset>({ height: 0, duration: 250 });

  useEffect(() => {
    const showEvent = Platform.OS === 'ios' ? 'keyboardWillShow' : 'keyboardDidShow';
    const hideEvent = Platform.OS === 'ios' ? 'keyboardWillHide' : 'keyboardDidHide';

    const show = Keyboard.addListener(showEvent, (e) => {
      setOffset({
        height: e.endCoordinates.height,
        duration: Platform.OS === 'ios' ? (e.duration ?? 250) : 250,
      });
    });
    const hide = Keyboard.addListener(hideEvent, (e) => {
      setOffset({
        height: 0,
        duration: Platform.OS === 'ios' ? (e.duration ?? 250) : 250,
      });
    });

    return () => {
      show.remove();
      hide.remove();
    };
  }, []);

  return offset;
}
