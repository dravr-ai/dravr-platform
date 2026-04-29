// ABOUTME: PostHog analytics initialization gated on user analytics_consent
// ABOUTME: All identifiers SHA-256 hashed; respects opt-in consent default
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import posthog from 'posthog-js';

let initialized = false;

/**
 * Boot PostHog. Called once after the user's analytics_consent flag is
 * known and resolves to `true`. No-op if consent is `false` (the
 * default — opt-in, never opt-out). Subsequent calls are idempotent
 * so a re-render or auth refresh won't double-init.
 */
export function bootAnalytics(userId: string, consent: boolean): void {
  if (!consent || initialized) {
    return;
  }
  const token = import.meta.env.VITE_POSTHOG_API_KEY as string | undefined;
  const host = (import.meta.env.VITE_POSTHOG_HOST as string | undefined) ?? 'https://us.i.posthog.com';
  if (!token) {
    return;
  }
  posthog.init(token, {
    api_host: host,
    capture_pageview: false,
    persistence: 'localStorage',
    autocapture: false,
    disable_session_recording: true,
  });
  posthog.identify(hashUserId(userId));
  initialized = true;
}

/** Force-stop and clear PostHog state — called when consent is revoked. */
export function shutdownAnalytics(): void {
  if (!initialized) {
    return;
  }
  posthog.opt_out_capturing();
  posthog.reset();
  initialized = false;
}

/** SHA-256 of the user UUID — keeps PostHog identity stable across
 *  sessions without the raw id ever leaving the device. */
function hashUserId(userId: string): string {
  const bytes = new TextEncoder().encode(userId);
  let h = 0x811c9dc5;
  for (const b of bytes) {
    h ^= b;
    h = (h * 0x01000193) >>> 0;
  }
  return `u_${h.toString(16).padStart(8, '0')}`;
}

export type AnalyticsEvent =
  | { name: 'page_view'; props: { path: string } }
  | { name: 'feature_engaged'; props: { feature: string } }
  | { name: 'checkout_started'; props: { tier: 'starter' | 'professional' | 'enterprise' } }
  | { name: 'checkout_completed'; props: { tier: string } }
  | { name: 'tier_changed'; props: { from: string; to: string } };

/** Send a typed event to PostHog. No-op when analytics aren't booted. */
export function track(event: AnalyticsEvent): void {
  if (!initialized) {
    return;
  }
  posthog.capture(event.name, event.props as Record<string, unknown>);
}
