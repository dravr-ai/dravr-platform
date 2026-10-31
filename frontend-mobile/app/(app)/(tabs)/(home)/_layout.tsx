// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Home tab stack layout for Expo Router — the athlete's Home under the native header, and a coach's athlete detail and reply review pushed over it
// ABOUTME: The screen sets its own title view (the pressable Dravr lockup); the title here is the spoken fallback

import { Stack } from 'expo-router';
import { useTranslation } from '@pierre/i18n';
import { useStackScreenOptions } from '../../../../src/navigation/stackOptions';

export default function HomeLayout() {
  const { t } = useTranslation();
  return (
    <Stack screenOptions={useStackScreenOptions()}>
      <Stack.Screen name="index" options={{ title: t('nav.home') }} />
      <Stack.Screen name="athlete/[athleteId]" options={{ title: '' }} />
      <Stack.Screen name="replies" options={{ title: t('coachReview.nav') }} />
    </Stack>
  );
}
