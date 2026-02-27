// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Root index route that redirects to the appropriate initial screen
// ABOUTME: Expo Router needs a matched route at / to avoid rendering empty Slot

import { Redirect } from 'expo-router';

export default function Index() {
  return <Redirect href="/(auth)/login" />;
}
