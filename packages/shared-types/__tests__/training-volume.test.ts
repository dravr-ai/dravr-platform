// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the weekly volume parser — a well-formed body reads through, a missing or impossible figure rejects the whole body
// ABOUTME: Red if an omitted figure is read as zero, which the page would draw as a week without training

import { describe, expect, it } from 'vitest';
import { parseTrainingVolumeResponse } from '../src/training-volume';

const RUN = { sport_type: 'run', activities: 2, distance_meters: 15_000, duration_seconds: 5_400, elevation_gain_meters: 80 };

function body(sport: Record<string, unknown> = RUN): Record<string, unknown> {
  return {
    today: '2026-10-07',
    weeks: [
      { week_start: '2026-09-28', sports: [] },
      { week_start: '2026-10-05', sports: [sport] },
    ],
    garmin_contribution: null,
  };
}

describe('parseTrainingVolumeResponse', () => {
  it('reads a well-formed body as sent', () => {
    expect(parseTrainingVolumeResponse(body())).toEqual(body());
  });

  it('reads a body with no weeks: nothing is stored yet', () => {
    expect(parseTrainingVolumeResponse({ today: '2026-10-07', weeks: [], garmin_contribution: null })).toEqual({
      today: '2026-10-07',
      weeks: [],
      garmin_contribution: null,
    });
  });

  it('keeps what Garmin contributed to the weeks, and reads an omitted contribution as none', () => {
    for (const contribution of ['sole', 'part'] as const) {
      expect(parseTrainingVolumeResponse({ ...body(), garmin_contribution: contribution })).toEqual({
        ...body(),
        garmin_contribution: contribution,
      });
    }
    const { garmin_contribution: _omitted, ...older } = body();
    expect(parseTrainingVolumeResponse(older)).toEqual(body());
  });

  it('rejects a Garmin contribution it has no line for', () => {
    expect(parseTrainingVolumeResponse({ ...body(), garmin_contribution: 'all' })).toBeNull();
    expect(parseTrainingVolumeResponse({ ...body(), garmin_contribution: false })).toBeNull();
  });

  it('rejects a body whose figure is missing rather than reading it as zero', () => {
    const { distance_meters: _dropped, ...partial } = RUN;
    expect(parseTrainingVolumeResponse(body(partial))).toBeNull();
  });

  it('rejects impossible figures and malformed dates', () => {
    expect(parseTrainingVolumeResponse(body({ ...RUN, duration_seconds: -1 }))).toBeNull();
    expect(parseTrainingVolumeResponse(body({ ...RUN, activities: 1.5 }))).toBeNull();
    expect(parseTrainingVolumeResponse(body({ ...RUN, sport_type: '' }))).toBeNull();
    expect(parseTrainingVolumeResponse({ today: '7 Oct', weeks: [] })).toBeNull();
    expect(parseTrainingVolumeResponse({ today: '2026-10-07', weeks: [{ week_start: 'Monday', sports: [] }] })).toBeNull();
    expect(parseTrainingVolumeResponse({ today: '2026-10-07' })).toBeNull();
  });
});
