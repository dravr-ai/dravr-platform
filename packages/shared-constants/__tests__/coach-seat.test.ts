// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Proves who is offered the coach seat, how a request's status reads as a seat state, what the seat step says, and that its words exist
// ABOUTME: The seat step says only what the server holds; every seat key resolves in all five catalogues, so no coach sees a raw key

import { describe, expect, it } from 'vitest';
import type { CoachAccessRequest } from '@pierre/shared-types';
import de from '../../i18n/src/locales/de/translation.json';
import en from '../../i18n/src/locales/en/translation.json';
import es from '../../i18n/src/locales/es/translation.json';
import fr from '../../i18n/src/locales/fr/translation.json';
import pt from '../../i18n/src/locales/pt/translation.json';
import {
  COACH_SEAT_KEYS,
  COACH_SEAT_SUBTITLE_KEY,
  canRequestCoachAccess,
  coachSeatMoment,
  coachSeatState,
  coachSeatStepState,
  connectedCoachPlatforms,
} from '../src/coach-seat';

const coachless = { coach_user_id: null };
const owner = { group: coachless, isOwner: true, managesRoster: false, coachesOthers: true };

function request(status: CoachAccessRequest['status']): CoachAccessRequest {
  return {
    id: 'req-1',
    user_id: 'user-1',
    group_id: 'group-1',
    group_tenant_id: 'tenant-1',
    status,
    created_at: '2026-11-12T19:32:00Z',
    decided_at: null,
    decided_by: null,
  };
}

function lookup(catalogue: unknown, key: string): unknown {
  return key.split('.').reduce<unknown>(
    (node, part) => (node && typeof node === 'object' ? (node as Record<string, unknown>)[part] : undefined),
    catalogue,
  );
}

describe('canRequestCoachAccess', () => {
  it('offers the seat to the owner of a coachless group who coaches others and holds no coach access', () => {
    expect(canRequestCoachAccess(owner)).toBe(true);
  });

  it('offers nothing to a member who is not the owner', () => {
    expect(canRequestCoachAccess({ ...owner, isOwner: false })).toBe(false);
  });

  it('offers nothing once the group has a human coach', () => {
    expect(canRequestCoachAccess({ ...owner, group: { coach_user_id: 'coach-1' } })).toBe(false);
  });

  it('offers nothing to a viewer who already holds coach access', () => {
    expect(canRequestCoachAccess({ ...owner, managesRoster: true })).toBe(false);
  });

  it('offers nothing to an owner who does not coach others', () => {
    expect(canRequestCoachAccess({ ...owner, coachesOthers: false })).toBe(false);
  });

  it('offers nothing before the group is read', () => {
    expect(canRequestCoachAccess({ ...owner, group: null })).toBe(false);
    expect(canRequestCoachAccess({ ...owner, group: undefined })).toBe(false);
  });
});

describe('coachSeatState', () => {
  it('reads no request as one still to make, never as pending', () => {
    expect(coachSeatState(null)).toBe('to_request');
    expect(coachSeatState(undefined)).toBe('to_request');
  });

  it('reads a pending and a declined request as such', () => {
    expect(coachSeatState(request('pending'))).toBe('pending');
    expect(coachSeatState(request('declined'))).toBe('declined');
  });

  it('reads a granted request whose access is gone as one to make again', () => {
    expect(coachSeatState(request('granted'))).toBe('to_request');
  });
});

describe('coachSeatMoment', () => {
  it('spells the day in the reader locale and the time on a 24-hour clock', () => {
    const moment = coachSeatMoment('2026-11-12T19:32:00Z', 'fr');
    expect(moment.date).toMatch(/nov\.? 2026/);
    expect(moment.time).toMatch(/^\d{2}:\d{2}$/);
  });

  it('returns an unparseable stamp verbatim rather than Invalid Date', () => {
    expect(coachSeatMoment('not-a-date', 'en')).toEqual({ date: 'not-a-date', time: '' });
  });
});

