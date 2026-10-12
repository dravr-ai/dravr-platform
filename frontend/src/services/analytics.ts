// ABOUTME: PostHog analytics initialization gated on user analytics_consent
// ABOUTME: The user is identified by the same SHA-256 id the server uses; respects opt-in consent default
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import posthog from 'posthog-js';
import { analyticsDistinctId, type Sha256Digest } from '@pierre/domain-utils';

let initialized = false;

/**
 * Counts every boot and shutdown. A boot hashes the user id before it starts
 * PostHog, and a consent revoke or a newer boot can land in that gap; the
 * boot that finds the count moved on has been overtaken and starts nothing.
 */
let generation = 0;

/** The browser's SHA-256. */
const webSha256: Sha256Digest = (data) => crypto.subtle.digest('SHA-256', data);

/**
 * Boot PostHog. Called once after the user's analytics_consent flag is
 * known and resolves to `true`. No-op if consent is `false` (the
 * default — opt-in, never opt-out). Subsequent calls are idempotent
 * so a re-render or auth refresh won't double-init.
 *
 * PostHog starts only once the user's id is hashed, and identifies them in
 * the same step, so no event is ever sent under an anonymous id.
 */
export async function bootAnalytics(userId: string, consent: boolean): Promise<void> {
  if (!consent || initialized) {
    return;
  }
  const token = import.meta.env.VITE_POSTHOG_API_KEY as string | undefined;
  const host = (import.meta.env.VITE_POSTHOG_HOST as string | undefined) ?? 'https://us.i.posthog.com';
  if (!token) {
    return;
  }
  const booting = ++generation;
  let distinctId: string;
  try {
    distinctId = await analyticsDistinctId(userId, webSha256);
  } catch (error) {
    // WebCrypto is absent outside a secure context. An id built any other way
    // would be a second person to PostHog, so analytics stay off.
    console.error('Analytics not started: the user id could not be hashed', error);
    return;
  }
  if (booting !== generation) {
    return;
  }
  posthog.init(token, {
    api_host: host,
    capture_pageview: false,
    persistence: 'localStorage',
    autocapture: false,
    disable_session_recording: true,
  });
  posthog.identify(distinctId);
  initialized = true;
}

/** Force-stop and clear PostHog state — called when consent is revoked. */
export function shutdownAnalytics(): void {
  generation += 1;
  if (!initialized) {
    return;
  }
  posthog.opt_out_capturing();
  posthog.reset();
  initialized = false;
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
