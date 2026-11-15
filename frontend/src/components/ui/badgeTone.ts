// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The one tone table for every tinted pill on the web — Badge, the verdict chip and drawer, admin status, role and category pills
// ABOUTME: Maps a tone to its `.badge-*` colour class in index.css and the hover tint an interactive chip takes

/**
 * A pill's colour: the phone's `BadgeVariant` set; the four pillars, for a
 * category pill and nothing else (a state takes its feedback tone);
 * `accent`, the solid primary container for a primary-family pill that must
 * read apart from a `primary` one or mark the current moment (a conversation
 * beside an LLM call; the plan card's "now"); and the muted
 * `secondary`, for a quiet or inactive state.
 */
export type BadgeTone =
  | 'neutral'
  | 'primary'
  | 'accent'
  | 'success'
  | 'warning'
  | 'error'
  | 'info'
  | 'activity'
  | 'nutrition'
  | 'recovery'
  | 'mobility'
  | 'secondary';

/**
 * Each tone's colour class — background and ink only, so a chip with its own
 * shape (the verdict chip's tighter padding) takes the same pair — and the
 * tint a pressable chip deepens to on hover. The classes live in index.css
 * because they sit in the components layer: a caller's utility class still
 * overrides them, which a utility string here could not promise.
 */
export const BADGE_TONE: Record<BadgeTone, { tone: string; hover: string }> = {
  neutral: { tone: 'badge-neutral', hover: 'hover:bg-surface-container-highest' },
  primary: { tone: 'badge-primary', hover: 'hover:bg-primary/25' },
  accent: { tone: 'badge-accent', hover: 'hover:bg-primary-container/80' },
  success: { tone: 'badge-success', hover: 'hover:bg-success/25' },
  warning: { tone: 'badge-warning', hover: 'hover:bg-warning/25' },
  error: { tone: 'badge-error', hover: 'hover:bg-error/25' },
  info: { tone: 'badge-info', hover: 'hover:bg-info/25' },
  activity: { tone: 'badge-activity', hover: 'hover:bg-activity/25' },
  nutrition: { tone: 'badge-nutrition', hover: 'hover:bg-nutrition/25' },
  recovery: { tone: 'badge-recovery', hover: 'hover:bg-recovery/25' },
  mobility: { tone: 'badge-mobility', hover: 'hover:bg-mobility/25' },
  secondary: { tone: 'badge-secondary', hover: 'hover:bg-surface-container-high' },
};
