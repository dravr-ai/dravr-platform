// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Assembles an OnboardingContext from mobile's per-step hooks and returns onboardingProgress() for the progress hairline
// ABOUTME: A screen-scoped, display-only duplicate of app/_layout.tsx's routing assembly — see the justOnboarded note below for why it can't just import that

import { useAuth } from '../contexts/AuthContext';
import { useOnboardingStatus } from './useOnboardingStatus';
import { useCoachProposalSeen } from './useCoachProposalSeen';
import { useProfileTypeChosen } from './useProfileTypeChosen';
import { useOnboardingFlag } from './useOnboardingFlag';
import { useProviderSkipped } from './useProviderSkipped';
import { useMessagingOnboarding } from './useMessagingOnboarding';
import {
  onboardingProgress,
  type OnboardingContext,
  type OnboardingProgressItem,
  type OnboardingStepId,
} from '@pierre/shared-constants';

/**
 * The step-progress hairline's data source.
 *
 * Builds the same `OnboardingContext` shape `app/_layout.tsx`'s `RootLayoutNav`
 * assembles for routing, from the same per-step hooks (`useOnboardingStatus`,
 * `useCoachProposalSeen`, `useProfileTypeChosen`, `useOnboardingFlag`,
 * `useProviderSkipped`, `useMessagingOnboarding`), so the bar and the gate can
 * never name a different current step. This duplicates that assembly rather
 * than importing it, because `_layout.tsx` is out of this lane's file scope —
 * see the Boreal v2.2 Phase 5 execution plan, P5.10.
 *
 * `justOnboarded` is the one field this hook cannot observe honestly: the
 * gate detects it as a `needs_provider_connection` true→false transition on a
 * ref mounted above the OAuth-callback screen, for the whole app session. A
 * screen-scoped hook that only mounts once routing has already landed the
 * user on `coach_proposal` mounts *after* that transition happened, so its
 * own local ref would only ever observe false→false. Since this hook exists
 * purely to feed a progress hairline — never to gate navigation, which stays
 * `_layout.tsx`'s job — it infers the field from `currentStepId` instead:
 * true exactly when the caller IS the coach-proposal screen, which only
 * renders when the gate already decided the same thing.
 */
export function useOnboardingProgress(currentStepId: OnboardingStepId): OnboardingProgressItem[] {
  const { user, isAuthenticated } = useAuth();
  const isAdminRole = user?.role === 'admin' || user?.role === 'super_admin';
  const onboardingActive = isAuthenticated && !isAdminRole;

  const { data: onboardingStatus } = useOnboardingStatus(onboardingActive);
  const { seen: coachProposalSeen } = useCoachProposalSeen(user?.id);
  const { chosen: profileTypeChosen } = useProfileTypeChosen(user?.id);
  const { done: aboutYouDone } = useOnboardingFlag('dravr.about_you_done.', user?.id);
  const { done: parqDone } = useOnboardingFlag('dravr.parq_done.', user?.id);
  const { skipped: skippedProvider } = useProviderSkipped(user?.id);
  const postConnect = onboardingActive && onboardingStatus?.needs_provider_connection === false;
  const messaging = useMessagingOnboarding(user?.id, postConnect);

  const ctx: OnboardingContext = {
    onboardingActive,
    needsProviderConnection: onboardingStatus?.needs_provider_connection,
    skippedProvider,
    justOnboarded: currentStepId === 'coach_proposal',
    // Fail open, same as `_layout.tsx`: a flag still loading must never make
    // the bar claim a finished step is still ahead of the user.
    profileTypeChosen: profileTypeChosen ?? true,
    aboutYouDone: aboutYouDone ?? true,
    parqDone: parqDone ?? true,
    coachProposalDone: coachProposalSeen ?? true,
    messagingAvailableCount: messaging.availableCount,
    messagingChannelChosen: messaging.channelChosen,
    messagingChannelDone: messaging.channelDone,
    messagingConfigureDone: messaging.configureDone,
  };

  return onboardingProgress(ctx);
}
