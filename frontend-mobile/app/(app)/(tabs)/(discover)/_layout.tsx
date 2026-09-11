// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Discover tab stack layout for Expo Router, under the native header
// ABOUTME: StoreScreen (index, large title) and StoreCoachDetailScreen; the coach edit sheet under edit/[coachId]

import { Stack } from 'expo-router';
import { useTranslation } from '@pierre/i18n';
import { useStackScreenOptions } from '../../../../src/navigation/stackOptions';

export default function DiscoverLayout() {
  const { t } = useTranslation();
  return (
    <Stack screenOptions={useStackScreenOptions()}>
      <Stack.Screen name="index" options={{ title: t('app.discover'), headerLargeTitle: true }} />
      <Stack.Screen name="[coachId]" options={{ title: '' }} />
      <Stack.Screen name="edit/[coachId]" options={{ title: t('app.editAgentTitle') }} />
    </Stack>
  );
}
