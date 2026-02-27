// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Coaches tab stack layout for Expo Router
// ABOUTME: Contains CoachLibrary (index), CoachDetail, and CoachEditor screens

import { Stack } from 'expo-router';

export default function CoachesLayout() {
  return <Stack screenOptions={{ headerShown: false }} />;
}
