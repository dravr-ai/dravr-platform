// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Persists an onboarding step answer and writes the same row into the cached onboarding status
// ABOUTME: So the progress hairline on the next screen reads a skip at once, not at the next status read

import { useCallback } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import type { OnboardingStatusResponse } from '@pierre/api-client';
import { QUERY_KEYS, withStepRecorded } from '@pierre/shared-constants';
import { userApi } from '../services/api';

/**
 * Records an onboarding step's answer: the server write, best-effort as each
 * screen's own local flag already moved the flow on, and the same row in the
 * cached `GET /api/me/onboarding-status` the onboarding context reads. The
 * progress hairline draws a step the user put off from those rows, and the
 * next status read would otherwise land after the next screen has drawn.
 */
export function useRecordOnboardingStep(): (
  stepId: string,
  status: 'complete' | 'skipped' | 'not_applicable',
) => void {
  const queryClient = useQueryClient();
  return useCallback(
    (stepId, status) => {
      queryClient.setQueryData<OnboardingStatusResponse>(
        QUERY_KEYS.user.onboardingStatus(),
        (cached) => cached && { ...cached, steps: withStepRecorded(cached.steps, stepId, status) },
      );
      userApi.setOnboardingStep(stepId, status).catch(() => {});
    },
    [queryClient],
  );
}
