// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The avatar palette of the conversation list, one design token per shared avatar slot, and the avatar's shape
// ABOUTME: Kept beside the row rather than in it so the row module exports a component and nothing else

import { AVATAR_SLOT_HUES, AVATAR_SLOTS, type AvatarSlotHue } from '@pierre/chat-utils';

/**
 * The Tailwind pair each shared hue resolves to on the web: the fill and its ink.
 *
 * DESIGN.md §2 pairs — the primary tint and its ink, tertiary, and the four
 * pillar accents — so a row's colour follows the theme like every other
 * surface. Each entry is a complete literal so the Tailwind scanner sees
 * every class in source. Keyed by `AvatarSlotHue`, so a hue added to the
 * shared list without a binding here fails to compile instead of rendering
 * an unstyled avatar.
 */
const HUE_CLASSES: Record<AvatarSlotHue, string> = {
  primary: 'bg-primary-container text-on-primary-container',
  activity: 'bg-activity/15 text-on-activity-container',
  nutrition: 'bg-nutrition/15 text-on-nutrition-container',
  recovery: 'bg-recovery/15 text-on-recovery-container',
  mobility: 'bg-mobility/15 text-on-mobility-container',
  tertiary: 'bg-tertiary/15 text-tertiary',
};

/**
 * The avatar palette, indexed by the row model's `avatarSlot`.
 *
 * The order is `AVATAR_SLOT_HUES`, the shared list the hash indexes into; this
 * module binds tokens to hues and orders nothing. The phone binds its own
 * tokens to the same list, so slot n is the same hue on both clients and a
 * thread keeps its colour from one device to the next. Its length is
 * `AVATAR_SLOTS` by construction, so every slot the hash can produce has a
 * colour.
 */
export const AVATAR_SLOT_CLASSES: readonly string[] = AVATAR_SLOT_HUES.map((hue) => HUE_CLASSES[hue]);

/**
 * The colour classes for a row, wrapping so a hash beyond the palette never
 * reads undefined. The wrap is Euclidean — `-1` is the last slot, as it is on
 * the phone — so even an out-of-range slot lands on the same hue on both clients.
 */
export function avatarSlotClass(slot: number): string {
  return AVATAR_SLOT_CLASSES[((slot % AVATAR_SLOTS) + AVATAR_SLOTS) % AVATAR_SLOTS];
}

/**
 * What the avatar stands for. A circle is one counterpart — a person or an
 * agent; a square is many — a room. The same convention as Fluent 2 and
 * GitLab Pajamas, and the inverse of Primer's, which gives the square to bots.
 * The phone's `InitialsAvatar` takes the same two values.
 */
export type AvatarShape = 'circle' | 'square';

/**
 * The corner class for a shape: full for a person or an agent, `xl` (12px,
 * the floating-card step of the ladder) for a room — DESIGN.md §2 radii.
 * Shape reaches no screen reader, so it only reinforces the kind glyph and
 * the accessible name beside it.
 */
export function avatarShapeClass(shape: AvatarShape): string {
  return shape === 'square' ? 'rounded-xl' : 'rounded-full';
}
