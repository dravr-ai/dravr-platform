// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Groups tab stack layout for Expo Router
// ABOUTME: Contains GroupList (index), GroupDetail, CreateGroup, and JoinGroup screens

import { Stack } from 'expo-router';

export default function GroupsLayout() {
  return <Stack screenOptions={{ headerShown: false }} />;
}
