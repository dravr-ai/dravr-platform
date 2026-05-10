// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Auth group stack layout with slide animation
// ABOUTME: Contains Login, Register, ForgotPassword, ResetPassword, PendingApproval screens

import { Stack } from 'expo-router';
import { useThemeColors } from '../../src/constants/theme';

export default function AuthLayout() {
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
