// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the device health wire — the counts parser's lenient-but-validated reading and the sample count barrage caps
// ABOUTME: A body of another shape reads as null, so the sync stops instead of saving anchors on a guess

import { describe, expect, it } from 'vitest';
import { deviceSamplesCount, parseDeviceHealthCounts, type DeviceSamples } from '../src/device-health';

const COUNTS = { activities: 2, refused: 1, erased: 0, nights: 3, days: 7, self_reports: 4 };

describe('parseDeviceHealthCounts', () => {
  it('reads every count the sync answers', () => {
    expect(parseDeviceHealthCounts(COUNTS)).toEqual(COUNTS);
  });

  it('reads an absent refused or erased as zero, as the delete answers', () => {
    expect(parseDeviceHealthCounts({ activities: 1, nights: 0, days: 0, self_reports: 0 })).toEqual({
      activities: 1,
      refused: 0,
      erased: 0,
      nights: 0,
      days: 0,
      self_reports: 0,
    });
  });

  it('reads a body missing a required count, or holding a bad one, as null', () => {
    expect(parseDeviceHealthCounts({ activities: 1, nights: 0, days: 0 })).toBeNull();
    expect(parseDeviceHealthCounts({ ...COUNTS, nights: -1 })).toBeNull();
    expect(parseDeviceHealthCounts({ ...COUNTS, days: 1.5 })).toBeNull();
    expect(parseDeviceHealthCounts({ ...COUNTS, activities: '2' })).toBeNull();
    expect(parseDeviceHealthCounts({ ...COUNTS, refused: null })).toBeNull();
    expect(parseDeviceHealthCounts({ ...COUNTS, erased: '0' })).toBeNull();
    expect(parseDeviceHealthCounts([COUNTS])).toBeNull();
    expect(parseDeviceHealthCounts(null)).toBeNull();
  });
});

describe('deviceSamplesCount', () => {
  it('counts every list, windows and deletions included', () => {
    const window = { from: '2026-09-29T16:00:00Z', to: '2026-09-30T16:00:00Z' };
    const samples: DeviceSamples = {
      source: 'health_connect',
      workouts: [],
      categories: [],
      quantities: [],
      days: [],
      states_of_mind: [],
      sleep_windows: [window, window],
      quantity_windows: [{ type: 'WeightRecord', ...window, utc_offset_seconds: -14400 }],
      deleted: { workouts: ['a', 'b'], self_reports: ['c'] },
    };
    expect(deviceSamplesCount(samples)).toBe(6);
  });
});
