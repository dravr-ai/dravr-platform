// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Settings tab stack layout for Expo Router, under the native header
// ABOUTME: SettingsScreen (index, large title) and one screen per pane, each titled here so no pane draws a header

import { Stack } from 'expo-router';
import { useTranslation } from '@pierre/i18n';
import { settingsPane } from '@pierre/shared-constants';
import { useStackScreenOptions } from '../../../../src/navigation/stackOptions';

export default function SettingsLayout() {
  const { t } = useTranslation();
  return (
    <Stack screenOptions={useStackScreenOptions()}>
      <Stack.Screen name="index" options={{ title: t('common.settings'), headerLargeTitle: true }} />
      <Stack.Screen name="about" options={{ title: t('about.title') }} />
      <Stack.Screen name="account" options={{ title: t('app.account') }} />
      <Stack.Screen name="coaching-style" options={{ title: t(settingsPane('coaching').nameKey) }} />
      <Stack.Screen name="connected-apps" options={{ title: t('app.connectedAppsLower') }} />
      <Stack.Screen name="connections" options={{ title: t(settingsPane('connections').nameKey) }} />
      <Stack.Screen name="messaging" options={{ title: t(settingsPane('messaging').nameKey) }} />
      <Stack.Screen name="notification-preferences" options={{ title: t('notifPrefs.title') }} />
      <Stack.Screen name="privacy" options={{ title: t(settingsPane('privacy').nameKey) }} />
      <Stack.Screen name="profile" options={{ title: t(settingsPane('profile').nameKey) }} />
      <Stack.Screen name="tokens" options={{ title: t(settingsPane('tokens').nameKey) }} />
    </Stack>
  );
}
