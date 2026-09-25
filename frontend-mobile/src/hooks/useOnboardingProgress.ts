// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The onboarding progress hairline's data: onboardingProgress() over the shared onboarding context
// ABOUTME: Reads the same useOnboardingContext() assembly as app/_layout.tsx's gate, so the bar and the gate name one current step

import { useOnboardingContext } from './useOnboardingContext';
import {
  onboardingProgress,
  type OnboardingProgressItem,
  type OnboardingStepId,
} from '@pierre/shared-constants';

/**
 * The step-progress hairline's data source.
 *
 * The context is `useOnboardingContext()`, the assembly the root layout's
 * routing gate reads, so the bar and the gate can never name a different
 * current step.
 *
 * `justOnboarded` is the one field a screen cannot observe from its own mount:
 * the gate sees it as a `needs_provider_connection` true→false transition on
 * the root layout, which is mounted above the OAuth-callback screen for the
 * whole session, while an onboarding screen mounts after that transition
 * happened and only ever sees false→false. This hook exists purely to feed a
 * progress hairline — never to gate navigation, which stays `_layout.tsx`'s
 * job — so it reads the field from `currentStepId` instead: true exactly when
 * the caller IS the coach-proposal screen, which only renders once the gate
 * decided the same thing.
 */
export function useOnboardingProgress(currentStepId: OnboardingStepId): OnboardingProgressItem[] {
  const { context } = useOnboardingContext();
  return onboardingProgress({ ...context, justOnboarded: currentStepId === 'coach_proposal' });
}
