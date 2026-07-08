// ABOUTME: Encapsulates onboarding flow state — server status query + per-step localStorage/session flags + transitions
// ABOUTME: Called from AppContent (mounted across the OAuth-callback screen) so the needs→false transition is never missed

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useCallback, useEffect, useRef, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { QUERY_KEYS } from '../constants/queryKeys';
import { userApi, messagingLinkApi } from '../services/api';
import type { AvailableChannel } from '@pierre/api-client';
import { useAuth } from './useAuth';
import { isServerStepComplete, type OnboardingContext } from '../onboarding/steps';

/** Stable empty list so the channels query's default `data` keeps identity across renders. */
const EMPTY_CHANNELS: AvailableChannel[] = [];

/** The onboarding context plus the per-step completion callbacks the flow wires into each step. */
export interface OnboardingState {
  ctx: OnboardingContext;
  /** Athlete/coach choice made — persist + advance. */
  completeProfileType: () => void;
  /** Coach proposal dismissed/accepted — persist + advance to the dashboard. */
  completeCoachProposal: () => void;
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
 * Must be called from a component that stays mounted across the OAuth-callback
 * screen (i.e. `AppContent`, above its `oauthCallback` early return): the
 * `justOnboarded` transition is detected by observing `needs_provider_connection`
 * flip true→false while the OAuth round-trip completes, so the query and the
 * `prevNeedsProvider` ref must keep running the whole time.
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

  const { data: onboardingStatus } = useQuery({
    queryKey: QUERY_KEYS.user.onboardingStatus(),
    queryFn: () => userApi.getOnboardingStatus(),
    enabled: onboardingActive,
    // Keep this fresh during the OAuth round-trip so the redirect flips as soon
    // as the provider connection lands.
    staleTime: 5_000,
  });

  // The messaging steps live in the just-connected phase, so only fetch the
  // connectable-channel list once the user is past the provider gate.
  const postConnect = onboardingActive && onboardingStatus?.needs_provider_connection === false;
  const { data: availableChannelsData } = useQuery({
    queryKey: ['messaging-available-channels'],
    queryFn: () => messagingLinkApi.getAvailableChannels(),
    enabled: postConnect,
    staleTime: 60_000,
  });
  // Defend against a non-array response (e.g. an error/HTML page): the messaging
  // steps must never intercept the flow on a malformed channels read — a string's
  // `.length` would otherwise read as a huge channel count.
  const availableChannels = Array.isArray(availableChannelsData)
    ? availableChannelsData
    : EMPTY_CHANNELS;

  // Session-only "continue without a provider": lets the user into the app
  // without connecting. Deliberately not persisted — the connect prompts in
  // chat and on the coach screens carry the nudge from here on.
  const [skippedProvider, setSkippedProvider] = useState(false);
  // Session "skip messaging" escape (from either messaging step) → dashboard.
  const [skippedMessaging, setSkippedMessaging] = useState(false);
  // The messaging channel picked this session, before it's server-persisted.
  const [chosenChannelLocal, setChosenChannelLocal] = useState<string | null>(null);
  // The messaging-configure step finished this session (link landed or skipped).
  const [messagingConfigureDoneLocal, setMessagingConfigureDoneLocal] = useState(false);

  // Coach-proposal completion, persisted per-user in localStorage as a fast,
  // offline-capable flag alongside the durable server-side step record.
  const coachProposalKey = user?.user_id ? `dravr.coach_proposal_done.${user.user_id}` : null;
  const [coachProposalDoneLocal, setCoachProposalDoneLocal] = useState(false);
  useEffect(() => {
    setCoachProposalDoneLocal(
      coachProposalKey ? localStorage.getItem(coachProposalKey) === '1' : true,
    );
  }, [coachProposalKey]);

  // Profile-type completion, persisted per-user so the athlete-vs-coach step
  // shows once within the first-run connect flow.
  const profileTypeKey = user?.user_id ? `dravr.profile_type_chosen.${user.user_id}` : null;
  const [profileTypeChosenLocal, setProfileTypeChosenLocal] = useState(false);
  useEffect(() => {
    setProfileTypeChosenLocal(profileTypeKey ? localStorage.getItem(profileTypeKey) === '1' : true);
  }, [profileTypeKey]);

  // A step counts as done when EITHER the local flag or the durable server
  // record says so. The server record is what makes onboarding survive device
  // changes / cache clears (a step done on desktop is skipped on mobile). The OR
  // is monotonic toward "done", so it never re-shows a locally-completed step.
  const serverSteps = onboardingStatus?.steps;
  const profileTypeChosen =
    profileTypeChosenLocal || isServerStepComplete(serverSteps, 'profile_type');
  const coachProposalDone =
    coachProposalDoneLocal || isServerStepComplete(serverSteps, 'coach_proposal');

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

  // Detect the first-in-session provider connection (needs true→false) so the
  // coach-proposal step fires only right after the user connects, never on a
  // plain login by an already-onboarded user.
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
    (stepId: string, status: 'complete' | 'skipped', chosenChannel?: string) => {
      void userApi.setOnboardingStep(stepId, status, chosenChannel).catch(() => {});
    },
    [],
  );

  const completeProfileType = useCallback(() => {
    if (profileTypeKey) localStorage.setItem(profileTypeKey, '1');
    setProfileTypeChosenLocal(true);
    persistStep('profile_type', 'complete');
  }, [profileTypeKey, persistStep]);

  const completeCoachProposal = useCallback(() => {
    if (coachProposalKey) localStorage.setItem(coachProposalKey, '1');
    setCoachProposalDoneLocal(true);
    setJustOnboarded(false);
    persistStep('coach_proposal', 'complete');
  }, [coachProposalKey, persistStep]);

  const skipProvider = useCallback(() => setSkippedProvider(true), []);

  const chooseChannel = useCallback(
    (channel: string) => {
      setChosenChannelLocal(channel);
      persistStep('messaging_channel', 'complete', channel);
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
    profileTypeChosen,
    coachProposalDone,
    messagingAvailableCount,
    messagingChannelChosen,
    messagingChannelDone,
    messagingConfigureDone,
  };

  return {
    ctx,
    completeProfileType,
    completeCoachProposal,
    skipProvider,
    availableChannels,
    chosenChannel,
    chooseChannel,
    completeMessagingConfigure,
    skipMessaging,
  };
}
