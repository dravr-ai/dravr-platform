// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Assembles the OnboardingContext the shared step registry reads, from mobile's per-step hooks
// ABOUTME: The one assembly behind the root layout's routing gate and the onboarding progress hairline

import { useEffect, useMemo, useRef, useState } from 'react';
import {
  athleteStepsWaived as serverAthleteStepsWaived,
  isServerStepComplete,
  isConnectProviderRecorded,
  isConnectProviderSkipped,
  beforeProviderGate,
  pastProviderGate,
  skippedStepIds,
  type OnboardingContext,
} from '@pierre/shared-constants';
import { useAuth } from '../contexts/AuthContext';
import { useOnboardingStatus } from './useOnboardingStatus';
import { useCoachProposalSeen } from './useCoachProposalSeen';
import { useProfileTypeChosen } from './useProfileTypeChosen';
import {
  ATHLETE_STEPS_WAIVED_PREFIX,
  COACH_GROUP_DONE_PREFIX,
  COACH_SEAT_DONE_PREFIX,
  useOnboardingFlag,
} from './useOnboardingFlag';
import { useCoachGroupCreated } from './useCoachGroupCreated';
import { useProviderSkipped } from './useProviderSkipped';
import { useConnectProviderDone } from './useConnectProviderDone';

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
 * mounts after the transition only ever sees false→false. It keeps the connect
 * step up for a second service; the agent step does not read it — that one is
 * derived from server state (carnet#1042).
 */
export function useOnboardingContext(): OnboardingContextState {
  const { user, isAuthenticated } = useAuth();
  const isAdminRole = user?.role === 'admin' || user?.role === 'super_admin';
  const onboardingActive = isAuthenticated && user?.user_status === 'active' && !isAdminRole;

  const { data: onboardingStatus } = useOnboardingStatus(onboardingActive);
  const { seen: coachProposalSeenLocal } = useCoachProposalSeen(user?.id);
  // The agent step is gated on server state alone (carnet#1042), so its done
  // flag reads the server's step row too: one answered on the web or on
  // another device is not asked again on a fresh install.
  const coachProposalSeen =
    coachProposalSeenLocal === undefined
      ? undefined
      : coachProposalSeenLocal ||
        isServerStepComplete(onboardingStatus?.steps, 'coach_proposal');
  const { chosen: profileTypeChosenLocal } = useProfileTypeChosen(user?.id);
  // The server's step row too, as web reads it: a role answered on another
  // surface, or settled by a team-issued coach-seat invitation before the
  // first sign-in, is not asked again on the phone.
  const profileTypeChosen =
    profileTypeChosenLocal === undefined
      ? undefined
      : profileTypeChosenLocal || isServerStepComplete(onboardingStatus?.steps, 'profile_type');
  // The PAR-Q, the third screen: the same fail-open flag, plus the server's
  // step row, which the chat intake and the web write too.
  const { done: parqDoneLocal } = useOnboardingFlag('dravr.parq_done.', user?.id);
  const parqDone =
    parqDoneLocal === undefined
      ? undefined
      : parqDoneLocal || isServerStepComplete(onboardingStatus?.steps, 'parq');
  // A coach who does not train: the athlete steps leave the journey. Fails
  // closed — see useOnboardingFlag's `fallback`. The server's not_applicable
  // step rows carry it across devices and from the chat intake.
  const { done: athleteStepsWaivedLocal } = useOnboardingFlag(
    ATHLETE_STEPS_WAIVED_PREFIX,
    user?.id,
    false,
  );
  const athleteStepsWaived =
    athleteStepsWaivedLocal === true || serverAthleteStepsWaived(onboardingStatus?.steps);
  // The coach's group step: server truth for who coaches, the usual fail-open
  // flag plus the server step row for whether it is done.
  const coachesOthers = onboardingStatus?.coaches_others === true;
  const { done: coachGroupDoneLocal } = useOnboardingFlag(COACH_GROUP_DONE_PREFIX, user?.id);
  const coachGroupDone =
    coachGroupDoneLocal !== false || isServerStepComplete(onboardingStatus?.steps, 'coach_group');
  // The coach-seat step: a device flag, fail-open like the others; the seat
  // itself is server state the screen reads. The group made this session
  // moves the coach on to its invite.
  const { done: coachSeatDoneLocal } = useOnboardingFlag(COACH_SEAT_DONE_PREFIX, user?.id);
  // The seat step's server row exists only where a team-issued coach-seat
  // invitation settled the seat before sign-in, so that coach never meets it.
  const coachSeatSettled = isServerStepComplete(onboardingStatus?.steps, 'coach_seat');
  const { created: coachGroupCreated } = useCoachGroupCreated(user?.id);
  // « Continuer sans connecter », matching web: this session's tap, or the
  // connect step's skipped row it writes, so the screen never comes back.
  const { skipped: skippedProviderLocal } = useProviderSkipped(user?.id);
  const skippedProvider = skippedProviderLocal || isConnectProviderSkipped(onboardingStatus?.steps);
  // The steps put off, from the server's rows — which each screen writes into
  // the cached status as it skips (`useRecordOnboardingStep`).
  const skippedSteps = useMemo(() => skippedStepIds(onboardingStatus?.steps), [onboardingStatus?.steps]);
  const { done: connectProviderDone } = useConnectProviderDone(user?.id);
  const needsProviderConnection = onboardingStatus?.needs_provider_connection;
  // Server-only: the status read records it the first time a provider is
  // connected, so disconnecting later never reopens the wizard (carnet#1341).
  const connectProviderRecorded = isConnectProviderRecorded(onboardingStatus?.steps);
  // A coach who does not train is past the gate without connecting, so the
  // group flag loads (and is waited on) for them too.
  const postConnect =
    onboardingActive && pastProviderGate({ needsProviderConnection, connectProviderRecorded, athleteStepsWaived });

  // First-connect transition (needs true→false) — keeps the connect step up
  // until Continue, so a second service is one tap away.
  const [justOnboarded, setJustOnboarded] = useState(false);
  const prevNeedsProvider = useRef<boolean | undefined>(undefined);
  useEffect(() => {
    if (prevNeedsProvider.current === true && needsProviderConnection === false) {
      setJustOnboarded(true);
    }
    prevNeedsProvider.current = needsProviderConnection;
  }, [needsProviderConnection]);

  // Server truth on the session's user (carnet#1366). Only an explicit "never
  // answered" asks; an older server or cached user that does not say counts as
  // answered, so the question never lands on someone who already gave it.
  const analyticsConsentAnswered = user?.analytics_consent_answered !== false;

  const preConnectPending =
    beforeProviderGate({ needsProviderConnection, connectProviderRecorded }) &&
    (profileTypeChosen === undefined || athleteStepsWaivedLocal === undefined);
  // The PAR-Q comes after Tes données, connected or skipped, and before the
  // team: its flag and the team's are waited on past the gate and after a skip.
  const postConnectPending =
    (postConnect || (onboardingActive && skippedProvider)) &&
    (coachProposalSeen === undefined || coachGroupDoneLocal === undefined || parqDone === undefined);
  // A coach who trains and skipped the connect step reaches the coach steps
  // before the gate, so their flags are waited on wherever the coach is.
  const coachPending =
    coachesOthers && (coachSeatDoneLocal === undefined || coachGroupDoneLocal === undefined);

  const context = useMemo<OnboardingContext>(
    () => ({
      onboardingActive,
      needsProviderConnection,
      connectProviderRecorded,
      skippedProvider,
      skippedSteps,
      justOnboarded,
      connectProviderDone,
      profileTypeChosen: profileTypeChosen ?? true,
      parqDone: parqDone ?? true,
      athleteStepsWaived,
      coachesOthers,
      coachGroupDone,
      coachSeatDone: coachSeatDoneLocal !== false || coachSeatSettled,
      coachGroupCreated,
      coachProposalDone: coachProposalSeen ?? true,
      analyticsConsentAnswered,
    }),
    [
      onboardingActive,
      needsProviderConnection,
      connectProviderRecorded,
      skippedProvider,
      skippedSteps,
      justOnboarded,
      connectProviderDone,
      profileTypeChosen,
      parqDone,
      athleteStepsWaived,
      coachesOthers,
      coachGroupDone,
      coachSeatDoneLocal,
      coachSeatSettled,
      coachGroupCreated,
      coachProposalSeen,
      analyticsConsentAnswered,
    ],
  );

  return { context, settled: !preConnectPending && !postConnectPending && !coachPending };
}
