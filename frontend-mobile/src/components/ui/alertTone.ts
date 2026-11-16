// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The one tone table for every tinted message block on the phone — banners, notices, inline errors — the web's `.alert-*` family
// ABOUTME: Fill, edge and ink per feedback tone as whole NativeWind classes, plus the ink as a colour value for a glyph

import type { ThemeColors } from '../../contexts/ThemeContext';
import { badgeInkColor } from './Badge';

/** A message block's colour: the four feedback states. A pillar hue never colours one (DESIGN.md "Alerts"). */
export type AlertTone = 'success' | 'warning' | 'error' | 'info';

/**
 * Fill, edge and ink per tone, as whole class strings so NativeWind generates
 * every one: the hue's /10 tint, its /30 edge (drawn only where the block sets
 * a border width) and the ink the token tree binds to the hue. React Native
 * inherits no text colour from a fill, so every `Text` inside a block names
 * the `ink` class itself; a glyph takes `alertInkColor`. The ink clears AA on
 * the tint over every ground from `surface` to `surface-container-highest` in
 * both schemes (4.76:1 at worst, light `warning` on the highest tier); the
 * bare hue on its own tint reads 2.03:1 there.
 */
export const ALERT_TONE: Record<AlertTone, { fill: string; edge: string; ink: string }> = {
  success: { fill: 'bg-success/10', edge: 'border-success/30', ink: 'text-on-success-container' },
  warning: { fill: 'bg-warning/10', edge: 'border-warning/30', ink: 'text-on-warning-container' },
  error: { fill: 'bg-error/10', edge: 'border-error/30', ink: 'text-on-error-container' },
  info: { fill: 'bg-info/10', edge: 'border-info/30', ink: 'text-on-info-container' },
};

/**
 * The tone's ink as a colour value, for a glyph whose `color` is a prop (a
 * Feather or lucide icon beside the block's text). The alert and the badge
 * bind the same ink to a feedback hue, so this is the badge's runtime twin.
 */
export function alertInkColor(tone: AlertTone, colors: ThemeColors): string {
  return badgeInkColor(tone, colors);
}
