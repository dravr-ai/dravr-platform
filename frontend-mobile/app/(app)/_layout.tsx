// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: App group layout wrapping tabs with app-level modal screens
// ABOUTME: ShareInsight, AdaptedInsight, and Connections present as modals over tabs

import { Stack } from 'expo-router';

export default function AppLayout() {
  return (
    <Stack screenOptions={{ headerShown: false }}>
      <Stack.Screen name="(tabs)" />
      <Stack.Screen
        name="share-insight"
        options={{ presentation: 'modal', gestureEnabled: true }}
      />
      <Stack.Screen
        name="adapted-insight"
        options={{ presentation: 'modal', gestureEnabled: true }}
      />
      <Stack.Screen
        name="connections"
        options={{ presentation: 'modal', gestureEnabled: true }}
      />
      <Stack.Screen
        name="notifications"
        options={{ presentation: 'modal', gestureEnabled: true }}
      />
    </Stack>
  );
}
