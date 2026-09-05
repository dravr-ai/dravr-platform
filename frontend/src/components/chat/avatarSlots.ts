// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The avatar palette of the conversation list, one design token per shared avatar slot
// ABOUTME: Kept beside the row rather than in it so the row module exports a component and nothing else

import { AVATAR_SLOTS } from '@pierre/chat-utils';

/**
 * The avatar palette, indexed by the row model's `avatarSlot`.
 *
 * Six DESIGN.md §2 pairs — the primary tint and its ink, tertiary, and the four pillar accents — so a
 * row's colour follows the theme like every other surface. Its length is
 * pinned to `AVATAR_SLOTS` by test: the shared hash spreads rows over exactly
 * this many slots, and a shorter list would leave a slot with no colour.
 */
export const AVATAR_SLOT_CLASSES: readonly string[] = [
  'bg-primary-container text-on-primary-container',
  'bg-tertiary/15 text-tertiary',
  'bg-activity/15 text-on-activity-container',
  'bg-nutrition/15 text-on-nutrition-container',
  'bg-recovery/15 text-on-recovery-container',
  'bg-mobility/15 text-on-mobility-container',
];

/** The colour classes for a row, wrapping so a hash beyond the palette never reads undefined. */
export function avatarSlotClass(slot: number): string {
  return AVATAR_SLOT_CLASSES[Math.abs(slot) % Math.min(AVATAR_SLOTS, AVATAR_SLOT_CLASSES.length)];
}
