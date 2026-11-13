// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Segmented progress hairline for onboarding screens — the phone equivalent of web's labelled step dots
// ABOUTME: One segment per step of the journey, a skipped one dashed, and a "Step N of M: label" caption under it

import React from 'react';
import { Text, View } from 'react-native';
import { useTranslation } from '@pierre/i18n';
import type { OnboardingProgressItem, OnboardingStepStatus } from '@pierre/shared-constants';

export interface OnboardingProgressBarProps {
  steps: OnboardingProgressItem[];
  testID?: string;
}

/**
 * A hairline under the onboarding header — the phone's answer to web's row
 * of labelled step dots (Boreal v2.2: a hairline, not dots, on the phone) —
 * cut into one segment per step of the journey `onboardingProgress()`
 * returns. The journey is decided by the role, so the segments never change
 * count mid-flow: the steps behind the user and the one they are on are
 * filled, a step they put off stays as a dashed segment, the ones ahead are
 * the pale track. The caption under it says where they are, which is also
 * the bar's accessible name.
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
  const behind = steps.filter((step) => step.status === 'done' || step.status === 'skipped').length;
  const whereAmI = current
    ? t('onboarding.stepOfLabel', {
        current: currentIndex + 1,
        total: steps.length,
        label: t(current.labelKey),
      })
    : undefined;

  return (
    <View
      accessibilityRole="progressbar"
      accessibilityLabel={whereAmI}
      accessibilityValue={{ min: 0, max: steps.length, now: current ? currentIndex + 1 : behind }}
      testID={testID ?? 'onboarding-progress-bar'}
    >
      <View
        className="flex-row gap-1"
        importantForAccessibility="no-hide-descendants"
        accessibilityElementsHidden
      >
        {steps.map((step) => (
          <View
            key={step.id}
            testID={`onboarding-progress-segment-${step.id}`}
            className={`h-0.5 flex-1 rounded-full ${segmentClass(step.status)}`}
          />
        ))}
      </View>
      {whereAmI ? (
        <Text
          className="mt-2 text-xs text-on-surface-variant"
          importantForAccessibility="no"
          accessibilityElementsHidden
          testID="onboarding-progress-caption"
        >
          {whereAmI}
        </Text>
      ) : null}
    </View>
  );
}

/** A segment's look: filled once reached, dashed when put off, the pale track ahead. */
function segmentClass(status: OnboardingStepStatus): string {
  if (status === 'skipped') return 'border border-dashed border-outline';
  return status === 'upcoming' ? 'bg-outline-variant' : 'bg-primary';
}
