// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Chat tab stack layout for Expo Router
// ABOUTME: The conversation list is the index; a thread is the [conversationId] route pushed over it

import { Stack } from 'expo-router';

export default function ChatLayout() {
  return <Stack screenOptions={{ headerShown: false }} />;
}
