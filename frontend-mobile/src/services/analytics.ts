// ABOUTME: PostHog mobile analytics gated on user analytics_consent
// ABOUTME: Same opt-in default as web; identifier is hashed before identify()
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import PostHog from 'posthog-react-native';

let client: PostHog | null = null;

/**
 * Boot PostHog mobile. Called from `_layout.tsx` once the auth flow
 * resolves and the user's `analytics_consent` flag is `true`.
 * No-op when consent is `false` (the default — opt-in only).
 */
export async function bootMobileAnalytics(userId: string, consent: boolean): Promise<void> {
  if (!consent || client) return;
  const apiKey = process.env.EXPO_PUBLIC_POSTHOG_API_KEY;
  const host = process.env.EXPO_PUBLIC_POSTHOG_HOST ?? 'https://us.i.posthog.com';
  if (!apiKey) return;

  client = new PostHog(apiKey, {
    host,
    captureAppLifecycleEvents: false,
    enableSessionReplay: false,
    flushInterval: 30,
  });
  client.identify(hashUserId(userId));
}

/** Force-stop and clear PostHog state on consent revoke. */
export function shutdownMobileAnalytics(): void {
  if (!client) return;
  client.reset();
  client = null;
}

/** Stable hashed identifier — never sends the raw user UUID. */
function hashUserId(userId: string): string {
  let h = 0x811c9dc5;
  for (let i = 0; i < userId.length; i++) {
    h ^= userId.charCodeAt(i);
    h = (h * 0x01000193) >>> 0;
  }
  return `u_${h.toString(16).padStart(8, '0')}`;
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
