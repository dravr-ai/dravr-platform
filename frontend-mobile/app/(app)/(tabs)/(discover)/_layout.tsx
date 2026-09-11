// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Discover tab stack layout for Expo Router, under the native header
// ABOUTME: StoreScreen (index, large title) and StoreAgentDetailScreen; the agent edit sheet under edit/[agentId]

import { Stack } from 'expo-router';
import { useTranslation } from '@pierre/i18n';
import { useStackScreenOptions } from '../../../../src/navigation/stackOptions';

export default function DiscoverLayout() {
  const { t } = useTranslation();
  return (
    <Stack screenOptions={useStackScreenOptions()}>
      <Stack.Screen name="index" options={{ title: t('app.discover'), headerLargeTitle: true }} />
      <Stack.Screen name="[agentId]" options={{ title: '' }} />
      <Stack.Screen name="edit/[agentId]" options={{ title: t('app.editAgentTitle') }} />
    </Stack>
  );
}
