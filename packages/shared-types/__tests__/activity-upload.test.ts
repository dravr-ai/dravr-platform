// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the activity upload parser — a 201 body reads through, a malformed row or an empty answer rejects it
// ABOUTME: Red if the client would show an upload as done from a body that stored nothing

import { describe, expect, it } from 'vitest';
import { parseActivityUploadResponse } from '../src/activity-upload';

const ROW = {
  id: `${'a'.repeat(64)}-0`,
  provider: 'upload',
  name: '',
  sport_type: 'ride',
  start_date: '2026-10-07T07:00:00Z',
  duration_seconds: 600,
  distance_meters: 4792,
  elevation_gain_meters: null,
  has_gps: true,
  summary_polyline: null,
  attribution: null,
};

describe('parseActivityUploadResponse', () => {
  it('reads a 201 body as sent', () => {
    const body = { activities: [ROW], already_held: [{ provider: 'strava', id: '42' }] };
    expect(parseActivityUploadResponse(body)).toEqual(body);
  });

  it('rejects an answer that stored no activity', () => {
    expect(parseActivityUploadResponse({ activities: [], already_held: [] })).toBeNull();
  });

  it('rejects a malformed row rather than dropping it', () => {
    const body = { activities: [ROW, { ...ROW, start_date: 'yesterday' }], already_held: [] };
    expect(parseActivityUploadResponse(body)).toBeNull();
  });

  it('rejects a held copy with no id, and a body without the held list', () => {
    expect(parseActivityUploadResponse({ activities: [ROW], already_held: [{ provider: 'strava' }] })).toBeNull();
    expect(parseActivityUploadResponse({ activities: [ROW] })).toBeNull();
  });
});
