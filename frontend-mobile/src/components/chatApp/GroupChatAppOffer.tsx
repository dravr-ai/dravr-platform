// ABOUTME: The group agent's one-time chat-app card in a coach's group thread (mobile) — « Écris-moi aussi depuis Telegram ? »
// ABOUTME: Configure opens the chat-app sheet; « Pas maintenant » is the onboarding record, so it holds on every device (carnet#1525)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React, { useState } from 'react';
import { Pressable, Text, View } from 'react-native';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import type { AvailableChannel, ChannelLink } from '@pierre/api-client';
import type { CoachingGroup } from '@pierre/shared-types';
import { useTranslation } from '@pierre/i18n';
import {
  CHAT_APP_OFFER_STEP_ID,
  QUERY_KEYS,
  chatAppOfferAnswered,
  offeredChatApp,
  withStepRecorded,
} from '@pierre/shared-constants';
import { useAuth } from '../../contexts/AuthContext';
import { useOnboardingStatus } from '../../hooks/useOnboardingStatus';
import { messagingApi, userApi } from '../../services/api';
import { AVAILABLE_CHANNELS_KEY } from './ChatAppSheet';

/** The onboarding status as the shared query caches it. */
type OnboardingStatus = Awaited<ReturnType<typeof userApi.getOnboardingStatus>>;

const NO_CHANNELS: AvailableChannel[] = [];

/** What the card needs once it is shown: the app it names, and « Pas maintenant ». */
export interface GroupChatAppOfferState {
  app: AvailableChannel;
  /** Store the answer, then hide the card. Rejects, card kept, when it was not stored. */
  decline: () => Promise<void>;
}

/**
 * Whether the group thread offers its coach a chat app, once — the web hook's
 * rule on the phone: only the coach who owns the group, only while the
 * workspace offers an app, the coach has none linked, and the offer was never
 * answered on any device (the `messaging_configure` onboarding record).
 */
export function useGroupChatAppOffer(group: CoachingGroup | null): GroupChatAppOfferState | null {
  const { user } = useAuth();
  const queryClient = useQueryClient();
  const status = useOnboardingStatus(group !== null);
  const candidate =
    group !== null &&
    user?.id === group.owner_id &&
    status.data?.coaches_others === true &&
    !chatAppOfferAnswered(status.data.steps);
  const channels = useQuery({
    queryKey: AVAILABLE_CHANNELS_KEY,
    queryFn: () => messagingApi.getAvailableChannels(),
    enabled: candidate,
    staleTime: 60_000,
  });
  const links = useQuery({
    queryKey: ['messaging-links'],
    queryFn: () => messagingApi.listLinks(),
    enabled: candidate,
  });
  const offered = Array.isArray(channels.data) ? channels.data : NO_CHANNELS;
  const linked: ChannelLink[] = Array.isArray(links.data) ? links.data : [];
  const app = offeredChatApp(offered);

  if (!candidate || app === undefined || links.data === undefined || linked.length > 0) return null;

  const decline = async () => {
    // Stored first: a card hidden on an answer that never landed would come
    // back on the next open, so a failure keeps it up with the error.
    await userApi.setOnboardingStep(CHAT_APP_OFFER_STEP_ID, 'skipped');
    const key = QUERY_KEYS.user.onboardingStatus();
    queryClient.setQueryData<OnboardingStatus>(key, (previous) =>
      previous
        ? { ...previous, steps: withStepRecorded(previous.steps, CHAT_APP_OFFER_STEP_ID, 'skipped') }
        : previous,
    );
    void queryClient.invalidateQueries({ queryKey: key });
  };

  return { app, decline };
}

interface GroupChatAppOfferProps {
  offer: GroupChatAppOfferState;
  /** The group's agent as the thread names it: the card speaks in its voice. */
  agentLabel: string;
  /** The thread's last row is the agent's, so the card continues that run without its name again. */
  continuesRun?: boolean;
  /** « Configurer <app> »: open the chat-app sheet on that app. */
  onConfigure: (channel: string) => void;
}

/**
 * The card at the end of the thread, under the agent's name: a question, one
 * tonal action and a quiet « Pas maintenant » — an offer the coach may not
 * want, so nothing here is a filled button. Both controls are 44 high.
 */
export function GroupChatAppOffer({ offer, agentLabel, continuesRun = false, onConfigure }: GroupChatAppOfferProps) {
  const { t } = useTranslation();
  const [failed, setFailed] = useState(false);
  const [declining, setDeclining] = useState(false);
  const app = offer.app.display_name;

  const decline = () => {
    setFailed(false);
    setDeclining(true);
    offer
      .decline()
      .catch(() => setFailed(true))
      .finally(() => setDeclining(false));
  };

  return (
    <View className="mt-3 items-start" testID="group-chat-app-offer">
      {continuesRun ? null : <Text className="mb-1 text-xs font-semibold text-text-secondary">{agentLabel}</Text>}
      <Text className="text-base font-medium leading-6 text-text-primary">{t('chatApp.offer.title', { app })}</Text>
      <Text className="mt-1 text-sm leading-5 text-text-secondary">{t('chatApp.offer.body', { app })}</Text>
      <View className="mt-2 flex-row flex-wrap items-center gap-4">
        <Pressable
          onPress={() => onConfigure(offer.app.channel)}
          disabled={declining}
          accessibilityRole="button"
          accessibilityState={{ disabled: declining }}
          testID="group-chat-app-offer-configure"
          className={`min-h-11 justify-center rounded-lg bg-primary/15 px-3 ${declining ? 'opacity-50' : ''}`}
        >
          <Text className="text-sm font-medium text-primary">{t('chatApp.offer.configure', { app })}</Text>
        </Pressable>
        <Pressable
          onPress={decline}
          disabled={declining}
          accessibilityRole="button"
          accessibilityState={{ disabled: declining }}
          testID="group-chat-app-offer-not-now"
          className={`min-h-11 justify-center px-1 ${declining ? 'opacity-50' : ''}`}
        >
          <Text className="text-sm font-medium text-text-secondary">{t('chatApp.offer.notNow')}</Text>
        </Pressable>
      </View>
      {failed ? (
        <Text accessibilityRole="alert" className="mt-2 text-sm text-error">
          {t('chatApp.offer.notNowFailed')}
        </Text>
      ) : null}
    </View>
  );
}
