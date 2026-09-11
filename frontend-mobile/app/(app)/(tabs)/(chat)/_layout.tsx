// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Chat tab stack layout for Expo Router — the conversation list under the native header
// ABOUTME: A thread lives outside this stack, pushed over the whole tab bar from app/(app)/chat

import { Stack } from 'expo-router';
import { useTranslation } from '@pierre/i18n';
import { useStackScreenOptions } from '../../../../src/navigation/stackOptions';

export default function ChatLayout() {
  const { t } = useTranslation();
  return (
    <Stack screenOptions={useStackScreenOptions()}>
      <Stack.Screen name="index" options={{ title: t('app.convListTitle') }} />
    </Stack>
  );
}
