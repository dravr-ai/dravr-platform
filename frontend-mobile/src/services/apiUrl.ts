// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
// ABOUTME: Single source of truth for the Pierre API base URL across mobile code paths
// ABOUTME: REST (services/api/index.ts) and SSE consumers (hooks/useAgUiProgress) resolve through this

import { Platform } from 'react-native';

/**
 * Resolve the Pierre API base URL.
 *
 * Precedence:
 * 1. `EXPO_PUBLIC_API_URL` env var (tunnel mode, staging, prod).
 * 2. Android emulator fallback (`10.0.2.2` routes to host loopback).
 * 3. `localhost:8081` for iOS Simulator + web dev.
 *
 * Exported as a plain function so both the REST axios client and the
 * AG-UI SSE consumer resolve the same value at call time — keeping
 * the two paths in lockstep if the URL ever changes mid-session
 * (e.g., tunnel restart). Don't inline this anywhere; reference it
 * from the single import.
 */
export function getApiUrl(): string {
  if (process.env.EXPO_PUBLIC_API_URL) {
    return process.env.EXPO_PUBLIC_API_URL;
  }
  if (Platform.OS === 'android') {
    return 'http://10.0.2.2:8081';
  }
  return 'http://localhost:8081';
}
