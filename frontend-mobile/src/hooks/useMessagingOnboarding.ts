// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Messaging onboarding state (mobile) — available channels, chosen channel, done flags + persist callbacks
// ABOUTME: AsyncStorage-cached flags (like useCoachProposalSeen) + durable server steps, shared by the gate + screens

import { useCallback } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import AsyncStorage from '@react-native-async-storage/async-storage';
import type { AvailableChannel } from '@pierre/api-client';
import { isServerStepComplete } from '@pierre/shared-constants';
import { messagingApi, userApi } from '../services/api';
import { useOnboardingStatus } from './useOnboardingStatus';

const EMPTY: AvailableChannel[] = [];
const chosenKey = (userId: string) => `dravr.messaging_channel.${userId}`;
const doneKey = (userId: string) => `dravr.messaging_configure_done.${userId}`;
const SKIP = 'skip';

/** Messaging onboarding state + actions, shared by the gate and the messaging screens. */
export interface MessagingOnboardingState {
  availableChannels: AvailableChannel[];
  availableCount: number;
  chosenChannel: string | null;
  channelChosen: boolean;
  channelDone: boolean;
  configureDone: boolean;
  /** True while any flag needed to decide the messaging steps is still loading. */
  loading: boolean;
  /** Pick a channel — persist locally + server, advancing to configure. */
  chooseChannel: (channel: string) => Promise<void>;
  /** The channel linked (or configure accepted) — persist + finish. */
  completeConfigure: () => Promise<void>;
  /** Skip messaging setup entirely — persist skip for both steps. */
  skipMessaging: () => Promise<void>;
}

/**
 * Owns the messaging onboarding state. `enabled` should be true only in the
 * post-connect phase (so the channel list isn't fetched during the provider
 * gate). Mirrors the web `useOnboardingState` messaging logic: the chosen channel
 * is the local pick, else the server-persisted `chosen_channel`, else the sole
 * configured channel; completion is local-flag OR skipped OR the durable server
 * step. Fails open so a hiccup never traps the user on a messaging step.
 */
export function useMessagingOnboarding(
  userId: string | undefined,
  enabled: boolean,
): MessagingOnboardingState {
  const queryClient = useQueryClient();
  const { data: onboardingStatus } = useOnboardingStatus(enabled);
  const serverSteps = onboardingStatus?.steps;
  const serverChosen = onboardingStatus?.chosen_channel ?? null;

  const { data: channelsData, isLoading: channelsLoading } = useQuery({
    queryKey: ['messaging-available-channels'],
    queryFn: () => messagingApi.getAvailableChannels(),
    enabled,
    staleTime: 60_000,
  });
  const availableChannels = Array.isArray(channelsData) ? channelsData : EMPTY;
  const availableCount = availableChannels.length;

  const { data: localChosen } = useQuery({
    queryKey: ['messaging-channel-chosen', userId],
    queryFn: async () => {
      if (!userId) return null;
      try {
        return (await AsyncStorage.getItem(chosenKey(userId))) ?? null;
      } catch {
        return null;
      }
    },
    staleTime: Infinity,
  });

  const { data: localConfigureDone } = useQuery({
    queryKey: ['messaging-configure-done', userId],
    queryFn: async () => {
      if (!userId) return true;
      try {
        return (await AsyncStorage.getItem(doneKey(userId))) === '1';
      } catch {
        return true;
      }
    },
    staleTime: Infinity,
  });

  const skipped = localChosen === SKIP;
  const chosenChannel = skipped
    ? null
    : localChosen && localChosen !== SKIP
      ? localChosen
      : (serverChosen ?? (availableCount === 1 ? (availableChannels[0]?.channel ?? null) : null));
  const channelChosen = chosenChannel !== null;
  const channelDone =
    channelChosen || skipped || isServerStepComplete(serverSteps, 'messaging_channel');
  const configureDone =
    localConfigureDone === true || skipped || isServerStepComplete(serverSteps, 'messaging_configure');

  // Only "loading" when the reads that could change the messaging decision are in
  // flight; when disabled (pre-connect) the messaging steps are irrelevant.
  const loading =
    enabled && (channelsLoading || localChosen === undefined || localConfigureDone === undefined);

  const persist = useCallback(
    (stepId: 'messaging_channel' | 'messaging_configure', status: 'complete' | 'skipped', channel?: string) => {
      void userApi.setOnboardingStep(stepId, status, channel).catch(() => {});
    },
    [],
  );

  const chooseChannel = useCallback(
    async (channel: string) => {
      if (userId) {
        try {
          await AsyncStorage.setItem(chosenKey(userId), channel);
        } catch {
          // Non-fatal: the cache update below still advances the flow.
        }
      }
      queryClient.setQueryData(['messaging-channel-chosen', userId], channel);
      persist('messaging_channel', 'complete', channel);
    },
    [userId, queryClient, persist],
  );

  const completeConfigure = useCallback(async () => {
    if (userId) {
      try {
        await AsyncStorage.setItem(doneKey(userId), '1');
      } catch {
        // Non-fatal.
      }
    }
    queryClient.setQueryData(['messaging-configure-done', userId], true);
    persist('messaging_configure', 'complete');
  }, [userId, queryClient, persist]);

  const skipMessaging = useCallback(async () => {
    if (userId) {
      try {
        await AsyncStorage.setItem(chosenKey(userId), SKIP);
        await AsyncStorage.setItem(doneKey(userId), '1');
      } catch {
        // Non-fatal.
      }
    }
    queryClient.setQueryData(['messaging-channel-chosen', userId], SKIP);
    queryClient.setQueryData(['messaging-configure-done', userId], true);
    persist('messaging_channel', 'skipped');
    persist('messaging_configure', 'skipped');
  }, [userId, queryClient, persist]);

  return {
    availableChannels,
    availableCount,
    chosenChannel,
    channelChosen,
    channelDone,
    configureDone,
    loading,
    chooseChannel,
    completeConfigure,
    skipMessaging,
  };
}
