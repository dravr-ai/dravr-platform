// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for setting one anchor — pace entry per km, mi or 100 m, the form read into an edit, and the two saves' caches
// ABOUTME: A pace prefills as the screen shows it, a typed range is sent lower number first, and how it was found is always asked

import { describe, it, expect, vi } from 'vitest';
import { createElement, type ReactNode } from 'react';
import { act, renderHook, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import type { PhysiologyAnchor, PhysiologyAnchorEdit, PhysiologyResponse } from '@pierre/shared-types';
import { formatClock } from '../src/activityView';
import {
  anchorEntryUnit,
  createCoachPhysiologyHooks,
  createPhysiologyEditHook,
  initialAnchorForm,
  newAnchorForm,
  paceDistanceFor,
  paceSeconds,
  parseEnteredNumber,
  parsePace,
  readAnchorForm,
  storedAnchorForm,
} from '../src/physiologyEdit';
import { UnitsContext } from '../src/unitsHook';

const CRITICAL_SPEED: PhysiologyAnchor = {
  anchor: 'critical_speed',
  unit: 'metres_per_second',
  value: 4,
  low: 3.92,
  high: 4.08,
  origin: 'athlete',
  as_of: '2026-10-28',
  method: '3-min all-out',
  confidence: 'medium',
  value_text: '4:10/km',
  range_text: '4:05–4:15/km',
  source_text: 'your own value',
};

const BODY: PhysiologyResponse = {
  sports: [{ sport: 'run', anchors: [CRITICAL_SPEED] }],
  offered: [{ sport: 'run', anchors: [{ anchor: 'critical_speed', unit: 'metres_per_second' }] }],
};

function wrapperFor(client: QueryClient, units: 'metric' | 'imperial') {
  return ({ children }: { children: ReactNode }) =>
    createElement(QueryClientProvider, { client }, createElement(UnitsContext.Provider, { value: units }, children));
}

describe('pace entry', () => {
  it('enters a run pace per km or mi as the reader reads, a pool pace per 100 m, and none on the bike', () => {
    expect(paceDistanceFor('run', 'metric')).toBe('km');
    expect(paceDistanceFor('run', 'imperial')).toBe('mi');
    expect(paceDistanceFor('swim', 'imperial')).toBe('100m');
    expect(paceDistanceFor('ride', 'metric')).toBeNull();
  });

  it('writes a speed as the pace the screen shows', () => {
    expect(formatClock(paceSeconds(4, 'km'))).toBe('4:10');
    expect(formatClock(paceSeconds(4, 'mi'))).toBe('6:42');
    expect(formatClock(paceSeconds(1.25, '100m'))).toBe('1:20');
  });

  it('reads m:ss and plain numbers, and nothing else', () => {
    expect(parsePace('4:10')).toBe(250);
    expect(parsePace(' 12:05 ')).toBe(725);
    expect(parsePace('4:75')).toBeNull();
    expect(parsePace('250')).toBeNull();
    expect(parsePace('0:00')).toBeNull();
    expect(parseEnteredNumber('285')).toBe(285);
    expect(parseEnteredNumber('1,25')).toBe(1.25);
    expect(parseEnteredNumber('28a')).toBeNull();
  });
});

describe('the anchor form', () => {
  it('prefills a stored pace, its range from the faster end, and leaves the day and kind to the editor', () => {
    const form = storedAnchorForm('run', CRITICAL_SPEED, 'metric');
    expect(form).toMatchObject({
      value: '4:10',
      low: '4:05',
      high: '4:15',
      confidence: 'medium',
      method: '3-min all-out',
      measuredOn: '',
      kind: null,
    });
  });

  it('reads a pace into the seconds the server converts, with its distance', () => {
    const form = { ...storedAnchorForm('run', CRITICAL_SPEED, 'imperial'), kind: 'measured' as const };
    const read = readAnchorForm(form, 'imperial');
    expect(read).toEqual({
      ok: true,
      edit: {
        value: 402,
        low: 394,
        high: 411,
        pace_per: 'mi',
        kind: 'measured',
        confidence: 'medium',
        method: '3-min all-out',
        measured_on: null,
      } satisfies PhysiologyAnchorEdit,
    });
  });

  it('sends watts as typed, a range lower number first, and a new anchor at low confidence', () => {
    const form = { ...newAnchorForm('ride', 'ftp', 'watts'), value: '285', low: '295', high: '275', kind: 'estimated' as const };
    const read = readAnchorForm(form, 'metric');
    expect(read).toEqual({
      ok: true,
      edit: {
        value: 285,
        low: 275,
        high: 295,
        pace_per: null,
        kind: 'estimated',
        confidence: 'low',
        method: null,
        measured_on: null,
      },
    });
  });

  it('opens on the stored anchor, or a new one on the first anchor offered, and on nothing when none is', () => {
    expect(initialAnchorForm({ mode: 'edit', sport: 'run', anchor: CRITICAL_SPEED }, BODY.offered, 'metric')).toMatchObject({
      sport: 'run',
      anchor: 'critical_speed',
      value: '4:10',
    });
    const offered = [
      { sport: 'ride' as const, anchors: [] },
      { sport: 'swim' as const, anchors: [{ anchor: 'critical_speed' as const, unit: 'metres_per_second' as const }] },
    ];
    expect(initialAnchorForm({ mode: 'add' }, offered, 'metric')).toEqual(
      newAnchorForm('swim', 'critical_speed', 'metres_per_second'),
    );
    expect(initialAnchorForm({ mode: 'add' }, [], 'metric')).toBeNull();
  });

  it('names the unit a value is typed in: watts, bpm, or a pace per its distance', () => {
    const t = (key: string, options?: Record<string, unknown>) => `${key}|${String(options?.distance)}`;
    expect(anchorEntryUnit(t, 'watts', null)).toBe('W');
    expect(anchorEntryUnit(t, 'beats_per_minute', null)).toBe('bpm');
    expect(anchorEntryUnit(t, 'metres_per_second', 'mi')).toBe('physiology.edit.paceUnit|mi');
    expect(anchorEntryUnit(t, 'metres_per_second', '100m')).toBe('physiology.edit.paceUnit|100 m');
  });

  it('refuses a value that is no number, a pace that is no pace, and an unanswered kind', () => {
    const watts = { ...newAnchorForm('ride', 'ftp', 'watts'), value: 'fast', kind: 'measured' as const };
    expect(readAnchorForm(watts, 'metric')).toEqual({ ok: false, problem: { field: 'value', reason: 'invalidNumber' } });
    const pace = { ...newAnchorForm('swim', 'critical_speed', 'metres_per_second'), value: '80', kind: 'measured' as const };
    expect(readAnchorForm(pace, 'metric')).toEqual({ ok: false, problem: { field: 'value', reason: 'invalidPace' } });
    const unasked = { ...newAnchorForm('ride', 'ftp', 'watts'), value: '285' };
    expect(readAnchorForm(unasked, 'metric')).toEqual({ ok: false, problem: { field: 'kind', reason: 'kindRequired' } });
  });
});

describe('the saves', () => {
  it("puts the athlete's re-read anchors in the pane's cache, in the app language", async () => {
    const updatePhysiologyAnchor = vi.fn().mockResolvedValue(BODY);
    const usePhysiologyEdit = createPhysiologyEditHook({ updatePhysiologyAnchor }, { useLanguage: () => 'fr' });
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const { result } = renderHook(() => usePhysiologyEdit(), { wrapper: wrapperFor(client, 'metric') });
    const edit: PhysiologyAnchorEdit = {
      value: 250,
      low: null,
      high: null,
      pace_per: 'km',
      kind: 'measured',
      confidence: 'low',
      method: null,
      measured_on: null,
    };

    await act(() => result.current.mutateAsync({ sport: 'run', anchor: 'critical_speed', edit }));

    expect(updatePhysiologyAnchor).toHaveBeenCalledWith('run', 'critical_speed', edit, 'fr');
    expect(client.getQueryData(QUERY_KEYS.user.physiology('fr', 'metric'))).toEqual(BODY);
  });

  it("reads a coached athlete's thresholds, a withdrawn consent as its own state, and caches the coach's save", async () => {
    const getAthletePhysiology = vi
      .fn()
      .mockResolvedValueOnce(BODY)
      .mockRejectedValueOnce(
        Object.assign(new Error('Request failed with status code 403'), {
          response: { status: 403, data: { details: { reason: 'coach_sharing_revoked' } } },
        }),
      );
    const updateAthletePhysiologyAnchor = vi.fn().mockResolvedValue(BODY);
    const hooks = createCoachPhysiologyHooks(
      { getAthletePhysiology, updateAthletePhysiologyAnchor },
      { useLanguage: () => 'en' },
    );
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const wrapper = wrapperFor(client, 'imperial');

    const read = renderHook(() => hooks.useAthletePhysiology('athlete-1', 'group-1'), { wrapper });
    await waitFor(() => expect(read.result.current.read).toEqual({ status: 'ok', physiology: BODY }));
    expect(getAthletePhysiology).toHaveBeenCalledWith('athlete-1', 'group-1', 'en');

    const revoked = renderHook(() => hooks.useAthletePhysiology('athlete-2', 'group-1'), { wrapper });
    await waitFor(() => expect(revoked.result.current.read).toEqual({ status: 'consentRevoked' }));

    const save = renderHook(() => hooks.useAthletePhysiologyEdit('athlete-1', 'group-1'), { wrapper });
    const edit: PhysiologyAnchorEdit = {
      value: 290,
      low: null,
      high: null,
      pace_per: null,
      kind: 'estimated',
      confidence: 'low',
      method: null,
      measured_on: null,
    };
    await act(() => save.result.current.mutateAsync({ sport: 'ride', anchor: 'ftp', edit }));
    expect(updateAthletePhysiologyAnchor).toHaveBeenCalledWith('athlete-1', 'group-1', 'ride', 'ftp', edit, 'en');
    expect(client.getQueryData(QUERY_KEYS.chat.athletePhysiology('athlete-1', 'group-1', 'en', 'imperial'))).toEqual({
      status: 'ok',
      physiology: BODY,
    });
  });

  it('reads nothing until the view names an athlete', () => {
    const getAthletePhysiology = vi.fn();
    const hooks = createCoachPhysiologyHooks(
      { getAthletePhysiology, updateAthletePhysiologyAnchor: vi.fn() },
      { useLanguage: () => 'en' },
    );
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    renderHook(() => hooks.useAthletePhysiology(null, 'group-1'), { wrapper: wrapperFor(client, 'metric') });
    expect(getAthletePhysiology).not.toHaveBeenCalled();
  });
});
