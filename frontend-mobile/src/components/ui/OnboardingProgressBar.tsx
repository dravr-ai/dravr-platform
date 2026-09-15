// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Filling progress hairline for onboarding screens — the phone equivalent of web's labelled step dots
// ABOUTME: Renders under a screen's header; no visible per-step labels, its accessible name carries "Step N of M: label"

import React from 'react';
import { View } from 'react-native';
import { useTranslation } from '@pierre/i18n';
import type { OnboardingProgressItem } from '@pierre/shared-constants';

export interface OnboardingProgressBarProps {
  steps: OnboardingProgressItem[];
  testID?: string;
}

/**
 * A single filling hairline under the onboarding header — the phone's answer
 * to web's row of labelled step dots (Boreal v2.2 D5: a hairline, not dots,
 * on the phone). The fill fraction is position-based, matching
 * `onboardingProgress()`'s own "nothing reads done until it's behind us"
 * rule: the current step counts as `(index + 1)` of the journey's total, so
 * the bar never reads full until the last step is actually current.
 *
 * Renders nothing for an empty journey (onboarding already finished, or the
 * assembling hook's flags are still loading) rather than a bar stuck at an
 * arbitrary fraction.
 */
export function OnboardingProgressBar({ steps, testID }: OnboardingProgressBarProps) {
  const { t } = useTranslation();
  if (steps.length === 0) {
    return null;
  }

  const currentIndex = steps.findIndex((step) => step.status === 'current');
  const current = currentIndex >= 0 ? steps[currentIndex] : undefined;
  const doneCount = steps.filter((step) => step.status === 'done').length;
  const position = currentIndex >= 0 ? currentIndex + 1 : doneCount;
  const fraction = Math.min(1, position / steps.length);

  const label = current
    ? t('onboarding.stepOfLabel', {
        current: currentIndex + 1,
        total: steps.length,
        label: t(current.labelKey),
      })
    : undefined;

  return (
    <View
      className="h-0.5 w-full overflow-hidden rounded-full bg-outline-variant"
      accessibilityRole="progressbar"
      accessibilityLabel={label}
      testID={testID ?? 'onboarding-progress-bar'}
    >
      <View className="h-full rounded-full bg-primary" style={{ width: `${fraction * 100}%` }} />
    </View>
  );
}
