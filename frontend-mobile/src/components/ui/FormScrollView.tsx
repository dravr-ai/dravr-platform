// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The ScrollView every auth form uses, keeping the focused field above the keyboard on both platforms
// ABOUTME: Keyboard avoidance is the container's job here, so a form cannot ship without it and hide the field being typed into

import React from 'react';
import { KeyboardAwareScrollView } from 'react-native-keyboard-controller';
import type { ScrollViewProps } from 'react-native';
import { spacing } from '../../constants/theme';

/**
 * The gap kept between the keyboard and the caret of the focused field, so the
 * field's own error line below it stays readable while the athlete types.
 */
export const FORM_KEYBOARD_GAP = spacing.md;

/**
 * The scroll container every auth form uses.
 *
 * Expo makes Android edge-to-edge unconditionally: the window no longer
 * resizes for the keyboard, so nothing built on window resize can react to it.
 * `automaticallyAdjustKeyboardInsets` — the mechanism these forms relied on
 * before — is iOS-only, and React Native's `KeyboardAvoidingView` measures a
 * keyboard frame the platform no longer reports the same way, so wrapping the
 * forms in one changed nothing on device (carnet#353). On a 1080x1600 screen
 * the password field sat entirely behind the keyboard.
 *
 * `KeyboardAwareScrollView` reads the IME inset natively on both platforms —
 * Expo Go ships its native half — and scrolls the focused field clear of the
 * keyboard the way iOS's own inset adjustment did, so the iOS behaviour the
 * forms had is kept and Android gains it. The container owns this rather than
 * each of the four forms, which is how the Android gap went unnoticed: every
 * screen carried its own copy of an iOS-only prop.
 */
export function FormScrollView(props: ScrollViewProps) {
  return (
    <KeyboardAwareScrollView
      bottomOffset={FORM_KEYBOARD_GAP}
      keyboardShouldPersistTaps="handled"
      {...props}
    />
  );
}
