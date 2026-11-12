// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins what both clients offer for a card's past workouts: the export source, and the sign-in a grant chains straight into
// ABOUTME: An export this build has no copy for is never offered, and a card with no sign-in to copy its history opens nothing

import { describe, expect, it } from 'vitest';
import type { ConnectPlanCard } from '@pierre/shared-constants';
import type { HistoryState } from '@pierre/shared-types';
import { cardAwaitingHistory, cardExportSource } from '../src/cardHistory';

/** A Garmin card connected through its grant, its history in `state`. */
const garminGrant = (
  state: HistoryState,
  { signIn = 'sciotte_garmin', archive = 'garmin_archive' }: { signIn?: string | null; archive?: string | null } = {},
): ConnectPlanCard => ({
  provider: 'sciotte_garmin',
  connected: true,
  connected_backend: 'garmin',
  connection_type: 'oauth',
  needs_reauth: false,
  requires_oauth: true,
  own_app_required: false,
  consent_required: false,
  recommended_backend: 'oauth',
  history: { state, sign_in_target: signIn, archive_source: archive },
});

describe('cardExportSource', () => {
  it('names the source the server put on the card, connected or not', () => {
    expect(cardExportSource(garminGrant('imported'))).toBe('garmin_archive');
    expect(cardExportSource({ history: { state: 'not_applicable', sign_in_target: null, archive_source: 'strava_archive' } })).toBe(
      'strava_archive',
    );
  });

  it('is null for no source, or for a source this build has no words for', () => {
    expect(cardExportSource({})).toBeNull();
    expect(cardExportSource(garminGrant('needed', { archive: null }))).toBeNull();
    expect(cardExportSource(garminGrant('needed', { archive: 'polar_archive' }))).toBeNull();
  });
});

describe('cardAwaitingHistory', () => {
  it('chains a Garmin grant straight into the Garmin sign-in, found by the backend that serves the card', () => {
    const card = garminGrant('needed');
    expect(cardAwaitingHistory([card], 'garmin')).toEqual({ card, target: 'garmin' });
    expect(cardAwaitingHistory([card], 'sciotte_garmin')).toEqual({ card, target: 'garmin' });
  });

  it('chains into the sign-in whether or not the card also offers an export', () => {
    const card = garminGrant('needed', { archive: null });
    expect(cardAwaitingHistory([card], 'garmin')).toEqual({ card, target: 'garmin' });
  });

  it('opens nothing once the history is no longer needed', () => {
    for (const state of ['imported', 'not_applicable'] as const) {
      expect(cardAwaitingHistory([garminGrant(state)], 'garmin')).toBeUndefined();
    }
  });

  it('opens nothing for a card the server names no sign-in for', () => {
    expect(cardAwaitingHistory([garminGrant('needed', { signIn: null })], 'garmin')).toBeUndefined();
  });

  it("opens nothing for another provider's grant", () => {
    expect(cardAwaitingHistory([garminGrant('needed')], 'strava')).toBeUndefined();
  });
});
