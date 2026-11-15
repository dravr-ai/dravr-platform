// ABOUTME: Onboarding flow container — picks the current step from the registry and renders it, else the dashboard
// ABOUTME: Presentational: receives flow state + completion callbacks from useOnboardingState (mounted in AppContent)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { lazy, Suspense, type ReactNode } from 'react';
import OnboardingAnalyticsConsent from './OnboardingAnalyticsConsent';
import OnboardingProfileType from './OnboardingProfileType';
import OnboardingParq from './OnboardingParq';
import OnboardingConnectProvider from './OnboardingConnectProvider';
import OnboardingCoachProposal from './OnboardingCoachProposal';
import OnboardingProgress from './OnboardingProgress';
import OnboardingShell from './OnboardingShell';
import { currentOnboardingStep, onboardingProgress, type OnboardingStepId } from '../onboarding/steps';
import type { OnboardingState } from '../hooks/useOnboardingState';

// The coach-only steps load when a coach reaches them: every sign-up meets
// this flow before first paint, and an athlete never needs the seat, group
// and invite screens (or the coach-access request they carry).
const OnboardingCoachSeat = lazy(() => import('./OnboardingCoachSeat'));
const OnboardingCoachGroup = lazy(() => import('./OnboardingCoachGroup'));

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

  // The positioned parent the progress bar is laid over: it starts wherever
  // the flow does, so a strip App mounts above it stacks instead of overlapping.
  return (
    <div className="relative" data-testid="onboarding-flow">
      {step.hiddenFromProgress ? null : (
        <OnboardingProgress steps={onboardingProgress(state.ctx)} settledByInvite={state.settledByInvite} />
      )}
      {/* While a coach step's chunk arrives, the screen draws what every step
          draws while it loads — the shell and its spinner — not a blank page
          under the progress bar. */}
      <Suspense
        fallback={
          <OnboardingShell>
            <div className="flex flex-col items-center gap-4 py-8">
              <div className="pierre-spinner w-10 h-10 border-on-surface border-t-transparent" />
            </div>
          </OnboardingShell>
        }
      >
        {renderStep(step.id, state, userDisplayName)}
      </Suspense>
    </div>
  );
}

/** Maps a step id to its component, wired with the matching completion callback. */
function renderStep(
  id: OnboardingStepId,
  state: OnboardingState,
  userDisplayName?: string | null,
): ReactNode {
  switch (id) {
    case 'analytics_consent':
      return <OnboardingAnalyticsConsent onAnswer={state.answerAnalyticsConsent} />;
    case 'profile_type':
      return (
        <OnboardingProfileType
          userDisplayName={userDisplayName}
          onComplete={state.completeProfileType}
        />
      );
    case 'parq':
      return (
        <OnboardingParq userDisplayName={userDisplayName} onComplete={state.completeParq} />
      );
    case 'connect_provider':
      return (
        <OnboardingConnectProvider
          onContinueWithoutProvider={state.skipProvider}
          onContinue={
            state.ctx.needsProviderConnection === false ? state.completeConnectProvider : undefined
          }
        />
      );
    case 'coach_proposal':
      return (
        <OnboardingCoachProposal
          userDisplayName={userDisplayName}
          withoutProvider={state.ctx.needsProviderConnection === true}
          onSave={state.persistCoachProposal}
          onComplete={state.completeCoachProposal}
        />
      );
    case 'coach_seat':
      return <OnboardingCoachSeat onComplete={state.completeCoachSeat} />;
    // One screen component for both segments, at one place in the tree, so the
    // group just made and its invite link survive the step changing under it.
    case 'coach_group':
    case 'coach_invite':
      return (
        <OnboardingCoachGroup
          onCreated={state.markCoachGroupCreated}
          onComplete={state.completeCoachGroup}
          seatSettledByInvite={state.settledByInvite}
        />
      );
  }
}
