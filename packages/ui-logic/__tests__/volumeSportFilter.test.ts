// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins Home's volume-card sport choice, the one the activity list filters on
// ABOUTME: Red if a sport the weeks no longer hold keeps filtering the list, or no weeks yet filters anything

import { describe, expect, it } from 'vitest';
import { act, renderHook } from '@testing-library/react';
import { ALL_SPORTS } from '@pierre/shared-constants';
import type { VolumeWeekInput } from '@pierre/domain-utils';
import { useVolumeSportFilter } from '../src/volumeSportFilter';

const entry = (sport_type: string, duration_seconds: number) => ({
  sport_type,
  activities: 1,
  distance_meters: 0,
  duration_seconds,
  elevation_gain_meters: 0,
});

const RUN_AND_ROW: VolumeWeekInput[] = [{ week_start: '2026-10-19', sports: [entry('run', 7_200), entry('rowing', 1_800)] }];
const RUN_ONLY: VolumeWeekInput[] = [{ week_start: '2026-10-26', sports: [entry('run', 3_600)] }];

describe('useVolumeSportFilter', () => {
  it('filters on nothing until the weeks are read', () => {
    const { result } = renderHook(() => useVolumeSportFilter(undefined));
    expect(result.current).toMatchObject({ sports: [], selected: ALL_SPORTS, sport: null });
  });

  it('filters on the picked sport, and on every sport again on All', () => {
    const { result } = renderHook(() => useVolumeSportFilter(RUN_AND_ROW));
    expect(result.current.sports).toEqual(['run', 'rowing']);

    act(() => result.current.choose('rowing'));
    expect(result.current).toMatchObject({ selected: 'rowing', sport: 'rowing' });

    act(() => result.current.choose(ALL_SPORTS));
    expect(result.current).toMatchObject({ selected: ALL_SPORTS, sport: null });
  });

  it('falls back to every sport when the picked one drops out of the weeks', () => {
    const { result, rerender } = renderHook(({ weeks }) => useVolumeSportFilter(weeks), {
      initialProps: { weeks: RUN_AND_ROW },
    });
    act(() => result.current.choose('rowing'));

    rerender({ weeks: RUN_ONLY });
    expect(result.current).toMatchObject({ selected: ALL_SPORTS, sport: null });
  });
});
