// ABOUTME: Encapsulates onboarding flow state — server status query + per-step localStorage/session flags + transitions
// ABOUTME: Called from AppContent (mounted across the OAuth-callback screen); the agent step reads server state, not that transition

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useCallback, useEffect, useRef, useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import { userApi } from '../services/api';
import type { OnboardingStatusResponse } from '@pierre/api-client';
import { useAuth } from './useAuth';
import { useOnboardingStatus } from './useOnboardingStatus';
import {
  ATHLETE_STEP_IDS,
  athleteStepsWaived as serverAthleteStepsWaived,
  isConnectProviderRecorded,
  isConnectProviderSkipped,
  isServerStepComplete,
  skippedStepIds,
  withStepRecorded,
  type OnboardingContext,
} from '../onboarding/steps';

/** What the Your team step writes: Start (`complete`) or Skip (`skipped`, the lead alone). */
export interface TeamStepChoice {
  status: 'complete' | 'skipped';
  /** The agent that leads the athlete's thread. */
  leadAgentId?: string;
  /** Every agent joining the team, the lead included. */
  teamAgentIds?: string[];
}

/** The onboarding context plus the per-step completion callbacks the flow wires into each step. */
export interface OnboardingState {
  ctx: OnboardingContext;
  /**
   * Store the answer to the usage-analytics question and wait for it. Not
   * best-effort: an unsaved answer would be asked again, so a failure
   * rejects and the step keeps the person on screen to retry.
   */
  answerAnalyticsConsent: (enabled: boolean) => Promise<void>;
  /**
   * Athlete/coach choice made — persist + advance. `trains: false` (a coach who
   * does not train) takes the athlete steps out of the journey.
   */
  completeProfileType: (choice: { trains: boolean }) => void;
  /** PAR-Q answered or skipped — persist + advance. */
  completeParq: (status?: 'complete' | 'skipped') => void;
  /**
   * Write the Your team step and wait for it. The server joins every agent in
   * `teamAgentIds` to the team, makes `leadAgentId` the selected agent and
   * binds the personal thread to it — so a failure rejects, and the step keeps
   * the athlete on screen to retry rather than reaching Home with no team.
   * A step written with neither id (no proposal loaded) changes no agent.
   */
  persistCoachProposal: (choice: TeamStepChoice) => Promise<void>;
  /** Advance past the Your team step (local flag only; the write is {@link persistCoachProposal}). */
  completeCoachProposal: () => void;
  /** The coach's group step finished (`complete`) or put off (`skipped`) — persist + advance. */
  completeCoachGroup: (status: 'complete' | 'skipped') => void;
  /** The coach moved past the coach-seat step (Continue or « Passer pour l'instant »). */
  completeCoachSeat: () => void;
  /** The coach's group was just made: the invite is the next screen. */
  markCoachGroupCreated: () => void;
  /**
   * A team-issued coach-seat invitation settled the role question and the
   * seat before onboarding began (server truth: `settled_by_invite`): the bar
   * says so, and the group step says the seat is already active.
   */
  settledByInvite: boolean;
  /** Continue past the connect step once a first provider is connected this session. */
  completeConnectProvider: () => void;
  /**
   * « Continuer sans connecter »: advances at once and saves the skip as the
   * connect step's `skipped` row, so Connect never comes back at sign-in.
   */
  skipProvider: () => void;
}

/** What the hook's host knows about how the user arrived. */
export interface OnboardingArrival {
  /**
   * The user opened a group invite link — a coach inviting an athlete — so
   * the role is recorded as athlete for them and Profil is behind them.
   */
  invitedAsAthlete?: boolean;
}

/**
 * Owns the onboarding gate state that previously lived inline in `AppContent`.
 *
 * Called from a component that stays mounted across the OAuth-callback screen
 * (i.e. `AppContent`, above its `oauthCallback` early return), so the tab that
 * started the connection observes `needs_provider_connection` flip true→false
 * (`justOnboarded`) and keeps the connect step up for a second service. The
 * coach-proposal step does not depend on that flip: it is derived from the
 * server status, so a fresh tab after the OAuth return meets it too
 * (carnet#1042).
 */
