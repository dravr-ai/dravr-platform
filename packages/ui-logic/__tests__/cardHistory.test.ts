// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins what both clients offer for a card's past workouts: the choices shown, the export source, the card a grant owes its sheet
// ABOUTME: An export this build has no copy for is never offered, and a card with no choice left never opens the sheet

import { describe, expect, it } from 'vitest';
import type { ConnectPlanCard } from '@pierre/shared-constants';
import type { HistoryState } from '@pierre/shared-types';
import { cardAwaitingHistory, cardExportSource, shownHistoryChoices } from '../src/cardHistory';

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

describe('shownHistoryChoices', () => {
  it('offers the sign-in and the export the build carries the copy of', () => {
    expect(shownHistoryChoices(garminGrant('needed'))).toEqual([
      { kind: 'sign_in', target: 'garmin' },
      { kind: 'archive', source: 'garmin_archive' },
    ]);
  });

  it('leaves out an export source this build has no words for', () => {
    expect(shownHistoryChoices(garminGrant('needed', { archive: 'polar_archive' }))).toEqual([
      { kind: 'sign_in', target: 'garmin' },
    ]);
  });

  it('is empty when the card offers neither a sign-in nor an export it can show', () => {
    expect(shownHistoryChoices(garminGrant('needed', { signIn: null, archive: 'polar_archive' }))).toEqual([]);
  });
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
  it('finds the card a Garmin grant landed on, by the backend that serves it', () => {
    const card = garminGrant('needed');
    expect(cardAwaitingHistory([card], 'garmin')).toBe(card);
    expect(cardAwaitingHistory([card], 'sciotte_garmin')).toBe(card);
  });

  it('opens nothing once the history is no longer needed', () => {
    for (const state of ['importing', 'incomplete', 'imported', 'not_applicable'] as const) {
      expect(cardAwaitingHistory([garminGrant(state)], 'garmin')).toBeUndefined();
    }
  });

  it('opens nothing for a card with no choice this build can show', () => {
    expect(cardAwaitingHistory([garminGrant('needed', { signIn: null, archive: 'polar_archive' })], 'garmin')).toBeUndefined();
  });

  it('opens nothing for another provider\'s grant', () => {
    expect(cardAwaitingHistory([garminGrant('needed')], 'strava')).toBeUndefined();
  });
});
