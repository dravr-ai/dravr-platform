// ABOUTME: PostHog mobile analytics gated on user analytics_consent
// ABOUTME: Same opt-in default and same SHA-256 user id as web and the server, hashed before identify()
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { analyticsDistinctId, type Sha256Digest } from '@pierre/domain-utils';
import * as Crypto from 'expo-crypto';
import PostHog from 'posthog-react-native';

let client: PostHog | null = null;

/**
 * Counts every boot and shutdown. A boot hashes the user id before it starts
 * PostHog, and a consent revoke or a newer boot can land in that gap; the
 * boot that finds the count moved on has been overtaken and starts nothing.
 */
let generation = 0;

/** SHA-256 from expo-crypto: native on both platforms, no WebCrypto in Hermes. */
const expoSha256: Sha256Digest = (data) => Crypto.digest(Crypto.CryptoDigestAlgorithm.SHA256, data);

/**
 * Boot PostHog mobile. Called from `_layout.tsx` once the auth flow
 * resolves and the user's `analytics_consent` flag is `true`.
 * No-op when consent is `false` (the default — opt-in only).
 *
 * PostHog starts only once the user's id is hashed, and identifies them in
 * the same step, so no event is ever sent under an anonymous id.
 */
export async function bootMobileAnalytics(userId: string, consent: boolean): Promise<void> {
  if (!consent || client) return;
  const apiKey = process.env.EXPO_PUBLIC_POSTHOG_API_KEY;
  const host = process.env.EXPO_PUBLIC_POSTHOG_HOST ?? 'https://us.i.posthog.com';
  if (!apiKey) return;

  const booting = ++generation;
  let distinctId: string;
  try {
    distinctId = await analyticsDistinctId(userId, expoSha256);
  } catch (error) {
    // An id built any other way would be a second person to PostHog, so
    // analytics stay off.
    console.error('Analytics not started: the user id could not be hashed', error);
    return;
  }
  if (booting !== generation) return;

  client = new PostHog(apiKey, {
    host,
    captureAppLifecycleEvents: false,
    enableSessionReplay: false,
    flushInterval: 30,
  });
  client.identify(distinctId);
}

/** Force-stop and clear PostHog state on consent revoke. */
export function shutdownMobileAnalytics(): void {
  generation += 1;
  if (!client) return;
  client.reset();
  client = null;
}

export type MobileAnalyticsEvent =
  | { name: 'screen_view'; props: { screen: string } }
  | { name: 'feature_engaged'; props: { feature: string } }
  | { name: 'checkout_started'; props: { tier: 'starter' | 'professional' | 'enterprise' } };

/** Fire a typed event. No-op when analytics aren't booted. */
export function trackMobile(event: MobileAnalyticsEvent): void {
  if (!client) return;
  // posthog-react-native's capture() takes any JSON-shaped object;
  // our typed event union is structurally compatible.
  client.capture(event.name, event.props as unknown as Parameters<typeof client.capture>[1]);
}
