// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
// ABOUTME: Single source of truth for the Pierre API base URL across mobile code paths
// ABOUTME: Every request the app makes — REST and the chat turn stream alike — resolves through this

import { Platform } from 'react-native';

/**
 * Resolve the Pierre API base URL.
 *
 * Precedence:
 * 1. `EXPO_PUBLIC_API_URL` env var (tunnel mode, and every EAS build profile).
 * 2. Android emulator fallback (`10.0.2.2` routes to host loopback).
 * 3. `localhost:8081` for iOS Simulator + web dev.
 *
 * LIMITATION(registre#450): `EXPO_PUBLIC_API_URL` names the same system (`https://app.dravr.ai`,
 * served by the dev GCP project) in the `production` EAS profile as well as `preview`, because
 * `infra/environments/prod/` has no database and its terraform apply is gated `if: false` — the
 * `dev` project is the only live environment, so a TestFlight build and an internal build read
 * and write the same database and the same Firebase auth tenant.
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
