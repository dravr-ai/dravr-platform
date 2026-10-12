// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for the shared gear ledger — the form's bounds, the distance fields in km or mi, the picker's choices and the hooks' cache moves
// ABOUTME: A refused field is named before a request, an edit sends only what changed, and a choice moves the pair's distance

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { createElement, type ReactNode } from 'react';
import { act, renderHook, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import type { ActivityGearResponse, GearItem, GearListResponse } from '@pierre/shared-types';
import {
  GEAR_FORM_KEYS,
  checkGearDraft,
  createGearHooks,
  distanceInputFromMeters,
  emptyGearDraft,
  formatGearDistance,
  gearChanges,
  gearChoices,
  gearDistanceLimit,
  gearDraftOf,
  metersFromDistanceInput,
  splitGear,
} from '../src/gearLedger';

function pair(id: string, overrides: Partial<GearItem> = {}): GearItem {
  return {
    id,
    kind: 'shoe',
    name: `Pair ${id}`,
    sport: 'run',
    initial_distance_meters: 0,
    warn_distance_meters: 500_000,
    distance_meters: 0,
    activity_count: 0,
    warn_reached: false,
    pinned: false,
    is_default: false,
    added_at: '2026-09-01T07:00:00Z',
    retired_at: null,
    last_used_at: null,
    ...overrides,
  };
}

describe('distance fields', () => {
  it('reads whole and decimal kilometres, a comma as the decimal mark', () => {
    expect(metersFromDistanceInput('120', 'metric')).toBe(120_000);
    expect(metersFromDistanceInput(' 42.5 ', 'metric')).toBe(42_500);
    expect(metersFromDistanceInput('42,5', 'metric')).toBe(42_500);
  });

  it('reads miles for an athlete who reads imperial', () => {
    expect(metersFromDistanceInput('1', 'imperial')).toBe(1609);
    expect(metersFromDistanceInput('310,7', 'imperial')).toBe(500_023);
  });

  it('refuses what is not a number', () => {
    for (const typed of ['', 'abc', '-3', '1e3', '12.5.1']) {
      expect(metersFromDistanceInput(typed, 'metric')).toBeNull();
    }
  });

  it('shows a stored distance with at most one decimal, in the athlete units', () => {
    expect(distanceInputFromMeters(500_000, 'metric')).toBe('500');
    expect(distanceInputFromMeters(42_195, 'metric')).toBe('42.2');
    expect(distanceInputFromMeters(500_000, 'imperial')).toBe('310.7');
  });

  it('prints a pair distance in whole kilometres or miles, grouped as the language groups them', () => {
    expect(formatGearDistance(512_340, 'metric', 'en')).toBe('512 km');
    expect(formatGearDistance(1_234_567, 'metric', 'en')).toBe('1,235 km');
    expect(formatGearDistance(0, 'metric', 'fr')).toBe('0 km');
    expect(formatGearDistance(512_340, 'imperial', 'en')).toBe('318 mi');
  });

  it('names a limit the athlete can type: the server bound rounded down to a whole unit', () => {
    expect(gearDistanceLimit('metric')).toBe(10_000_000);
    expect(formatGearDistance(gearDistanceLimit('imperial'), 'imperial', 'en')).toBe('6,213 mi');
    expect(gearDistanceLimit('imperial')).toBeLessThanOrEqual(10_000_000);
  });
});

describe('checkGearDraft', () => {
  it('turns a new pair form into the request, its warning at 500 km', () => {
    const draft = { ...emptyGearDraft('trail_running', 'metric'), name: '  Speedgoat 6 ', initialDistance: '80' };
    expect(checkGearDraft(draft)).toEqual({
      ok: true,
      request: {
        name: 'Speedgoat 6',
        sport: 'trail_running',
        initial_distance_meters: 80_000,
        warn_distance_meters: 500_000,
        pinned: false,
      },
    });
  });

  it('keeps the 500 km warning exact for an athlete who reads miles and leaves it as shown', () => {
    const draft = { ...emptyGearDraft('run', 'imperial'), name: 'Pegasus', initialDistance: '50' };
    expect(draft.warnDistance).toBe('310.7');
    expect(checkGearDraft(draft)).toEqual({
      ok: true,
      request: {
        name: 'Pegasus',
        sport: 'run',
        initial_distance_meters: 80_467,
        warn_distance_meters: 500_000,
        pinned: false,
      },
    });
  });

  it('names every field out of the server bounds', () => {
    const check = checkGearDraft({
      ...emptyGearDraft('run', 'metric'),
      name: '   ',
      initialDistance: '10001',
      warnDistance: '0',
    });
    expect(check).toEqual({
      ok: false,
      errors: {
        name: GEAR_FORM_KEYS.nameRequired,
        initialDistance: GEAR_FORM_KEYS.distanceInvalid,
        warnDistance: GEAR_FORM_KEYS.warnInvalid,
      },
    });
    const long = checkGearDraft({ ...emptyGearDraft('run', 'metric'), name: 'x'.repeat(81) });
    expect(long).toEqual({ ok: false, errors: { name: GEAR_FORM_KEYS.nameTooLong } });
    const tooFar = checkGearDraft({ ...emptyGearDraft('run', 'imperial'), name: 'Far', initialDistance: '6214' });
    expect(tooFar).toEqual({ ok: false, errors: { initialDistance: GEAR_FORM_KEYS.distanceInvalid } });
  });

  it('sends an edit with only the fields that changed', () => {
    const current = pair('a', { name: 'Pegasus', initial_distance_meters: 120_000, pinned: true });
    const draft = { ...gearDraftOf(current, 'metric'), warnDistance: '650' };
    const check = checkGearDraft(draft);
    expect(check.ok).toBe(true);
    if (check.ok) {
      expect(gearChanges(current, check.request)).toEqual({ warn_distance_meters: 650_000 });
    }
  });

  it('sends nothing for a pair opened in miles and saved as shown, though its figures were rounded', () => {
    const current = pair('a', { initial_distance_meters: 123_456, warn_distance_meters: 500_000 });
    const draft = gearDraftOf(current, 'imperial');
    expect(draft.initialDistance).toBe('76.7');
    const check = checkGearDraft(draft);
    expect(check.ok).toBe(true);
    if (check.ok) {
      expect(gearChanges(current, check.request)).toEqual({});
    }
  });
});

describe('the pairs offered', () => {
  const ledger = [pair('new'), pair('old', { retired_at: '2026-09-20T00:00:00Z' }), pair('mid')];

  it('splits the ledger into pairs in use and retired ones, in its order', () => {
    const { inUse, retired } = splitGear(ledger);
    expect(inUse.map((p) => p.id)).toEqual(['new', 'mid']);
    expect(retired.map((p) => p.id)).toEqual(['old']);
  });

  it('offers the pairs in use, and a retired pair only when the activity is attributed to it', () => {
    expect(gearChoices(ledger, null).map((p) => p.id)).toEqual(['new', 'mid']);
    expect(gearChoices(ledger, 'old').map((p) => p.id)).toEqual(['new', 'old', 'mid']);
  });
});

describe('createGearHooks', () => {
  const api = {
    listGear: vi.fn<() => Promise<GearListResponse>>(),
    createGear: vi.fn(),
    updateGear: vi.fn(),
    retireGear: vi.fn(),
    restoreGear: vi.fn(),
    deleteGear: vi.fn(),
    getActivityGear: vi.fn<(provider: string, id: string) => Promise<ActivityGearResponse>>(),
    setActivityGear: vi.fn<(provider: string, id: string, gearId: string | null) => Promise<ActivityGearResponse>>(),
  };
  const hooks = createGearHooks(api);

  function setup() {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
    const wrapper = ({ children }: { children: ReactNode }) =>
      createElement(QueryClientProvider, { client }, children);
    return { client, wrapper };
  }

  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('reads the activity pair, and a choice is shown at once and settles the ledger again', async () => {
    api.getActivityGear.mockResolvedValue({ gear_id: 'a', source: 'default' });
    api.setActivityGear.mockResolvedValue({ gear_id: null, source: 'declared' });
    api.listGear.mockResolvedValue({ gear: [pair('a')] });
    const { client, wrapper } = setup();
    const ledger = renderHook(() => hooks.useGearLedger(), { wrapper });
    await waitFor(() => expect(ledger.result.current.data).toBeDefined());
    const { result } = renderHook(() => hooks.useActivityGear('strava', '42', true), { wrapper });
    await waitFor(() => expect(result.current.attribution).toEqual({ gear_id: 'a', source: 'default' }));

    act(() => result.current.choose(null));

    await waitFor(() => expect(result.current.attribution).toEqual({ gear_id: null, source: 'declared' }));
    expect(api.setActivityGear).toHaveBeenCalledWith('strava', '42', null);
    expect(client.getQueryData(QUERY_KEYS.gear.activity('strava', '42'))).toEqual({
      gear_id: null,
      source: 'declared',
    });
    await waitFor(() => expect(api.listGear).toHaveBeenCalledTimes(2));
  });

  it('reads nothing for an activity the picker is not shown on', () => {
    const { wrapper } = setup();
    renderHook(() => hooks.useActivityGear('strava', 'ride-1', false), { wrapper });
    expect(api.getActivityGear).not.toHaveBeenCalled();
  });

  it('reads the ledger again after a pair is retired', async () => {
    api.listGear.mockResolvedValue({ gear: [pair('a')] });
    api.retireGear.mockResolvedValue(pair('a', { retired_at: '2026-10-09T00:00:00Z' }));
    const { wrapper } = setup();
    const ledger = renderHook(() => hooks.useGearLedger(), { wrapper });
    await waitFor(() => expect(ledger.result.current.data).toBeDefined());
    const { result } = renderHook(() => hooks.useGearChanges(), { wrapper });

    await act(() => result.current.retire.mutateAsync('a'));

    expect(api.retireGear).toHaveBeenCalledWith('a');
    await waitFor(() => expect(api.listGear).toHaveBeenCalledTimes(2));
  });
});
