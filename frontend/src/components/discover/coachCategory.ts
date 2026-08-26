// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Category presentation for the coaches pinned on Discover — emoji, badge classes, accent colours
// ABOUTME: Keyed by the lowercase category the API stores; a label the chips render TitleCase comes out the same

/** Category emoji avatar, matching mobile. */
const CATEGORY_EMOJIS: Record<string, string> = {
  training: '🏃',
  nutrition: '🥗',
  recovery: '😴',
  recipes: '👨‍🍳',
  mobility: '🧘',
  custom: '⚙️',
};

/** Badge classes per category (Boreal pillar tokens). */
const CATEGORY_BADGES: Record<string, string> = {
  training: 'bg-activity/10 text-activity border-activity/20',
  nutrition: 'bg-nutrition/10 text-nutrition border-nutrition/20',
  recovery: 'bg-recovery/10 text-recovery border-recovery/20',
  recipes: 'bg-warning/10 text-warning border-warning/20',
  mobility: 'bg-mobility/10 text-mobility border-mobility/20',
  custom: 'bg-primary/10 text-primary border-primary/20',
};

/** Left-accent colours (synced with shared-constants PILLAR_COLORS). */
const CATEGORY_ACCENTS: Record<string, string> = {
  training: '#3c6658',
  nutrition: '#8f6a2e',
  recovery: '#5e7a82',
  recipes: '#F97316',
  mobility: '#7a4d5e',
  custom: '#00241a',
};

function key(category: string): string {
  return category.toLowerCase();
}

export function categoryEmoji(category: string): string {
  return CATEGORY_EMOJIS[key(category)] ?? CATEGORY_EMOJIS.custom;
}

export function categoryBadgeClass(category: string): string {
  return CATEGORY_BADGES[key(category)] ?? CATEGORY_BADGES.custom;
}

export function categoryAccent(category: string): string {
  return CATEGORY_ACCENTS[key(category)] ?? CATEGORY_ACCENTS.custom;
}

/** LLM context window the token share is measured against. */
export const CONTEXT_WINDOW_SIZE = 128000;

/** Share of the context window a coach's prompt takes, as a one-decimal percentage. */
export function contextPercentage(tokens: number): string {
  return ((tokens / CONTEXT_WINDOW_SIZE) * 100).toFixed(1);
}
