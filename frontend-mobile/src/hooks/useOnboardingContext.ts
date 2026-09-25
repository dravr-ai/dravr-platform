// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Assembles the OnboardingContext the shared step registry reads, from mobile's per-step hooks
// ABOUTME: The one assembly behind the root layout's routing gate and the onboarding progress hairline

import { useEffect, useMemo, useRef, useState } from 'react';
import type { OnboardingContext } from '@pierre/shared-constants';
import { useAuth } from '../contexts/AuthContext';
import { useOnboardingStatus } from './useOnboardingStatus';
import { useCoachProposalSeen } from './useCoachProposalSeen';
import { useProfileTypeChosen } from './useProfileTypeChosen';
import { useOnboardingFlag } from './useOnboardingFlag';
import { useProviderSkipped } from './useProviderSkipped';
import { useMessagingOnboarding } from './useMessagingOnboarding';

export interface OnboardingContextState {
  /** The signals `currentOnboardingStep` and `onboardingProgress` read. */
  context: OnboardingContext;
  /**
   * Whether every per-user flag the current phase reads has resolved. While
   * one is still in flight the context fails open on it, so the gate holds
   * routing until this is true rather than flash chat and bounce back.
   */
  settled: boolean;
}

/**
 * The onboarding context for the signed-in user.
 *
 * Admins are exempt from the whole flow: their primary intent is
 * administering, not chatting, so a missing provider must not block them out
 * of the app. The per-user flags fail open while they load, so nothing ever
 * claims a finished step is still ahead of the user.
 *
 * `justOnboarded` is the `needs_provider_connection` true→false transition as
 * THIS mount observed it. The root layout mounts for the whole session and
 * above the OAuth callback, so its value is the session's; a screen that
 * mounts after the transition only ever sees false→false.
 */
export function useOnboardingContext(): OnboardingContextState {
  const { user, isAuthenticated } = useAuth();
  const isAdminRole = user?.role === 'admin' || user?.role === 'super_admin';
  const onboardingActive = isAuthenticated && user?.user_status === 'active' && !isAdminRole;

  const { data: onboardingStatus } = useOnboardingStatus(onboardingActive);
  const { seen: coachProposalSeen } = useCoachProposalSeen(user?.id);
  const { chosen: profileTypeChosen } = useProfileTypeChosen(user?.id);
  // The two pre-connect steps added alongside profile-type; same fail-open flag.
  const { done: aboutYouDone } = useOnboardingFlag('dravr.about_you_done.', user?.id);
  const { done: parqDone } = useOnboardingFlag('dravr.parq_done.', user?.id);
  // Session-only escape from the provider gate, matching web. Not persisted:
  // the nudge should come back next launch.
  const { skipped: skippedProvider } = useProviderSkipped(user?.id);
  const needsProviderConnection = onboardingStatus?.needs_provider_connection;
  // The messaging steps live post-connect; only fetch the channel list there.
  const postConnect = onboardingActive && needsProviderConnection === false;
  const messaging = useMessagingOnboarding(user?.id, postConnect);

  // First-connect transition (needs true→false) — gates coach-proposal only, so
  // an already-onboarded user simply opening the app is never intercepted by it.
  const [justOnboarded, setJustOnboarded] = useState(false);
  const prevNeedsProvider = useRef<boolean | undefined>(undefined);
  useEffect(() => {
    if (prevNeedsProvider.current === true && needsProviderConnection === false) {
      setJustOnboarded(true);
    }
    prevNeedsProvider.current = needsProviderConnection;
  }, [needsProviderConnection]);

  const preConnectPending =
    needsProviderConnection === true &&
    (profileTypeChosen === undefined || aboutYouDone === undefined || parqDone === undefined);
  const postConnectPending = postConnect && (coachProposalSeen === undefined || messaging.loading);

  const context = useMemo<OnboardingContext>(
    () => ({
      onboardingActive,
      needsProviderConnection,
      skippedProvider,
      justOnboarded,
      profileTypeChosen: profileTypeChosen ?? true,
      aboutYouDone: aboutYouDone ?? true,
      parqDone: parqDone ?? true,
      coachProposalDone: coachProposalSeen ?? true,
      messagingAvailableCount: messaging.availableCount,
      messagingChannelChosen: messaging.channelChosen,
      messagingChannelDone: messaging.channelDone,
      messagingConfigureDone: messaging.configureDone,
    }),
    [
      onboardingActive,
      needsProviderConnection,
      skippedProvider,
      justOnboarded,
      profileTypeChosen,
      aboutYouDone,
      parqDone,
      coachProposalSeen,
      messaging.availableCount,
      messaging.channelChosen,
      messaging.channelDone,
      messaging.configureDone,
    ],
  );

  return { context, settled: !preConnectPending && !postConnectPending };
}
