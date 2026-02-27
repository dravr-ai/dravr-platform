// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Social tab stack layout for Expo Router
// ABOUTME: Contains SocialFeed, Friends, Adapted Insights, Share Insight, and Activity screens

import { Stack } from 'expo-router';

export default function SocialLayout() {
  return <Stack screenOptions={{ headerShown: false }} />;
}
