// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests that the Strava card carries the provider id its grant actually lives under.
// ABOUTME: Guards the disconnect-does-nothing regression where the card sent `sciotte` for an OAuth grant.

import { describe, it, expect } from 'vitest';
import type { ExtendedProviderStatus } from '@pierre/shared-types';

import { buildFitnessProviderCards } from '../fitnessProviderCards';

function provider(
  name: string,
  connected: boolean,
  extra: Partial<ExtendedProviderStatus> = {}
): ExtendedProviderStatus {
  return {
    provider: name,
    display_name: name,
    requires_oauth: false,
    connected,
    needs_reauth: false,
    capabilities: ['activities'],
    ...extra,
  } as ExtendedProviderStatus;
}

describe('buildFitnessProviderCards', () => {
  it('sends the native strava id when an OAuth grant backs the Strava card', () => {
    // This is the exact shape the server returns for an OAuth-connected user:
    // it coalesces the pair, so BOTH rows read connected. The card must still
    // disconnect `strava`, because `DELETE .../sciotte/disconnect` deletes a
    // sciotte row that does not exist, returns 204, and leaves the grant alive.
    const cards = buildFitnessProviderCards([
      provider('sciotte', true, { recommended_backend: 'oauth', seats_left: 10 }),
      provider('strava', true, { requires_oauth: true }),
    ]);

    const strava = cards.find((c) => c.provider === 'sciotte');
    expect(strava).toBeDefined();
    expect(strava?.connectionProvider).toBe('strava');
  });

  it('still folds when the server reports only the native strava row connected', () => {
    const cards = buildFitnessProviderCards([
      provider('sciotte', false, { recommended_backend: 'oauth' }),
      provider('strava', true, { requires_oauth: true, needs_reauth: true }),
    ]);

    const strava = cards.find((c) => c.provider === 'sciotte');
    expect(strava?.connected).toBe(true);
    expect(strava?.connectionProvider).toBe('strava');
    expect(strava?.needs_reauth).toBe(true);
  });

  it('keeps the sciotte id when the mirror is what actually backs the card', () => {
    // No native grant: a scrape-backed Strava card must disconnect `sciotte`.
    const cards = buildFitnessProviderCards([
      provider('sciotte', true, { recommended_backend: 'mirror', seats_left: 0 }),
    ]);

    expect(cards.find((c) => c.provider === 'sciotte')?.connectionProvider).toBe('sciotte');
  });

  it('hides the bare strava and garmin rows from the card list', () => {
    const cards = buildFitnessProviderCards([
      provider('sciotte', false),
      provider('strava', false),
      provider('garmin', false),
      provider('whoop', false),
    ]);

    expect(cards.map((c) => c.provider)).toEqual(['sciotte', 'whoop']);
  });

  it('leaves unrelated providers carrying their own id', () => {
    const cards = buildFitnessProviderCards([provider('whoop', true)]);
    expect(cards[0]?.connectionProvider).toBe('whoop');
  });
});
