// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Onboarding group stack layout — no header, no back navigation, gated by RootLayoutNav
// ABOUTME: Users only reach this stack when `needs_provider_connection` is true; they exit via OAuth success

import { Stack } from 'expo-router';
import { useThemeColors } from '../../src/constants/theme';

export default function OnboardingLayout() {
  const colors = useThemeColors();
  return (
    <Stack
      screenOptions={{
        headerShown: false,
        contentStyle: { backgroundColor: colors.background.primary },
        animation: 'slide_from_right',
      }}
    />
  );
}