describe('coach seat words', () => {
  const keys = [...Object.values(COACH_SEAT_KEYS), ...Object.values(COACH_SEAT_SUBTITLE_KEY)];

  it.each([
    ['en', en],
    ['fr', fr],
    ['es', es],
    ['de', de],
    ['pt', pt],
  ])('resolves every key in the %s catalogue', (_locale, catalogue) => {
    for (const key of keys) {
      expect(typeof lookup(catalogue, key), key).toBe('string');
    }
  });

  it('keeps every key under humanCoach, where the human coach is named (ADR-026)', () => {
    for (const key of keys) {
      expect(key.startsWith('humanCoach.')).toBe(true);
    }
  });
});

const card = (provider: string, display_name: string, extra: Partial<{ connected: boolean; account_role: 'athlete' | 'coach' }> = {}) => ({
  provider,
  display_name,
  connected: false,
  ...extra,
});

const none = [card('sciotte', 'Strava', { connected: true }), card('intervals_icu', 'Intervals.icu'), card('sciotte_trainingpeaks', 'TrainingPeaks')];

describe('coachSeatStepState', () => {
  it('is open while nothing coaching is connected and nothing asked — another provider does not count', () => {
    expect(coachSeatStepState({ managesRoster: false, request: null, providers: none, checkExpired: false })).toEqual({
      kind: 'open',
    });
  });

  it('says coach access first, whatever else is on record', () => {
    expect(
      coachSeatStepState({ managesRoster: true, request: request('pending'), providers: none, checkExpired: true }),
    ).toEqual({ kind: 'coach' });
  });

  it('dates a request waiting', () => {
    expect(
      coachSeatStepState({ managesRoster: false, request: request('pending'), providers: none, checkExpired: false }),
    ).toEqual({ kind: 'requested', sentAt: '2026-11-12T19:32:00Z' });
  });

  it('says a declined request, which no connection reopens', () => {
    const providers = [card('intervals_icu', 'Intervals.icu', { connected: true })];
    expect(
      coachSeatStepState({ managesRoster: false, request: request('declined'), providers, checkExpired: true }),
    ).toEqual({ kind: 'declined' });
  });

  it('waits on a connection until its window ends, then says it produced nothing', () => {
    const providers = [card('intervals_icu', 'Intervals.icu', { connected: true })];
    expect(coachSeatStepState({ managesRoster: false, request: null, providers, checkExpired: false })).toEqual({
      kind: 'checking',
    });
    expect(coachSeatStepState({ managesRoster: false, request: null, providers, checkExpired: true })).toEqual({
      kind: 'nothing',
      provider: 'Intervals.icu',
    });
  });

  it('settles at once on a TrainingPeaks athlete account: only a coach account grants', () => {
    const athlete = [card('sciotte_trainingpeaks', 'TrainingPeaks', { connected: true, account_role: 'athlete' })];
    expect(coachSeatStepState({ managesRoster: false, request: null, providers: athlete, checkExpired: false })).toEqual({
      kind: 'nothing',
      provider: 'TrainingPeaks',
    });
    const coach = [card('sciotte_trainingpeaks', 'TrainingPeaks', { connected: true, account_role: 'coach' })];
    expect(coachSeatStepState({ managesRoster: false, request: null, providers: coach, checkExpired: false })).toEqual({
      kind: 'checking',
    });
  });

  it('a granted request whose access was taken back reads as no seat', () => {
    expect(
      coachSeatStepState({ managesRoster: false, request: request('granted'), providers: none, checkExpired: false }),
    ).toEqual({ kind: 'open' });
  });
});

describe('connectedCoachPlatforms', () => {
  it('lists the connected coaching platforms in seat order, TrainingPeaks first', () => {
    const providers = [
      card('intervals_icu', 'Intervals.icu', { connected: true }),
      card('sciotte', 'Strava', { connected: true }),
      card('sciotte_trainingpeaks', 'TrainingPeaks', { connected: true }),
    ];
    expect(connectedCoachPlatforms(providers).map((p) => p.provider)).toEqual(['sciotte_trainingpeaks', 'intervals_icu']);
  });
});
