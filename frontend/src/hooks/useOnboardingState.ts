// ABOUTME: Encapsulates onboarding flow state — server status query + per-step localStorage/session flags + transitions
// ABOUTME: Called from AppContent (mounted across the OAuth-callback screen); the agent step reads server state, not that transition

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useCallback, useEffect, useRef, useState } from 'react';
import { userApi } from '../services/api';
import type { AvailableChannel } from '@pierre/api-client';
import { useAuth } from './useAuth';
import { useOnboardingStatus } from './useOnboardingStatus';
import { useAvailableChannels } from './useAvailableChannels';
import {
  ATHLETE_STEP_IDS,
  athleteStepsWaived as serverAthleteStepsWaived,
  isServerStepComplete,
  pastProviderGate,
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
   * Athlete/coach choice made — persist + advance. `trains: false` (a coach who
   * does not train) takes the athlete steps out of the journey.
   */
  completeProfileType: (choice: { trains: boolean }) => void;
  /** About-you answers submitted or skipped — persist + advance. */
  completeAboutYou: (status?: 'complete' | 'skipped') => void;
  /** PAR-Q answered or skipped — persist + advance. */
  completeParq: (status?: 'complete' | 'skipped') => void;
  /** The check-in cadence chosen (stored by the step) or skipped — persist + advance. */
  completeCheckIn: (status: 'complete' | 'skipped') => void;
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
  /** Continue past the connect step once a first provider is connected this session. */
  completeConnectProvider: () => void;
  /** Session-only "continue without connecting" — escapes the flow to the dashboard. */
  skipProvider: () => void;
  /** Messaging channels the tenant has configured (for the picker step). */
  availableChannels: AvailableChannel[];
  /** The chosen messaging channel id, or `null` if none picked yet. */
  chosenChannel: string | null;
  /** Pick a messaging channel — persist + advance to configure. */
  chooseChannel: (channel: string) => void;
  /** The messaging channel linked (or the configure step accepted) — persist + finish. */
  completeMessagingConfigure: () => void;
  /** Skip messaging setup entirely (either messaging step) — escapes to the dashboard. */
  skipMessaging: () => void;
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
export function useOnboardingState(): OnboardingState {
  const { user, isAuthenticated } = useAuth();

  // Admins are exempt: their primary intent is administering, not chatting, so
  // a missing provider on the admin account must not block them from the
  // dashboard. The backend 403 still applies if an admin tries to chat without
  // a provider. (See `feedback_spa_gate_on_role_not_is_admin`: gate on role.)
  const isAdminRole = user?.role === 'admin' || user?.role === 'super_admin';
  const onboardingActive = Boolean(
    isAuthenticated && user?.user_status === 'active' && !isAdminRole,
  );

  const { data: onboardingStatus } = useOnboardingStatus(onboardingActive);

  // Session-only "continue without a provider": lets the user into the app
  // without connecting. Deliberately not persisted — the connect prompts in
  // chat and on the agent screens carry the nudge from here on.
  const [skippedProvider, setSkippedProvider] = useState(false);
  // Session-only: the user connected a first provider and pressed Continue. The
  // connect step stays up until then so a second service is one tap away.
  const [connectProviderDone, setConnectProviderDone] = useState(false);
  // Session "skip messaging" escape (from either messaging step) → dashboard.
  const [skippedMessaging, setSkippedMessaging] = useState(false);
  // The messaging channel picked this session, before it's server-persisted.
  const [chosenChannelLocal, setChosenChannelLocal] = useState<string | null>(null);
  // The messaging-configure step finished this session (link landed or skipped).
  const [messagingConfigureDoneLocal, setMessagingConfigureDoneLocal] = useState(false);

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

  // About-you and PAR-Q completion. Same shape as profile-type: a local flag for
  // instant advance, the durable server record for cross-device truth.
  const aboutYouKey = user?.user_id ? `dravr.about_you_done.${user.user_id}` : null;
  const [aboutYouDoneLocal, setAboutYouDoneLocal] = useState(false);
  useEffect(() => {
    setAboutYouDoneLocal(aboutYouKey ? localStorage.getItem(aboutYouKey) === '1' : true);
  }, [aboutYouKey]);

  const parqKey = user?.user_id ? `dravr.parq_done.${user.user_id}` : null;
  const [parqDoneLocal, setParqDoneLocal] = useState(false);
  useEffect(() => {
    setParqDoneLocal(parqKey ? localStorage.getItem(parqKey) === '1' : true);
  }, [parqKey]);

  // The check-in cadence question, the same local-plus-server shape: the
  // step stores the cadence itself, this records that it was answered.
  const checkInKey = user?.user_id ? `dravr.check_in_done.${user.user_id}` : null;
  const [checkInDoneLocal, setCheckInDoneLocal] = useState(false);
  useEffect(() => {
    setCheckInDoneLocal(checkInKey ? localStorage.getItem(checkInKey) === '1' : true);
  }, [checkInKey]);

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
  const aboutYouDone = aboutYouDoneLocal || isServerStepComplete(serverSteps, 'about_you');
  const parqDone = parqDoneLocal || isServerStepComplete(serverSteps, 'parq');
  const checkInDone = checkInDoneLocal || isServerStepComplete(serverSteps, 'check_in');
  const athleteStepsWaived = athleteStepsWaivedLocal || serverAthleteStepsWaived(serverSteps);
  const coachGroupDone = coachGroupDoneLocal || isServerStepComplete(serverSteps, 'coach_group');
  const coachesOthers = onboardingStatus?.coaches_others === true;

  // The messaging steps live in the just-connected phase, so only fetch the
  // connectable-channel list once the user is past the provider gate — which a
  // coach who does not train is without connecting.
  const postConnect =
    onboardingActive &&
    pastProviderGate({
      needsProviderConnection: onboardingStatus?.needs_provider_connection,
      athleteStepsWaived,
    });
  const { channels: availableChannels } = useAvailableChannels({ enabled: postConnect });

  // Messaging derivations. The chosen channel is the one picked this session,
  // else the server-persisted `chosen_channel`, else the sole configured channel
  // (so a single-channel tenant auto-selects and skips the picker).
  const messagingAvailableCount = availableChannels.length;
  const chosenChannel =
    chosenChannelLocal ??
    onboardingStatus?.chosen_channel ??
    (messagingAvailableCount === 1 ? (availableChannels[0]?.channel ?? null) : null);
  const messagingChannelChosen = chosenChannel !== null;
  const messagingChannelDone =
    messagingChannelChosen ||
    skippedMessaging ||
    isServerStepComplete(serverSteps, 'messaging_channel');
  const messagingConfigureDone =
    messagingConfigureDoneLocal ||
    skippedMessaging ||
    isServerStepComplete(serverSteps, 'messaging_configure');

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

  // Durability best-effort: the localStorage flag already advanced the flow, so a
  // failed server write is non-fatal (mirrors the coaching-persona write). The
  // server record is read on the user's next session / other device.
  const persistStep = useCallback(
    (
      stepId: string,
      status: 'complete' | 'skipped' | 'not_applicable',
      options?: { chosenChannel?: string; agentId?: string },
    ) => {
      void userApi.setOnboardingStep(stepId, status, options).catch(() => {});
    },
    [],
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

  // Not best-effort like the other steps: this write is what puts agents on
  // the athlete's team, so its failure reaches the step.
  const persistCoachProposal = useCallback(
    ({ status, leadAgentId, teamAgentIds }: TeamStepChoice) =>
      userApi.setOnboardingStep('coach_proposal', status, {
        agentId: leadAgentId,
        teamAgentIds,
      }),
    [],
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

  const completeAboutYou = useCallback(
    (status: 'complete' | 'skipped' = 'complete') => {
      if (aboutYouKey) localStorage.setItem(aboutYouKey, '1');
      setAboutYouDoneLocal(true);
      persistStep('about_you', status);
    },
    [aboutYouKey, persistStep],
  );

  const completeParq = useCallback(
    (status: 'complete' | 'skipped' = 'complete') => {
      if (parqKey) localStorage.setItem(parqKey, '1');
      setParqDoneLocal(true);
      persistStep('parq', status);
    },
    [parqKey, persistStep],
  );

  const completeCheckIn = useCallback(
    (status: 'complete' | 'skipped') => {
      if (checkInKey) localStorage.setItem(checkInKey, '1');
      setCheckInDoneLocal(true);
      persistStep('check_in', status);
    },
    [checkInKey, persistStep],
  );

  const skipProvider = useCallback(() => setSkippedProvider(true), []);
  const completeConnectProvider = useCallback(() => setConnectProviderDone(true), []);

  const chooseChannel = useCallback(
    (channel: string) => {
      setChosenChannelLocal(channel);
      persistStep('messaging_channel', 'complete', { chosenChannel: channel });
    },
    [persistStep],
  );

  const completeMessagingConfigure = useCallback(() => {
    setMessagingConfigureDoneLocal(true);
    persistStep('messaging_configure', 'complete');
  }, [persistStep]);

  const skipMessaging = useCallback(() => {
    setSkippedMessaging(true);
    persistStep('messaging_channel', 'skipped');
    persistStep('messaging_configure', 'skipped');
  }, [persistStep]);

  const ctx: OnboardingContext = {
    onboardingActive,
    needsProviderConnection: onboardingStatus?.needs_provider_connection,
    skippedProvider,
    justOnboarded,
    connectProviderDone,
    profileTypeChosen,
    coachProposalDone,
    aboutYouDone,
    parqDone,
    checkInDone,
    athleteStepsWaived,
    coachesOthers,
    coachGroupDone,
    messagingAvailableCount,
    messagingChannelChosen,
    messagingChannelDone,
    messagingConfigureDone,
  };

  return {
    ctx,
    completeProfileType,
    completeAboutYou,
    completeParq,
    completeCheckIn,
    persistCoachProposal,
    completeCoachProposal,
    completeCoachGroup,
    completeConnectProvider,
    skipProvider,
    availableChannels,
    chosenChannel,
    chooseChannel,
    completeMessagingConfigure,
    skipMessaging,
  };
}
