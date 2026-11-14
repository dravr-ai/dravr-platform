// ABOUTME: The chat-app sheet (mobile) — pick Telegram, WhatsApp or another app the workspace offers, then link it
// ABOUTME: Opened from a coach's group card; the link landing closes it and says so — the web sheet's counterpart (carnet#1525)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React, { useCallback, useRef, useState } from 'react';
import { ActivityIndicator, ScrollView, Text, View } from 'react-native';
import Toast from 'react-native-toast-message';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import type { AvailableChannel } from '@pierre/api-client';
import { CHAT_APP_OFFER_STEP_ID, QUERY_KEYS } from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';
import { Sheet } from '../ui/Sheet';
import { messagingApi, userApi } from '../../services/api';
import { ChatAppPicker } from './ChatAppPicker';
import { ChatAppConnect } from './ChatAppConnect';

/** The query the sheet and the group card share for what the workspace offers. */
export const AVAILABLE_CHANNELS_KEY = ['messaging-available-channels'] as const;

const EMPTY: AvailableChannel[] = [];

interface ChatAppSheetProps {
  visible: boolean;
  /** The app the offer named; without one the sheet opens on the choice, or on the only app. */
  initialChannel?: string;
  onClose: () => void;
}

/**
 * Link a chat app from the phone — the two screens that closed the onboarding
 * wizard, moved into a sheet. The choice comes first when the workspace offers
 * more than one app; one app goes straight to its link, and « Utiliser une
 * autre app » goes back to the choice. When the link lands the sheet closes
 * with a toast and the offer's record is written `complete`, so the group card
 * does not come back. Closing the sheet answers nothing.
 */
export function ChatAppSheet({ visible, initialChannel, onClose }: ChatAppSheetProps) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const { data, isLoading } = useQuery({
    queryKey: AVAILABLE_CHANNELS_KEY,
    queryFn: () => messagingApi.getAvailableChannels(),
    enabled: visible,
    staleTime: 0,
  });
  const channels = Array.isArray(data) ? data : EMPTY;
  const [picked, setPicked] = useState<string | null>(initialChannel ?? null);
  const linkedOnce = useRef(false);

  // An app the workspace stopped offering since the card was drawn falls back
  // to the choice rather than to a link that cannot start.
  const only = channels.length === 1 ? channels[0].channel : null;
  const chosen = channels.some((c) => c.channel === picked) ? picked : only;
  const displayName = channels.find((c) => c.channel === chosen)?.display_name ?? chosen ?? '';

  const onLinked = useCallback(() => {
    // The poll can report the same landed link on more than one render.
    if (linkedOnce.current) return;
    linkedOnce.current = true;
    userApi.setOnboardingStep(CHAT_APP_OFFER_STEP_ID, 'complete').catch(() => {});
    void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.user.onboardingStatus() });
    Toast.show({ type: 'success', text1: t('chatApp.linkedToast', { app: displayName }) });
    onClose();
  }, [displayName, onClose, queryClient, t]);

  let body: React.ReactNode;
  if (isLoading) {
    body = (
      <View className="items-center py-10">
        <ActivityIndicator size="large" />
      </View>
    );
  } else if (channels.length === 0) {
    body = (
      <Text className="py-6 text-center text-sm text-on-surface-variant" testID="chat-app-none">
        {t('app.noChatAppsConfigured')}
      </Text>
    );
  } else if (chosen === null) {
    body = <ChatAppPicker channels={channels} onSelect={setPicked} />;
  } else {
    body = (
      <ChatAppConnect
        key={chosen}
        channel={chosen}
        displayName={displayName}
        onLinked={onLinked}
        onUseAnother={channels.length > 1 ? () => setPicked(null) : undefined}
      />
    );
  }

  return (
    <Sheet visible={visible} onClose={onClose} testID="chat-app-sheet">
      <Text className="mb-4 text-lg font-semibold text-on-surface" accessibilityRole="header">
        {chosen ? t('app.connectChannelTitle', { channel: displayName }) : t('chatApp.pickTitle')}
      </Text>
      <ScrollView showsVerticalScrollIndicator={false}>{visible ? body : null}</ScrollView>
    </Sheet>
  );
}
