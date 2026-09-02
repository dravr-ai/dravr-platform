// ABOUTME: Coach tuning bounds and the category vocabulary shared by the web and mobile coach surfaces
// ABOUTME: Bounds mirror pierre_core::constants::tool_execution; category labels are corpus keys resolved with t()

import type { CoachCategory } from '@pierre/shared-types';

/**
 * Smallest tool-loop iteration budget a coach may be given. One pass still
 * lets the model call a tool and answer from its result.
 */
export const MIN_MAX_TOOL_ITERATIONS = 1;

/**
 * Largest tool-loop iteration budget a coach may be given. Caps how long one
 * chat turn can spend fanning out tool calls before it must answer.
 */
export const MAX_MAX_TOOL_ITERATIONS = 50;

/**
 * Budget a coach runs on when it carries no per-coach override — the value the
 * tenant-wide `tool_execution.max_iterations` setting itself defaults to.
 */
export const DEFAULT_MAX_TOOL_ITERATIONS = 10;

/**
 * The corpus key naming each coach category — the filter chips, the card
 * badges and the onboarding proposal read one table, so a screen never shows
 * a category as a translated word in one place and a raw enum in another.
 */
export const COACH_CATEGORY_LABEL_KEY: Record<CoachCategory, string> = {
  training: 'chat.categoryTraining',
  nutrition: 'chat.categoryNutrition',
  recovery: 'chat.categoryRecovery',
  recipes: 'chat.categoryRecipes',
  mobility: 'chat.categoryMobility',
  custom: 'chat.categoryCustom',
};

/**
 * The label key for a category as the wire carries it. A catalogue row's
 * category is a plain string on some reads, so a value outside the six known
 * ones reads as custom rather than as a missing key.
 */
export function coachCategoryLabelKey(category: string): string {
  return COACH_CATEGORY_LABEL_KEY[category as CoachCategory] ?? COACH_CATEGORY_LABEL_KEY.custom;
}
