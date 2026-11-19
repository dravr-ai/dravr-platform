// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The one tone table for every tinted message block on the web — Alert, the notice and banner shapes, an inline form error box
// ABOUTME: Maps a feedback tone to its `.alert-*` colour class in index.css and the ink a child that sets its own colour takes

/** A message block's colour: the four feedback states. A pillar hue never colours one (DESIGN.md "Alerts"). */
export type AlertTone = 'success' | 'warning' | 'error' | 'info';

/**
 * Each tone's colour class — the hue's /10 tint, its /30 edge and the ink the
 * token tree binds to it — and that ink as a utility, for a child that names
 * its colour (an icon, a title, a line set apart from the body). The classes
 * carry colour only and live in index.css's components layer, so a banner
 * with its own shape (a header strip, a toast-sized error, a list row) takes
 * the same pair and its utilities still win. The edge shows only where the
 * shape draws a border.
 */
export const ALERT_TONE: Record<AlertTone, { tone: string; ink: string }> = {
  success: { tone: 'alert-success', ink: 'text-on-success-container' },
  warning: { tone: 'alert-warning', ink: 'text-on-warning-container' },
  error: { tone: 'alert-error', ink: 'text-on-error-container' },
  info: { tone: 'alert-info', ink: 'text-on-info-container' },
};
