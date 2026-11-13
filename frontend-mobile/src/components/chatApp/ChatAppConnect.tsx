// ABOUTME: The chat-app link inside the chat-app sheet (mobile) — deep link or OAuth, then the poll that sees it land
// ABOUTME: Taps straight through to the chat app on a phone, offers the qr_svg handoff on a tablet; « Utiliser une autre app » goes back

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React, { useEffect } from 'react';
import { View, Text, Pressable, ActivityIndicator } from 'react-native';
import { WebView } from 'react-native-webview';
import * as Device from 'expo-device';
import { useQuery } from '@tanstack/react-query';
import { Button } from '../ui';
import { messagingApi } from '../../services/api';
import { openExternal } from '../../utils/openExternal';
import { BOREAL_LIGHT, CHANNEL_LINK_POLL_INTERVAL_MS } from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';

// A QR ground is not themed: the phone camera needs white behind the code in both schemes.
const QR_GROUND = BOREAL_LIGHT.surfaceContainerLowest;

/**
 * Connect the chosen messaging channel (mobile). On a phone the whole view is
 * one "Open app" button — the deep link carries the pairing code, so the chat
 * app opens with it already in hand. A tablet additionally gets the backend
 * `qr_svg` (rendered in a WebView) to hand the link off to a phone; OAuth
 * channels show a single connect button. Polls the user's linked channels and
 * reports the instant the link lands.
 */
export function ChatAppConnect({
  channel,
  displayName,
  onLinked,
  onUseAnother,
}: {
  channel: string;
  displayName: string;
  onLinked: () => void;
  /** Back to the app choice; absent when the workspace offers only this app. */
  onUseAnother?: () => void;
}) {
  const { t } = useTranslation();

  const {
    data: link,
    isLoading,
    isError,
    refetch,
    isFetching,
  } = useQuery({
    queryKey: ['messaging-link-init', channel],
    queryFn: () => messagingApi.initLink(channel),
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
    if (links?.some((l) => l.channel === channel)) {
      onLinked();
    }
  }, [links, channel, onLinked]);

  const useAnother = onUseAnother ? (
    <Pressable
      onPress={onUseAnother}
      accessibilityRole="button"
      testID="chat-app-use-another"
      className="min-h-11 items-center justify-center self-center px-2"
    >
      <Text className="text-sm font-medium text-primary">{t('chatApp.useAnother')}</Text>
    </Pressable>
  ) : null;

  if (isLoading) {
    return (
      <View className="items-center gap-4 py-10">
        <ActivityIndicator size="large" />
        <Text className="text-sm text-on-surface">{t('app.preparingLink', { channel: displayName })}</Text>
      </View>
    );
  }

  if (isError || !link) {
    return (
      <View className="items-center gap-4 py-10">
        <Text className="text-base text-on-surface font-medium text-center">
          {t('app.couldNotStartConnection', { channel: displayName })}
        </Text>
        <Button title={t('app.tryAgainLower')} variant="secondary" onPress={() => void refetch()} disabled={isFetching} />
        {useAnother}
      </View>
    );
  }

  const isDeepLink = link.method === 'deep_link';

  // The QR exists to hand the link off to a DIFFERENT device — it is how someone
  // onboarding on a laptop reaches the chat app that only lives on their phone.
  // On a phone it is the opposite of useful: the athlete cannot scan the screen
  // they are holding, and "scan with another device" reads as an instruction to
  // go find one, burying the single tap that actually works. A tablet keeps it,
  // because `supportsTablet` is on and an iPad athlete plausibly runs Telegram
  // on their phone. An unknown device type keeps it too: hiding a handoff that
  // was needed is the worse of the two failures.
  const showQr =
    isDeepLink && Boolean(link.qr_svg) && Device.deviceType !== Device.DeviceType.PHONE;

  return (
    <View className="items-center gap-5">
      {showQr ? (
        <>
          <View className="rounded-2xl bg-white p-3" style={{ width: 200, height: 200 }}>
            <WebView
              originWhitelist={['*']}
              source={{
                html: `<html><head><meta name="viewport" content="width=device-width, initial-scale=1"></head><body style="margin:0;background:${QR_GROUND};display:flex;align-items:center;justify-content:center;height:100%">${link.qr_svg}</body></html>`,
              }}
              style={{ width: 176, height: 176, backgroundColor: QR_GROUND }}
              scrollEnabled={false}
              testID="messaging-qr"
            />
          </View>
          <Text className="max-w-xs text-center text-sm text-on-surface-variant">
            {/* Telegram ends on its Start button; WhatsApp opens a chat with
                the code already typed, which the athlete sends. */}
            {t(link.channel === 'whatsapp' ? 'app.scanQrThenSend' : 'app.scanQrThenStart', {
              channel: displayName,
            })}
          </Text>
        </>
      ) : (
        <Text className="max-w-xs text-center text-sm text-on-surface-variant">
          {t('app.tapToConnectAutoReturn', { channel: displayName })}
        </Text>
      )}

      <Button
        title={
          isDeepLink
            ? t('app.openChannel', { channel: displayName })
            : t('app.connectWithChannel', { channel: displayName })
        }
        onPress={() => void openExternal(link.linking_url, t)}
        fullWidth
      />

      <View className="flex-row items-center gap-2">
        <ActivityIndicator size="small" />
        <Text className="text-xs text-on-surface-variant">
          {t('app.waitingToFinishIn', { channel: displayName })}
        </Text>
      </View>
      {useAnother}
    </View>
  );
}
