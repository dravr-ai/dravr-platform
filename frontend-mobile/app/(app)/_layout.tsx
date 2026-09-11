// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: App group layout — the tabs, the chat thread pushed over them, and the app-level modal screens
// ABOUTME: The thread sits here rather than in the chat tab so the tab bar leaves when it opens

import React from 'react';
import { Stack } from 'expo-router';
import { useTranslation } from '@pierre/i18n';
import { settingsPane } from '@pierre/shared-constants';
import { HeaderCloseButton } from '../../src/components/ui/HeaderCloseButton';
import { useStackScreenOptions } from '../../src/navigation/stackOptions';

export default function AppLayout() {
  const { t } = useTranslation();
  const closeButton = () => <HeaderCloseButton />;
  return (
    <Stack screenOptions={useStackScreenOptions()}>
      <Stack.Screen name="(tabs)" options={{ headerShown: false }} />
      {/* The thread names itself: title, avatar and subtitle come from the conversation. */}
      <Stack.Screen name="chat/[conversationId]" options={{ title: '' }} />
      <Stack.Screen
        name="connections"
        options={{
          presentation: 'modal',
          gestureEnabled: true,
          title: t(settingsPane('connections').nameKey),
          headerLeft: closeButton,
        }}
      />
      <Stack.Screen
        name="notifications"
        options={{
          presentation: 'modal',
          gestureEnabled: true,
          title: t('common.notifications'),
          headerLeft: () => <HeaderCloseButton testID="notifications-back" />,
        }}
      />
      {/* Memory names itself: both clients read the one `shell.memoryTitle`. */}
      <Stack.Screen
        name="memory"
        options={{ presentation: 'modal', gestureEnabled: true, title: '', headerLeft: closeButton }}
      />
      <Stack.Screen
        name="billing"
        options={{
          presentation: 'modal',
          gestureEnabled: true,
          title: t(settingsPane('billing').nameKey),
          headerLeft: closeButton,
        }}
      />
    </Stack>
  );
}