export function useOnboardingState({ invitedAsAthlete = false }: OnboardingArrival = {}): OnboardingState {
  const { user, isAuthenticated, updateUser } = useAuth();

  // Admins are exempt: their primary intent is administering, not chatting, so
  // a missing provider on the admin account must not block them from the
  // dashboard. The backend 403 still applies if an admin tries to chat without
  // a provider. (See `feedback_spa_gate_on_role_not_is_admin`: gate on role.)
  const isAdminRole = user?.role === 'admin' || user?.role === 'super_admin';
  const onboardingActive = Boolean(
    isAuthenticated && user?.user_status === 'active' && !isAdminRole,
  );

  const { data: onboardingStatus } = useOnboardingStatus(onboardingActive);

  // "Continue without connecting", as tapped in this tab. The tap also writes
  // the connect step `skipped`, which carries it to every later sign-in and
  // device; Home's "Connect your data" row is the nudge from here on.
  const [skippedProviderLocal, setSkippedProviderLocal] = useState(false);
  // Session-only: the user connected a first provider and pressed Continue. The
  // connect step stays up until then so a second service is one tap away.
  const [connectProviderDone, setConnectProviderDone] = useState(false);

  // Coach-proposal completion, persisted per-user in localStorage as a fast,
  // offline-capable flag alongside the durable server-side step record.
  const coachProposalKey = user?.user_id ? `dravr.coach_proposal_done.${user.user_id}` : null;
  // Read on the first render too: the step is gated on server state alone, so
  // a cached status must not flash it for one frame before the effect runs.
  const [coachProposalDoneLocal, setCoachProposalDoneLocal] = useState(() =>
    coachProposalKey ? localStorage.getItem(coachProposalKey) === '1' : true,
  );
  useEffect(() => {
    setCoachProposalDoneLocal(
      coachProposalKey ? localStorage.getItem(coachProposalKey) === '1' : true,
    );
  }, [coachProposalKey]);

  // PAR-Q completion. Same shape as profile-type: a local flag for instant
  // advance, the durable server record for cross-device truth.
  const parqKey = user?.user_id ? `dravr.parq_done.${user.user_id}` : null;
  const [parqDoneLocal, setParqDoneLocal] = useState(false);
  useEffect(() => {
    setParqDoneLocal(parqKey ? localStorage.getItem(parqKey) === '1' : true);
  }, [parqKey]);

  // Profile-type completion, persisted per-user so the athlete-vs-coach step
  // shows once within the first-run connect flow.
  const profileTypeKey = user?.user_id ? `dravr.profile_type_chosen.${user.user_id}` : null;
  const [profileTypeChosenLocal, setProfileTypeChosenLocal] = useState(false);
  useEffect(() => {
    setProfileTypeChosenLocal(profileTypeKey ? localStorage.getItem(profileTypeKey) === '1' : true);
  }, [profileTypeKey]);

  // The coach's group step, same local-plus-server shape as the steps above.
  const coachGroupKey = user?.user_id ? `dravr.coach_group_done.${user.user_id}` : null;
  const [coachGroupDoneLocal, setCoachGroupDoneLocal] = useState(false);
  useEffect(() => {
    setCoachGroupDoneLocal(coachGroupKey ? localStorage.getItem(coachGroupKey) === '1' : true);
  }, [coachGroupKey]);

  // The coach-seat step, a device flag like the others: the seat itself is
  // server state (`manages_roster`, the coach-access request) the step reads.
  const coachSeatKey = user?.user_id ? `dravr.coach_seat_done.${user.user_id}` : null;
  // Read on the first render too, like the agent step's flag: a cached status
  // must not flash the step for one frame at a coach who is past it.
  const [coachSeatDoneLocal, setCoachSeatDoneLocal] = useState(() =>
    coachSeatKey ? localStorage.getItem(coachSeatKey) === '1' : false,
  );
  useEffect(() => {
    setCoachSeatDoneLocal(coachSeatKey ? localStorage.getItem(coachSeatKey) === '1' : true);
  }, [coachSeatKey]);
  // The group made this session: its invite, which only this session holds
  // the link for, is the next screen.
  const [coachGroupCreated, setCoachGroupCreated] = useState(false);

  // A coach who does not train: the athlete steps leave the journey. Unlike the
  // done-flags above this one defaults to false with no user — it removes
  // steps, so failing open would skip the PAR-Q for an athlete.
  const athleteStepsWaivedKey = user?.user_id
    ? `dravr.athlete_steps_waived.${user.user_id}`
    : null;
  const [athleteStepsWaivedLocal, setAthleteStepsWaivedLocal] = useState(false);
  useEffect(() => {
    setAthleteStepsWaivedLocal(
      athleteStepsWaivedKey ? localStorage.getItem(athleteStepsWaivedKey) === '1' : false,
    );
  }, [athleteStepsWaivedKey]);

  // A step counts as done when EITHER the local flag or the durable server
  // record says so. The server record is what makes onboarding survive device
  // changes / cache clears (a step done on desktop is skipped on mobile). The OR
  // is monotonic toward "done", so it never re-shows a locally-completed step.
  const serverSteps = onboardingStatus?.steps;
  const profileTypeChosen =
    profileTypeChosenLocal || isServerStepComplete(serverSteps, 'profile_type');
  const coachProposalDone =
    coachProposalDoneLocal || isServerStepComplete(serverSteps, 'coach_proposal');
  const parqDone = parqDoneLocal || isServerStepComplete(serverSteps, 'parq');
  const skippedProvider = skippedProviderLocal || isConnectProviderSkipped(serverSteps);
  const athleteStepsWaived = athleteStepsWaivedLocal || serverAthleteStepsWaived(serverSteps);
  const coachGroupDone = coachGroupDoneLocal || isServerStepComplete(serverSteps, 'coach_group');
  // The seat step's server row exists only where a coach-seat invitation
  // settled the seat before sign-in, so that coach never meets the step.
  const coachSeatDone = coachSeatDoneLocal || isServerStepComplete(serverSteps, 'coach_seat');
  const settledByInvite = onboardingStatus?.settled_by_invite === true;
  const coachesOthers = onboardingStatus?.coaches_others === true;
  // Server-only: the status read records it the first time a provider is
  // connected, so disconnecting later never reopens the wizard (carnet#1341).
  const connectProviderRecorded = isConnectProviderRecorded(serverSteps);

  // Detect the first-in-session provider connection (needs true→false), which
  // keeps the connect step on screen until Continue. The coach-proposal step is
  // gated on server state instead (see the shared registry).
  const [justOnboarded, setJustOnboarded] = useState(false);
  const prevNeedsProvider = useRef<boolean | undefined>(undefined);
  useEffect(() => {
    const now = onboardingStatus?.needs_provider_connection;
    if (prevNeedsProvider.current === true && now === false) {
      setJustOnboarded(true);
    }
    prevNeedsProvider.current = now;
  }, [onboardingStatus?.needs_provider_connection]);

  // The step row as the server will hold it, in the cached status at once: the
  // progress bar reads a skip from those rows, and the next status read would
  // otherwise come seconds after the user has moved on.
  const queryClient = useQueryClient();
  const recordStepInCache = useCallback(
    (stepId: string, status: 'complete' | 'skipped' | 'not_applicable') => {
      queryClient.setQueryData<OnboardingStatusResponse>(
        QUERY_KEYS.user.onboardingStatus(),
        (cached) => cached && { ...cached, steps: withStepRecorded(cached.steps, stepId, status) },
      );
    },
    [queryClient],
  );

  // Durability best-effort: the localStorage flag already advanced the flow, so a
  // failed server write is non-fatal (mirrors the coaching-persona write). The
  // server record is read on the user's next session / other device.
  const persistStep = useCallback(
    (stepId: string, status: 'complete' | 'skipped' | 'not_applicable') => {
      recordStepInCache(stepId, status);
      void userApi.setOnboardingStep(stepId, status).catch(() => {});
    },
    [recordStepInCache],
  );

  const completeProfileType = useCallback(
    ({ trains }: { trains: boolean }) => {
      if (!trains) {
        if (athleteStepsWaivedKey) localStorage.setItem(athleteStepsWaivedKey, '1');
        setAthleteStepsWaivedLocal(true);
        for (const stepId of ATHLETE_STEP_IDS) persistStep(stepId, 'not_applicable');
      }
      if (profileTypeKey) localStorage.setItem(profileTypeKey, '1');
      setProfileTypeChosenLocal(true);
      persistStep('profile_type', 'complete');
    },
    [athleteStepsWaivedKey, profileTypeKey, persistStep],
  );

  // The session's user is what App boots analytics on, so a yes starts
  // PostHog at once; the answered flag closes the step on this surface.
  const answerAnalyticsConsent = useCallback(
    async (enabled: boolean) => {
      await userApi.updateAnalyticsConsent(enabled);
      updateUser({ analytics_consent: enabled, analytics_consent_answered: true });
    },
    [updateUser],
  );

  // Not best-effort like the other steps: this write is what puts agents on
  // the athlete's team, so its failure reaches the step.
  const persistCoachProposal = useCallback(
    async ({ status, leadAgentId, teamAgentIds }: TeamStepChoice) => {
      await userApi.setOnboardingStep('coach_proposal', status, {
        agentId: leadAgentId,
        teamAgentIds,
      });
      recordStepInCache('coach_proposal', status);
    },
    [recordStepInCache],
  );

  const completeCoachProposal = useCallback(() => {
    if (coachProposalKey) localStorage.setItem(coachProposalKey, '1');
    setCoachProposalDoneLocal(true);
    setJustOnboarded(false);
  }, [coachProposalKey]);

  const completeCoachGroup = useCallback(
    (status: 'complete' | 'skipped') => {
      if (coachGroupKey) localStorage.setItem(coachGroupKey, '1');
      setCoachGroupDoneLocal(true);
      persistStep('coach_group', status);
    },
    [coachGroupKey, persistStep],
  );

  const completeCoachSeat = useCallback(() => {
    if (coachSeatKey) localStorage.setItem(coachSeatKey, '1');
    setCoachSeatDoneLocal(true);
  }, [coachSeatKey]);

  const markCoachGroupCreated = useCallback(() => setCoachGroupCreated(true), []);

  // An invite link records the role it implies — athlete — as the role
  // question's answer would, once, so the wizard opens past Profil and the bar
  // shows Profil done because the role is recorded, not by a case of its own.
  // Best-effort like the other step writes: a new account already reads as an
  // athlete, and the role stays changeable in Settings.
  const inviteRoleRecorded = useRef(false);
  const statusLoaded = onboardingStatus !== undefined;
  useEffect(() => {
    if (!invitedAsAthlete || !onboardingActive || !statusLoaded || inviteRoleRecorded.current) return;
    if (profileTypeChosen || coachesOthers || athleteStepsWaived) return;
    inviteRoleRecorded.current = true;
    void userApi.setCoachingRole(false).catch(() => {});
    completeProfileType({ trains: true });
  }, [
    invitedAsAthlete,
    onboardingActive,
    statusLoaded,
    profileTypeChosen,
    coachesOthers,
    athleteStepsWaived,
    completeProfileType,
  ]);

  const completeParq = useCallback(
    (status: 'complete' | 'skipped' = 'complete') => {
      if (parqKey) localStorage.setItem(parqKey, '1');
      setParqDoneLocal(true);
      persistStep('parq', status);
    },
    [parqKey, persistStep],
  );

  const skipProvider = useCallback(() => {
    setSkippedProviderLocal(true);
    persistStep('connect_provider', 'skipped');
  }, [persistStep]);
  const completeConnectProvider = useCallback(() => setConnectProviderDone(true), []);

  const ctx: OnboardingContext = {
    onboardingActive,
    needsProviderConnection: onboardingStatus?.needs_provider_connection,
    connectProviderRecorded,
    skippedProvider,
    skippedSteps: skippedStepIds(serverSteps),
    justOnboarded,
    connectProviderDone,
    profileTypeChosen,
    coachProposalDone,
    parqDone,
    athleteStepsWaived,
    coachesOthers,
    coachGroupDone,
    coachSeatDone,
    coachGroupCreated,
    analyticsConsentAnswered: user?.analytics_consent_answered !== false,
  };

  return {
    ctx,
    answerAnalyticsConsent,
    completeProfileType,
    completeParq,
    persistCoachProposal,
    completeCoachProposal,
    completeCoachGroup,
    completeCoachSeat,
    markCoachGroupCreated,
    settledByInvite,
    completeConnectProvider,
    skipProvider,
  };
}
