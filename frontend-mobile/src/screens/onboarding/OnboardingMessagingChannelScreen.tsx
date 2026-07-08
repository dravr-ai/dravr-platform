// ABOUTME: Onboarding step (mobile) — pick which messaging app to chat with the coach on (Telegram/WhatsApp/…)
// ABOUTME: Mirrors the web OnboardingMessagingChannel; advances via the shared useMessagingOnboarding cache

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React, { useState } from 'react';
import { View, Text, ScrollView, Pressable, ActivityIndicator } from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import type { AvailableChannel } from '@pierre/api-client';
import { Card, Button } from '../../components/ui';
import { useAuth } from '../../contexts/AuthContext';
import { useMessagingOnboarding } from '../../hooks/useMessagingOnboarding';

/**
 * Messaging-channel picker (mobile). Reached post-connect when the tenant has
 * more than one channel configured; picking one persists the choice and flips the
 * shared cache so RootLayoutNav routes on to the configure step.
 */
export function OnboardingMessagingChannelScreen() {
  const { user } = useAuth();
  const messaging = useMessagingOnboarding(user?.id, true);
  const [choosing, setChoosing] = useState<string | null>(null);

  const handleSelect = async (channel: string) => {
    if (choosing) return;
    setChoosing(channel);
    await messaging.chooseChannel(channel);
  };

  const heading = user?.display_name ? `Almost there, ${user.display_name}` : 'Almost there';

  return (
    <Shell heading={heading}>
      <Text className="mt-3 text-sm text-on-surface-variant text-center">
        Where do you want to chat with your coach? Pick the app you already use — you can add more
        later in Settings.
      </Text>
      <View className="mt-6 gap-4">
        {messaging.availableChannels.map((channel) => (
          <ChannelCard
            key={channel.channel}
            channel={channel}
            busy={choosing === channel.channel}
            disabled={choosing !== null}
            onSelect={() => void handleSelect(channel.channel)}
          />
        ))}
      </View>
      <View className="mt-6">
        <Button
          title="Skip for now"
          variant="secondary"
          onPress={() => void messaging.skipMessaging()}
          disabled={choosing !== null}
        />
      </View>
    </Shell>
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
  const hint = channel.method === 'deep_link' ? 'Scan a QR or tap to open the app' : 'Connect with one tap';
  return (
    <Pressable
      onPress={onSelect}
      disabled={disabled}
      accessibilityRole="button"
      accessibilityLabel={channel.display_name}
      className="rounded-2xl border border-outline-variant bg-surface-container-low px-4 py-4 active:opacity-80"
      style={disabled ? { opacity: 0.6 } : undefined}
    >
      <View className="flex-row items-center gap-2">
        <Text className="text-base font-semibold text-on-surface">{channel.display_name}</Text>
        {channel.recommended ? (
          <Text className="rounded-full bg-primary/15 px-2 py-0.5 text-[11px] font-medium text-primary">
            Recommended
          </Text>
        ) : null}
      </View>
      <Text className="mt-1 text-sm text-on-surface-variant">{hint}</Text>
      {busy ? (
        <View className="mt-2 flex-row items-center gap-2">
          <ActivityIndicator size="small" />
          <Text className="text-xs text-on-surface-variant">Setting things up…</Text>
        </View>
      ) : null}
    </Pressable>
  );
}

function Shell({ heading, children }: { heading?: string; children: React.ReactNode }) {
  return (
    <SafeAreaView className="flex-1 bg-background-primary" testID="messaging-channel-screen">
      <ScrollView contentContainerClassName="px-5 py-8">
        <Card>
          <View className="px-2 py-2">
            {heading ? (
              <Text className="text-2xl font-bold text-on-surface text-center">{heading}</Text>
            ) : null}
            {children}
          </View>
        </Card>
      </ScrollView>
    </SafeAreaView>
  );
}
