// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The one table placing an agent category under a pillar on the web, and the classes its three readers derive
// ABOUTME: The admin pill's tone, the store's dot and the team picker's avatar — each with its own fallback, as the phone's readers keep theirs

import { BADGE_TONE, type BadgeTone } from './ui/badgeTone';

/** The pillar a coach category draws its hue from. */
export type CategoryPillar = 'activity' | 'nutrition' | 'recovery' | 'mobility';

/**
 * Which pillar a category belongs to, or none. Keyed by the lower-cased
 * category, since admin screens show whatever casing the catalogue stored;
 * the phone's `categoryPillar` folds case the same way. Recipes are food, so they sit under nutrition, on both. `analysis`,
 * `custom` and any key this build has no pillar for answer `null`; each
 * reader below says what that draws.
 */
const CATEGORY_PILLAR: Record<string, CategoryPillar> = {
  training: 'activity',
  nutrition: 'nutrition',
  recipes: 'nutrition',
  recovery: 'recovery',
  mobility: 'mobility',
};

export function coachCategoryPillar(category: string): CategoryPillar | null {
  return CATEGORY_PILLAR[category.toLowerCase()] ?? null;
}

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
 * The fill of the dot beside a category word in the store (DESIGN.md §2): the
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
