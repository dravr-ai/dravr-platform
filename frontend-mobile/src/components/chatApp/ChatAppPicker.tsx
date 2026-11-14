// ABOUTME: The chat-app choice inside the chat-app sheet (mobile) — which messaging app to talk to the agent on
// ABOUTME: Once the wizard's picker step; now the sheet's first view, shown only when the workspace offers more than one app

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React, { useState } from 'react';
import { View, Text, Pressable, ActivityIndicator } from 'react-native';
import * as Device from 'expo-device';
import type { AvailableChannel } from '@pierre/api-client';
import { useTranslation } from '@pierre/i18n';

/**
 * Messaging-channel picker (mobile), the counterpart of the web's. The sheet
 * opens on it only when more than one app is offered (a single app goes
 * straight to its link), and comes back to it from « Utiliser une autre app ».
 * Closing the sheet is the skip.
 */
export function ChatAppPicker({
  channels,
  onSelect,
}: {
  channels: AvailableChannel[];
  onSelect: (channel: string) => void;
}) {
  const { t } = useTranslation();
  const [choosing, setChoosing] = useState<string | null>(null);

  const handleSelect = (channel: string) => {
    if (choosing) return;
    setChoosing(channel);
    onSelect(channel);
  };

  return (
    <View testID="chat-app-picker">
      <Text className="text-sm text-on-surface-variant">{t('app.whereToChat')}</Text>
      <View className="mt-4 gap-3">
        {channels.map((channel) => (
          <ChannelCard
            key={channel.channel}
            channel={channel}
            busy={choosing === channel.channel}
            disabled={choosing !== null}
            onSelect={() => handleSelect(channel.channel)}
          />
        ))}
      </View>
    </View>
  );
}

function ChannelCard({
  channel,
  busy,
  disabled,
  onSelect,
}: {
  channel: AvailableChannel;
  busy: boolean;
  disabled: boolean;
  onSelect: () => void;
}) {
  const { t } = useTranslation();
  // On a phone a deep link is one tap: there is no QR to scan from the screen
  // in hand, so the hint offers one only where the link view shows it.
  const hint =
    channel.method === 'deep_link' && Device.deviceType !== Device.DeviceType.PHONE
      ? t('app.scanQrOrTap')
      : t('app.connectOneTap');
  return (
    <Pressable
      onPress={onSelect}
      disabled={disabled}
      accessibilityRole="button"
      accessibilityLabel={channel.display_name}
      testID={`chat-app-choose-${channel.channel}`}
      className="rounded-2xl border border-outline-variant bg-surface-container-low px-4 py-3 active:opacity-80"
      style={disabled ? { opacity: 0.6 } : undefined}
    >
      <View className="flex-row items-center gap-2">
        <Text className="text-base font-semibold text-on-surface">{channel.display_name}</Text>
        {channel.recommended ? (
          <Text className="rounded-full bg-primary/15 px-2 py-0.5 text-xs font-medium text-primary">
            {t('app.recommended')}
          </Text>
        ) : null}
      </View>
      <Text className="mt-1 text-sm text-on-surface-variant">{hint}</Text>
      {busy ? (
        <View className="mt-2 flex-row items-center gap-2">
          <ActivityIndicator size="small" />
          <Text className="text-xs text-on-surface-variant">{t('app.settingThingsUp')}</Text>
        </View>
      ) : null}
    </Pressable>
  );
}
