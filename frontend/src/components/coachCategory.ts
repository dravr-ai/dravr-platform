// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The classes the web's three readers derive from an agent category's pillar, which the shared table decides
// ABOUTME: The admin pill's tone, the store's dot and the team picker's avatar — each with its own fallback, as the phone's readers keep theirs

import { coachCategoryPillar, type CategoryPillar } from '@pierre/shared-constants';
import { BADGE_TONE, type BadgeTone } from './ui/badgeTone';

/** The badge tone of an admin category pill; a category with no pillar takes `primary`. */
export function coachCategoryTone(category: string): BadgeTone {
  return coachCategoryPillar(category) ?? 'primary';
}

/** Whole class strings, so Tailwind generates each one. */
const PILLAR_DOT: Record<CategoryPillar, string> = {
  activity: 'bg-activity',
  nutrition: 'bg-nutrition',
  recovery: 'bg-recovery',
  mobility: 'bg-mobility',
};

/**
 * The fill of the dot beside a category word in the store (DESIGN.md "Status"): the
 * pillar's hue, and `bg-outline-variant` for a category with no pillar, so a
 * custom agent's dot is neutral rather than claiming the brand colour.
 */
export function coachCategoryDotClass(category: string): string {
  const pillar = coachCategoryPillar(category);
  return pillar ? PILLAR_DOT[pillar] : 'bg-outline-variant';
}

/**
 * An agent's initials avatar in the team picker: its pillar's badge tone (the
 * tint under the pillar's bound ink, hue as ground, never as text on paper),
 * and the accent tone — the primary container pair — for a category with no
 * pillar.
 */
export function coachCategoryAvatarClass(category: string): string {
  const pillar = coachCategoryPillar(category);
  return BADGE_TONE[pillar ?? 'accent'].tone;
}
