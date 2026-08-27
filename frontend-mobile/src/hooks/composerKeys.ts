// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The composer's hardware-keyboard event type and the keys both palettes navigate with
// ABOUTME: One definition for the command palette, the mention palette and the input bar that offers them keys

import type { NativeSyntheticEvent, TextInputKeyPressEventData } from 'react-native';

/** A keystroke as `TextInput.onKeyPress` reports it. */
export type ComposerKeyEvent = NativeSyntheticEvent<TextInputKeyPressEventData>;

/**
 * The keys a palette answers to, spelled as the platform reports them.
 *
 * Arrows move the highlight, Enter and Tab take the highlighted row, Escape
 * dismisses the palette for the current draft — the same contract the web
 * composer implements, so an athlete with a keyboard finds the same behaviour
 * on either client.
 */
export const COMPOSER_KEYS = {
  down: 'ArrowDown',
  up: 'ArrowUp',
  enter: 'Enter',
  tab: 'Tab',
  escape: 'Escape',
} as const;

/** The key this event carries. */
export function composerKey(event: ComposerKeyEvent): string {
  return event.nativeEvent.key;
}
