// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The one colour set every React Native Switch on the phone takes — track per state, thumb per state, iOS off ground
// ABOUTME: The web ui/Switch's pair on the native control: brand track under its ink when on, an outline track when off, each at 3:1

import type { ThemeColors } from '../../contexts/ThemeContext';

/** The props a native `Switch` reads its colours from. */
export interface SwitchColors {
  trackColor: { false: string; true: string };
  ios_backgroundColor: string;
  thumbColor: string;
}

/**
 * Spread onto a `Switch` with its current value: `{...switchColors(colors.tokens, on)}`.
 * On, the track is `primary` under an `onPrimary` thumb; off, the track is
 * `outline` under a `surfaceContainerLowest` thumb, so the control stands
 * apart from its ground in both states (a faint `border.default` track reads
 * under 1.7:1 on every ground) and the thumb from its track — iOS's default
 * white thumb reads 1.7:1 on the dark scheme's light `primary`. One function,
 * so no screen picks a second accent — `switchTone.test.ts` measures every
 * pair.
 */
export function switchColors(tokens: ThemeColors['tokens'], on: boolean): SwitchColors {
  return {
    trackColor: { false: tokens.outline, true: tokens.primary },
    ios_backgroundColor: tokens.outline,
    thumbColor: on ? tokens.onPrimary : tokens.surfaceContainerLowest,
  };
}
