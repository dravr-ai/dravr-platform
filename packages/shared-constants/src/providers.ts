// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The provider capability-scope vocabulary — one word per wire slug, in the athlete's language
// ABOUTME: Mirrors the scopes the provider-status route builds; labels are corpus keys, resolved with t()

/**
 * Every capability slug `GET /api/oauth/providers` can put on a provider card,
 * in the order the route builds them (`crates/pierre-routes-auth/src/oauth.rs`).
 * The strings are the wire values, so they stay English whatever the athlete
 * reads.
 */
export const PROVIDER_SCOPES = ['activities', 'sleep', 'recovery', 'health'] as const;

export type ProviderScope = (typeof PROVIDER_SCOPES)[number];

/**
 * The corpus key naming each scope. Module scope cannot hold a hook, so the
 * table carries the key and each client resolves it with its own `t` — the
 * card's capability line then reads as words in the athlete's language
 * instead of as the wire slugs under otherwise translated chrome.
 */
export const PROVIDER_SCOPE_LABEL_KEY: Record<ProviderScope, string> = {
  activities: 'providers.scope.activities',
  sleep: 'providers.scope.sleep',
  recovery: 'providers.scope.recovery',
  health: 'providers.scope.health',
};

/**
 * The label key for `scope`, or `null` for a slug the catalogue has no word
 * for. A caller prints such a slug as itself: a provider that starts
 * advertising a new capability shows its wire name, never a missing-key
 * string.
 */
export function providerScopeLabelKey(scope: string): string | null {
  return PROVIDER_SCOPE_LABEL_KEY[scope as ProviderScope] ?? null;
}
