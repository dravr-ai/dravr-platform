// ABOUTME: Onboarding flow container — picks the current step from the registry and renders it, else the dashboard
// ABOUTME: Presentational: receives flow state + completion callbacks from useOnboardingState (mounted in AppContent)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import type { ReactNode } from 'react';
import OnboardingProfileType from './OnboardingProfileType';
import OnboardingAboutYou from './OnboardingAboutYou';
import OnboardingParq from './OnboardingParq';
import OnboardingConnectProvider from './OnboardingConnectProvider';
import OnboardingCoachProposal from './OnboardingCoachProposal';
import OnboardingMessagingChannel from './OnboardingMessagingChannel';
import OnboardingMessagingConfigure from './OnboardingMessagingConfigure';
import OnboardingProgress from './OnboardingProgress';
import { currentOnboardingStep, onboardingProgress, type OnboardingStepId } from '../onboarding/steps';
import type { OnboardingState } from '../hooks/useOnboardingState';

/**
 * Renders the current onboarding step — the first applicable step not yet
 * complete — or `fallback` (the dashboard) once onboarding is finished. The
 * step decision is owned by the registry (`currentOnboardingStep`); this
 * component only maps each step id to its component + completion callback.
 */
export default function OnboardingFlow({
  state,
  userDisplayName,
  fallback,
}: {
  state: OnboardingState;
  userDisplayName?: string | null;
  fallback: ReactNode;
}) {
  const step = currentOnboardingStep(state.ctx);
  if (!step) {
    return <>{fallback}</>;
  }

  return (
    <>
      <OnboardingProgress steps={onboardingProgress(state.ctx)} />
      {renderStep(step.id, state, userDisplayName)}
    </>
  );
}

/** Maps a step id to its component, wired with the matching completion callback. */
function renderStep(
  id: OnboardingStepId,
  state: OnboardingState,
  userDisplayName?: string | null,
): ReactNode {
  switch (id) {
    case 'profile_type':
      return (
        <OnboardingProfileType
          userDisplayName={userDisplayName}
          onComplete={state.completeProfileType}
        />
      );
    case 'about_you':
      return (
        <OnboardingAboutYou
          userDisplayName={userDisplayName}
          onComplete={state.completeAboutYou}
        />
      );
    case 'parq':
      return (
        <OnboardingParq userDisplayName={userDisplayName} onComplete={state.completeParq} />
      );
    case 'connect_provider':
      return (
        <OnboardingConnectProvider
          userDisplayName={userDisplayName}
          onContinueWithoutProvider={state.skipProvider}
        />
      );
    case 'coach_proposal':
      return (
        <OnboardingCoachProposal
          userDisplayName={userDisplayName}
          onComplete={state.completeCoachProposal}
        />
      );
    case 'messaging_channel':
      return (
        <OnboardingMessagingChannel
          userDisplayName={userDisplayName}
          channels={state.availableChannels}
          onSelect={state.chooseChannel}
          onSkip={state.skipMessaging}
        />
      );
    case 'messaging_configure': {
      const chosen = state.availableChannels.find((c) => c.channel === state.chosenChannel);
      const channel = state.chosenChannel ?? '';
      return (
        <OnboardingMessagingConfigure
          userDisplayName={userDisplayName}
          channel={channel}
          displayName={chosen?.display_name ?? channel}
          onLinked={state.completeMessagingConfigure}
          onSkip={state.skipMessaging}
        />
      );
    }
  }
}
