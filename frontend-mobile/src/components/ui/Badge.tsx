// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The pill that names a state beside content — neutral, primary, success, warning, error, info — the web's `.badge` family on the phone
// ABOUTME: One tone table, so a badge and the verdict chip take the same tint and bound ink instead of hand-writing them (DESIGN.md "Badges and chips")

import React from 'react';
import { Text, View } from 'react-native';
import type { ThemeColors } from '../../contexts/ThemeContext';

/** The web `Badge`'s variants; `neutral` is its `.badge-neutral`. */
export type BadgeVariant = 'neutral' | 'primary' | 'success' | 'warning' | 'error' | 'info';

/**
 * Ground and ink per variant, as whole class strings so NativeWind generates
 * every one. React Native has no `background` shorthand that sets an ink, so
 * each filled pill names its label's colour itself: the hue's /15 tint
 * carries the ink the token tree binds to it, which clears AA in both schemes
 * on every ground from `surface` to `surface-container-highest` (4.58:1 at
 * worst, light `info` on the highest tier; the test measures every pair).
 * Error and primary label in their container inks, as the web's `.badge-error`
 * and `.badge-primary` do: the bare hue falls to 4.2:1 (error, light
 * `surface-container-low`) and 4.17:1 (primary, light
 * `surface-container-high`) on its own tint, where the bound inks hold 6.1:1
 * and 6.8:1.
 *
 * Anything tinted by tone reads this table — a Badge, and the verdict chip,
 * which is a button with a glyph rather than a Badge.
 */
export const BADGE_TONE: Record<BadgeVariant, { fill: string; ink: string }> = {
  neutral: { fill: 'bg-surface-container-high', ink: 'text-on-surface' },
  primary: { fill: 'bg-primary/15', ink: 'text-on-primary-container' },
  success: { fill: 'bg-success/15', ink: 'text-on-success-container' },
  warning: { fill: 'bg-warning/15', ink: 'text-on-warning-container' },
  error: { fill: 'bg-error/15', ink: 'text-on-error-container' },
  info: { fill: 'bg-info/15', ink: 'text-on-info-container' },
};

/**
 * The same ink as a colour value, for a glyph whose `color` is a prop rather
 * than a class (a vector icon beside a tone's label). Each entry is the
 * runtime twin of the `ink` class above — the token that class names.
 */
export function badgeInkColor(variant: BadgeVariant, colors: ThemeColors): string {
  switch (variant) {
    case 'neutral':
      return colors.tokens.onSurface;
    case 'primary':
      return colors.tokens.onPrimaryContainer;
    case 'success':
      return colors.ink.success;
    case 'warning':
      return colors.ink.warning;
    case 'error':
      return colors.tokens.onErrorContainer;
    case 'info':
      return colors.ink.info;
  }
}

export interface BadgeProps {
  /** The words the badge says; they carry the meaning, the tint only echoes it. */
  label: string;
  variant: BadgeVariant;
  /** Outer layout only (margins, `self-start` in a column); the recipe itself is not overridable. */
  className?: string;
  testID?: string;
}

/** A pill around its label, tinted by its variant. */
export function Badge({ label, variant, className, testID }: BadgeProps) {
  const { fill, ink } = BADGE_TONE[variant];
  return (
    <View testID={testID} className={`rounded-full px-3 py-1 ${fill}${className ? ` ${className}` : ''}`}>
      <Text className={`text-xs font-medium ${ink}`}>{label}</Text>
    </View>
  );
}
