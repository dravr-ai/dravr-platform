// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the gear ledger parsers — a served pair and an activity's pair read through, any other shape rejects the body
// ABOUTME: Red if a screen would draw a pair's distance or warning from a body that does not carry them

import { describe, expect, it } from 'vitest';
import {
  GEAR_SPORTS,
  isGearSport,
  parseActivityGearResponse,
  parseGearItem,
  parseGearListResponse,
} from '../src/gear';

const PAIR = {
  id: '6f1c2a5e-3b1d-4c6a-9a7e-2f4b5c6d7e8f',
  kind: 'shoe',
  name: 'Pegasus 41',
  sport: 'run',
  initial_distance_meters: 120_000,
  warn_distance_meters: 500_000,
  distance_meters: 512_300,
  activity_count: 41,
  warn_reached: true,
  pinned: false,
  is_default: true,
  added_at: '2026-08-01T07:00:00Z',
  retired_at: null,
  last_used_at: '2026-10-08T06:30:00Z',
};

describe('parseGearListResponse', () => {
  it('reads a served ledger as sent, retired pairs included', () => {
    const retired = { ...PAIR, id: 'b2', retired_at: '2026-09-30T10:00:00Z', is_default: false };
    const body = { gear: [PAIR, retired] };
    expect(parseGearListResponse(body)).toEqual(body);
  });

  it('reads an empty ledger', () => {
    expect(parseGearListResponse({ gear: [] })).toEqual({ gear: [] });
  });

  it('rejects the whole ledger when one pair is malformed, rather than dropping it', () => {
    expect(parseGearListResponse({ gear: [PAIR, { ...PAIR, distance_meters: -1 }] })).toBeNull();
    expect(parseGearListResponse({ pairs: [PAIR] })).toBeNull();
  });
});

describe('parseGearItem', () => {
  it('rejects a sport no pair is declared for, a fractional count and a missing flag', () => {
    expect(parseGearItem({ ...PAIR, sport: 'ride' })).toBeNull();
    expect(parseGearItem({ ...PAIR, activity_count: 1.5 })).toBeNull();
    const withoutWarning: Record<string, unknown> = { ...PAIR };
    delete withoutWarning.warn_reached;
    expect(parseGearItem(withoutWarning)).toBeNull();
  });

  it('rejects an unreadable instant', () => {
    expect(parseGearItem({ ...PAIR, added_at: 'yesterday' })).toBeNull();
    expect(parseGearItem({ ...PAIR, last_used_at: 'soon' })).toBeNull();
  });
});

describe('parseActivityGearResponse', () => {
  it('reads a default, a declared pair, a declared none and nothing attributed', () => {
    for (const body of [
      { gear_id: PAIR.id, source: 'default' },
      { gear_id: PAIR.id, source: 'declared' },
      { gear_id: null, source: 'declared' },
      { gear_id: null, source: null },
    ]) {
      expect(parseActivityGearResponse(body)).toEqual(body);
    }
  });

  it('rejects a pair with no source and an unknown source', () => {
    expect(parseActivityGearResponse({ gear_id: PAIR.id, source: null })).toBeNull();
    expect(parseActivityGearResponse({ gear_id: PAIR.id, source: 'strava' })).toBeNull();
    expect(parseActivityGearResponse({ gear_id: '', source: 'declared' })).toBeNull();
  });
});

describe('GEAR_SPORTS', () => {
  it('names the four sports the server declares a pair for', () => {
    expect([...GEAR_SPORTS]).toEqual(['run', 'trail_running', 'walk', 'hike']);
    expect(isGearSport('trail_running')).toBe(true);
    expect(isGearSport('virtual_run')).toBe(false);
  });
});
