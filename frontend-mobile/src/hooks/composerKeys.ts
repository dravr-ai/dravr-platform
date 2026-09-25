// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The composer's hardware-keyboard event type and how to read the key it carries
// ABOUTME: The key names the palettes answer to are PALETTE_KEYS in @pierre/ui-logic, shared with web

import type { NativeSyntheticEvent, TextInputKeyPressEventData } from 'react-native';

/** A keystroke as `TextInput.onKeyPress` reports it. */
export type ComposerKeyEvent = NativeSyntheticEvent<TextInputKeyPressEventData>;

/** The key this event carries, spelled as `PALETTE_KEYS` spells it. */
export function composerKey(event: ComposerKeyEvent): string {
  return event.nativeEvent.key;
}
