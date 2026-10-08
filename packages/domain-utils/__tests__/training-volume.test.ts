// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the weekly volume helpers — the sports in time order, a week's totals per filter, the metric choice, the bars
// ABOUTME: The web and mobile cards both read through these, so a week sums and draws the same on each

import { describe, expect, it } from 'vitest';
import {
  metricValue,
  projectVolumeBars,
  volumeBarAt,
  volumeMetric,
  volumeSports,
  weekTotals,
  type VolumeWeekInput,
} from '../src/training-volume';

const RUN = { sport_type: 'run', activities: 2, distance_meters: 15_000, duration_seconds: 5_400, elevation_gain_meters: 80 };
const RIDE = { sport_type: 'ride', activities: 1, distance_meters: 40_000, duration_seconds: 7_200, elevation_gain_meters: 450 };
const LIFT = { sport_type: 'strength_training', activities: 1, distance_meters: 0, duration_seconds: 3_600, elevation_gain_meters: 0 };

const WEEKS: VolumeWeekInput[] = [
  { week_start: '2026-09-21', sports: [RUN] },
  { week_start: '2026-09-28', sports: [] },
  { week_start: '2026-10-05', sports: [RIDE, RUN, LIFT] },
];

describe('volumeSports', () => {
  it('orders the sports by total time, most first', () => {
    // run 10 800 s, ride 7 200 s, strength 3 600 s.
    expect(volumeSports(WEEKS)).toEqual(['run', 'ride', 'strength_training']);
  });

  it('is empty when no week holds training', () => {
    expect(volumeSports([{ week_start: '2026-10-05', sports: [] }])).toEqual([]);
  });
});

describe('weekTotals', () => {
  it('sums every sport when no filter is set', () => {
    expect(weekTotals(WEEKS[2], null)).toEqual({
      activities: 4,
      distance_meters: 55_000,
      duration_seconds: 16_200,
      elevation_gain_meters: 530,
    });
  });

  it('keeps only the filtered sport', () => {
    expect(weekTotals(WEEKS[2], 'ride')).toEqual({
      activities: 1,
      distance_meters: 40_000,
      duration_seconds: 7_200,
      elevation_gain_meters: 450,
    });
  });

  it('answers zeros for a week without the sport', () => {
    expect(weekTotals(WEEKS[1], 'run').activities).toBe(0);
  });
});

describe('volumeMetric', () => {
  it('draws distance when a week recorded one', () => {
    const totals = WEEKS.map((week) => weekTotals(week, null));
    expect(volumeMetric(totals)).toBe('distance');
    expect(metricValue(totals[2], 'distance')).toBe(55_000);
  });

  it('draws time for a sport logged without distance', () => {
    const totals = WEEKS.map((week) => weekTotals(week, 'strength_training'));
    expect(volumeMetric(totals)).toBe('duration');
    expect(metricValue(totals[2], 'duration')).toBe(3_600);
  });
});

describe('projectVolumeBars', () => {
  const box = { width: 100, height: 50, padding: 5 };

  it('stands the tallest bar to the full height and scales the rest', () => {
    const bars = projectVolumeBars([10, 0, 5], box);
    expect(bars).toHaveLength(3);
    expect(bars[0].height).toBeCloseTo(40);
    expect(bars[0].y).toBeCloseTo(5);
    expect(bars[1].height).toBe(0);
    expect(bars[1].y).toBe(45);
    expect(bars[2].height).toBeCloseTo(20);
    // Evenly spaced slots of 30, each bar 70% of its slot, centred.
    expect(bars[0].width).toBeCloseTo(21);
    expect(bars[0].x).toBeCloseTo(9.5);
    expect(bars[2].x).toBeCloseTo(69.5);
  });

  it('draws no height when every week is zero', () => {
    expect(projectVolumeBars([0, 0], box).every((bar) => bar.height === 0)).toBe(true);
  });

  it('is empty without values', () => {
    expect(projectVolumeBars([], box)).toEqual([]);
  });
});

describe('volumeBarAt', () => {
  const box = { width: 100, height: 50, padding: 5 };

  it('finds the slot under a point and clamps to the ends', () => {
    expect(volumeBarAt(3, 10, box)).toBe(0);
    expect(volumeBarAt(3, 50, box)).toBe(1);
    expect(volumeBarAt(3, 94, box)).toBe(2);
    expect(volumeBarAt(3, -20, box)).toBe(0);
    expect(volumeBarAt(3, 200, box)).toBe(2);
  });

  it('has nothing to find without bars', () => {
    expect(volumeBarAt(0, 10, box)).toBeNull();
  });
});
