// ABOUTME: PostHog analytics initialization gated on user analytics_consent
// ABOUTME: The user is identified by the same SHA-256 id the server uses; respects opt-in consent default
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import type { CaptureResult, PostHog } from 'posthog-js';
import { analyticsDistinctId, type Sha256Digest } from '@pierre/domain-utils';

/**
 * `url` without its query strings, before and after the hash route, and
 * without a fragment that holds parameters instead of a route.
 *
 * posthog-js stamps the page address on every event (`$current_url`, the
 * session's entry address, and the initial and last referrer), and a link the
 * app opens can carry a secret in its query — an OAuth `state`, a sign-in
 * `code` — or in its fragment: the mailed confirmation and invitation links
 * put their single-use token in `#token=…` (`utils/tokenLink.ts`). The route
 * the user was on is all analytics need, and no dashboard route holds an `=`.
 */
export function withoutQuery(url: string): string {
  const hashAt = url.indexOf('#');
  const beforeHash = hashAt === -1 ? url : url.slice(0, hashAt);
  const base = beforeHash.split('?')[0];
  if (hashAt === -1) {
    return base;
  }
  const route = url.slice(hashAt + 1).split('?')[0];
  return route.includes('=') ? base : `${base}#${route}`;
}

/**
 * Address-bearing properties, as posthog-js sends them on events and persons:
 * `$current_url`, `$session_entry_url`, `$initial_current_url`, and the
 * `…referrer` family.
 */
const ADDRESS_PROPERTY = /(_url|referrer)$/;

/** Strip query strings from every address property of `bag`, in place. */
function scrubAddresses(bag: Record<string, unknown> | undefined): void {
  if (bag === undefined) {
    return;
  }
  for (const [key, value] of Object.entries(bag)) {
    if (typeof value === 'string' && ADDRESS_PROPERTY.test(key)) {
      bag[key] = withoutQuery(value);
    }
  }
}

/** posthog-js `before_send`: no event leaves with a query string in an address. */
export function scrubEventAddresses(event: CaptureResult | null): CaptureResult | null {
  if (event === null) {
    return null;
  }
  scrubAddresses(event.properties);
  scrubAddresses(event.$set);
  scrubAddresses(event.$set_once);
  return event;
}

/**
 * The started PostHog client, or `null` while analytics are off.
 *
 * posthog-js is imported only once a consenting user boots it: imported
 * statically it would add ~185 KB to the entry bundle every visitor downloads
 * before first paint, though consent is opt-in and off by default.
 */
let client: PostHog | null = null;

/**
 * Counts every boot and shutdown. A boot hashes the user id before it starts
 * PostHog, and a consent revoke or a newer boot can land in that gap; the
 * boot that finds the count moved on has been overtaken and starts nothing.
 */
let generation = 0;

/**
 * The page the user is on, held while PostHog is not running. The dashboard
 * reports its first route as it mounts, before the boot that follows sign-in
 * has hashed the id and loaded the library; without this that first
 * `$pageview` — often the only one of a short visit — would be dropped. Only the
 * latest page is kept, it never leaves the browser unless the user has
 * consented, and a shutdown forgets it.
 */
let heldPageview: Record<string, unknown> | null = null;

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
  if (!consent || client !== null) {
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
  const { default: posthog } = await import('posthog-js');
  // Loading the library is a second gap a revoke or a newer boot can land in.
  if (booting !== generation) {
    return;
  }
  // posthog-js keeps one instance per page and ignores a second init, so a
  // boot after a sign-out or a revoke reuses the instance the first one made.
  if (!posthog.__loaded) {
    posthog.init(token, {
      api_host: host,
      // Page views are sent by `track`, from the dashboard's route, which the
      // library cannot see: the app routes on the hash.
      capture_pageview: false,
      persistence: 'localStorage',
      autocapture: false,
      disable_session_recording: true,
      disable_surveys: true,
      // The CSP allows scripts from this origin only, so the library must
      // never inject one from PostHog's CDN (surveys, replay, site apps).
      disable_external_dependency_loading: true,
      before_send: scrubEventAddresses,
    });
  }
  // A sign-out or a revoke opts this browser out (`shutdownAnalytics`), and
  // posthog-js keeps that choice in localStorage across reloads. Starting
  // again means the user has consented again: without opting back in, every
  // event after the first sign-out would be dropped.
  if (posthog.has_opted_out_capturing()) {
    posthog.opt_in_capturing({ captureEventName: false });
  }
  posthog.identify(distinctId);
  client = posthog;
  if (heldPageview !== null) {
    posthog.capture('$pageview', heldPageview);
    heldPageview = null;
  }
}

/** Force-stop and clear PostHog state — called when consent is revoked. */
export function shutdownAnalytics(): void {
  generation += 1;
  heldPageview = null;
  if (client === null) {
    return;
  }
  client.opt_out_capturing();
  client.reset();
  client = null;
}

export type AnalyticsEvent =
  | { name: '$pageview'; props: { path: string } }
  | { name: 'feature_engaged'; props: { feature: string } }
  | { name: 'checkout_started'; props: { tier: 'starter' | 'professional' | 'enterprise' } }
  | { name: 'checkout_completed'; props: { tier: string } }
  | { name: 'tier_changed'; props: { from: string; to: string } };

/**
 * Send a typed event to PostHog. No-op when analytics aren't booted, except a
 * `$pageview`: the latest one is held and sent once a consenting boot
 * completes.
 *
 * A `$pageview` is PostHog's own page-view event, so the web analytics views
 * read it; `$current_url` is the full address, hash route included, as the
 * user saw it when the page changed.
 */
export function track(event: AnalyticsEvent): void {
  if (event.name === '$pageview') {
    const props = { ...event.props, $current_url: withoutQuery(window.location.href) };
    if (client === null) {
      heldPageview = props;
      return;
    }
    client.capture('$pageview', props);
    return;
  }
  if (client === null) {
    return;
  }
  client.capture(event.name, event.props as Record<string, unknown>);
}
