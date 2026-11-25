// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The name Connected apps gives a grant's or a standing policy's preset, in the reader's language
// ABOUTME: Each key is a literal so the catalogue gate sees it; an unknown preset is left unnamed

/**
 * The name of the preset a grant or standing policy names, or `null` for a
 * plain grant and for a preset this build does not know yet.
 */
export const presetLabel = (preset: string | null, t: (key: string) => string): string | null => {
  switch (preset) {
    case 'training_coach':
      return t('tokens.connectedAppsPresetTrainingCoach');
    case 'sleep_coach':
      return t('tokens.connectedAppsPresetSleepCoach');
    default:
      return null;
  }
};
