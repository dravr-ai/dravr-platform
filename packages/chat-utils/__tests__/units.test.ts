// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the unit conversions both clients print through — kilometres and metres, or miles and feet (carnet#835)
// ABOUTME: Red if a mile, a foot or a pace per mile drifts, or if a figure loses the athlete's decimal notation

import { describe, expect, it } from 'vitest';
import type { RouteClimb } from '@pierre/scene-types';
import {
  climbRange,
  distanceInUnit,
  formatDistance,
  formatElevation,
  formatSignedElevation,
  formatSpeedIn,
  paceSymbol,
  secondsPerDistanceUnit,
} from '../src';

describe('distances', () => {
  it('counts a distance in kilometres or miles', () => {
    expect(distanceInUnit(5_000, 'metric')).toBe(5);
    expect(distanceInUnit(1_609.344, 'imperial')).toBe(1);
  });

  it('prints a distance with its symbol in the athlete notation', () => {
    expect(formatDistance(92_000, 'metric', 1, 'en')).toBe('92.0 km');
    expect(formatDistance(92_000, 'metric', 1, 'fr')).toBe('92,0 km');
    expect(formatDistance(92_000, 'imperial', 1, 'en')).toBe('57.2 mi');
  });
});

describe('elevation', () => {
  it('prints metres or feet to the whole unit', () => {
    expect(formatElevation(120, 'metric', 'en')).toBe('120 m');
    expect(formatElevation(120, 'imperial', 'en')).toBe('394 ft');
  });

  it('signs a climb or a descent, and leaves zero unsigned', () => {
    expect(formatSignedElevation(4.2, 'metric', 'en')).toBe('+4 m');
    expect(formatSignedElevation(-3, 'imperial', 'en')).toBe('-10 ft');
    expect(formatSignedElevation(0.1, 'imperial', 'en')).toBe('0 ft');
  });
});

describe('pace and speed', () => {
  it('reads a pace per kilometre or per mile', () => {
    expect(secondsPerDistanceUnit(1000 / 256, 'metric')).toBeCloseTo(256);
    expect(secondsPerDistanceUnit(1_609.344 / 412, 'imperial')).toBeCloseTo(412);
    expect(paceSymbol('metric')).toBe('/km');
    expect(paceSymbol('imperial')).toBe('/mi');
  });

  it('prints a speed in km/h or mph', () => {
    expect(formatSpeedIn(7.9, 'metric', 'en')).toBe('28.4 km/h');
    expect(formatSpeedIn(7.9, 'imperial', 'de')).toBe('17,7 mph');
  });
});

describe('climbRange', () => {
  const climb = { start_index: 0, end_index: 1 } as RouteClimb;

  it('names a climb by its kilometres, or its miles on imperial units', () => {
    expect(climbRange([4_000, 8_000], climb, 'en', 'metric')).toBe('km 4.0–8.0');
    expect(climbRange([4_023.36, 8_046.72], climb, 'en', 'imperial')).toBe('mi 2.5–5.0');
  });
});
