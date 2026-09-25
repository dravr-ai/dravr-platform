// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The keys the composer's command and mention palettes answer to, spelled as both platforms report them
// ABOUTME: A DOM KeyboardEvent.key and a React Native onKeyPress nativeEvent.key use the same names

/**
 * Arrows move the highlight, Enter and Tab take the highlighted row, Escape
 * dismisses the palette for the current draft — one contract, so an athlete
 * with a keyboard finds the same behaviour on either client.
 */
export const PALETTE_KEYS = {
  down: 'ArrowDown',
  up: 'ArrowUp',
  enter: 'Enter',
  tab: 'Tab',
  escape: 'Escape',
} as const;
