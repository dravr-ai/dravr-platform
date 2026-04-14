// ABOUTME: Shared constants for social features (coach-mediated sharing)
// ABOUTME: Colors, labels, emojis, and icons for insights and reactions

import type { InsightType, ReactionType } from '@pierre/shared-types';

/**
 * Emoji mapping for reaction types
 */
export const REACTION_EMOJIS: Record<ReactionType, string> = {
  like: '\u{1F44D}',      // thumbs up
  celebrate: '\u{1F389}', // party popper
  inspire: '\u{1F4AA}',   // flexed biceps
  support: '\u{1F917}',   // hugging face
} as const;

/**
 * Color mapping for insight types (Boreal Editorial palette).
 *
 * Values read correctly against the light `surface` (#F9F9F6) and pull from
 * the same tonal family as PILLAR_COLORS so social insights feel like a
 * natural extension of the rest of the app.
 */
export const INSIGHT_TYPE_COLORS: Record<InsightType, string> = {
  achievement: '#3c6658',     // activity pillar — sage
  milestone: '#8f6a2e',       // nutrition pillar — warm bronze
  training_tip: '#0d3b2e',    // primary_container
  recovery: '#5e7a82',        // recovery pillar — muted slate
  motivation: '#7a4d5e',      // mobility pillar — aged rose
  coaching_insight: '#00241a', // primary — deep forest
} as const;

/**
 * Human-readable labels for insight types
 */
export const INSIGHT_TYPE_LABELS: Record<InsightType, string> = {
  achievement: 'Achievement',
  milestone: 'Milestone',
  training_tip: 'Training Tip',
  recovery: 'Recovery',
  motivation: 'Motivation',
  coaching_insight: 'Coach Chat',
} as const;

/**
 * Icon names for insight types (for SVG icon libraries)
 *
 * These correspond to common icon libraries like Lucide, Heroicons, etc.
 */
export const INSIGHT_TYPE_ICONS: Record<InsightType, string> = {
  achievement: 'award',
  milestone: 'flag',
  training_tip: 'zap',
  recovery: 'moon',
  motivation: 'sun',
  coaching_insight: 'message-circle',
} as const;

/**
 * Human-readable labels for reaction types
 */
export const REACTION_TYPE_LABELS: Record<ReactionType, string> = {
  like: 'Like',
  celebrate: 'Celebrate',
  inspire: 'Inspire',
  support: 'Support',
} as const;
