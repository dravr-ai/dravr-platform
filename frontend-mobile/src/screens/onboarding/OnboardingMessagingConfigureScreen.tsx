// ABOUTME: Onboarding step (mobile) — connect the chosen messaging app via QR (deep link) or OAuth, poll-to-complete
// ABOUTME: Renders the backend qr_svg in a WebView; polls GET /api/messaging/links to auto-advance when the link lands

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React, { useEffect } from 'react';
import { View, Text, ScrollView, ActivityIndicator, Linking } from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { WebView } from 'react-native-webview';
import { useQuery } from '@tanstack/react-query';
import { Card, Button } from '../../components/ui';
import { useAuth } from '../../contexts/AuthContext';
import { messagingApi } from '../../services/api';
import { useMessagingOnboarding } from '../../hooks/useMessagingOnboarding';
import { CHANNEL_LINK_POLL_INTERVAL_MS } from '@pierre/shared-constants';

/**
 * Connect the chosen messaging channel (mobile). Deep-link channels show a QR
 * (rendered from the backend `qr_svg` in a WebView) plus an "Open app" button;
 * OAuth channels show a single connect button. Polls the user's linked channels
 * and flips the shared cache the instant the link lands, routing on to chat.
 */
export function OnboardingMessagingConfigureScreen() {
  const { user } = useAuth();
  const messaging = useMessagingOnboarding(user?.id, true);
  const channel = messaging.chosenChannel;
  const displayName =
    messaging.availableChannels.find((c) => c.channel === channel)?.display_name ?? channel ?? '';

  const {
    data: link,
    isLoading,
    isError,
    refetch,
    isFetching,
  } = useQuery({
    queryKey: ['messaging-link-init', channel],
    queryFn: () => messagingApi.initLink(channel as string),
    enabled: Boolean(channel),
    staleTime: Infinity,
    retry: 1,
  });

  // The link is completed on this same phone (or another device), so the
  // screen can only learn by asking. The wait is what is polled for, so the
  // interval ends with it — the answer cannot change once the channel appears,
  // and a poll that kept running would bill an instance for a screen the
  // athlete has already left. Same rule, same constant, as the web client.
  const { data: links } = useQuery({
    queryKey: ['messaging-links'],
    queryFn: () => messagingApi.listLinks(),
    refetchInterval: query =>
      query.state.data?.some((l) => l.channel === channel)
        ? false
        : CHANNEL_LINK_POLL_INTERVAL_MS,
  });

  useEffect(() => {
    if (channel && links?.some((l) => l.channel === channel)) {
      void messaging.completeConfigure();
    }
  }, [links, channel, messaging]);

  if (isLoading || !channel) {
    return (
      <Shell heading={`Connect ${displayName}`}>
        <View className="items-center gap-4 py-10">
          <ActivityIndicator size="large" />
          <Text className="text-sm text-on-surface">Preparing your {displayName} link…</Text>
        </View>
      </Shell>
    );
  }

  if (isError || !link) {
    return (
      <Shell heading={`Connect ${displayName}`}>
        <View className="items-center gap-4 py-10">
          <Text className="text-base text-on-surface font-medium text-center">
            We couldn&apos;t start the {displayName} connection just now.
          </Text>
          <Button title="Try again" onPress={() => void refetch()} disabled={isFetching} />
          <Button title="Skip for now" variant="secondary" onPress={() => void messaging.skipMessaging()} />
        </View>
      </Shell>
    );
  }

  const isDeepLink = link.method === 'deep_link';

  return (
    <Shell heading={`Connect ${displayName}`}>
      <View className="mt-5 items-center gap-5">
        {isDeepLink && link.qr_svg ? (
          <>
            <View className="rounded-2xl bg-white p-3" style={{ width: 200, height: 200 }}>
              <WebView
                originWhitelist={['*']}
                source={{
                  html: `<html><head><meta name="viewport" content="width=device-width, initial-scale=1"></head><body style="margin:0;background:#fff;display:flex;align-items:center;justify-content:center;height:100%">${link.qr_svg}</body></html>`,
                }}
                style={{ width: 176, height: 176, backgroundColor: '#fff' }}
                scrollEnabled={false}
                testID="messaging-qr"
              />
            </View>
            <Text className="max-w-xs text-center text-sm text-on-surface-variant">
              Scan the QR with another device, or tap below to open {displayName}. Then press Start to
              finish.
            </Text>
          </>
        ) : (
          <Text className="max-w-xs text-center text-sm text-on-surface-variant">
            Tap below to connect {displayName}. You&apos;ll come back here automatically once it&apos;s
            done.
          </Text>
        )}

        <Button
          title={isDeepLink ? `Open ${displayName}` : `Connect with ${displayName}`}
          onPress={() => void Linking.openURL(link.linking_url)}
          fullWidth
        />

        <View className="flex-row items-center gap-2">
          <ActivityIndicator size="small" />
          <Text className="text-xs text-on-surface-variant">Waiting for you to finish in {displayName}…</Text>
        </View>
      </View>

      <View className="mt-6">
        <Button title="Skip for now" variant="secondary" onPress={() => void messaging.skipMessaging()} />
      </View>
    </Shell>
  );
}

function Shell({ heading, children }: { heading?: string; children: React.ReactNode }) {
  return (
    <SafeAreaView className="flex-1 bg-background-primary" testID="messaging-configure-screen">
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
