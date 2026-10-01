// ABOUTME: Unit tests for the activity-detail wire parser — one workout's Home row, figures, splits and laps
// ABOUTME: Feeds it the body the server sends, then the malformed ones it must refuse rather than half-render

import { describe, it, expect } from 'vitest';
import { parseActivityDetailResponse } from '../src/home';

/** The body `GET /api/me/activities/strava/tempo-10k` answers: every key present. */
const body = {
  activity: {
    id: 'tempo-10k',
    provider: 'strava',
    name: 'Tempo 10k',
    sport_type: 'run',
    start_date: '2026-09-29T10:00:00Z',
    duration_seconds: 2700,
    distance_meters: 10000,
    elevation_gain_meters: 64,
    has_gps: true,
    summary_polyline: null,
  },
  average_heart_rate: 158,
  max_heart_rate: 176,
  average_speed_mps: 3.9,
  max_speed_mps: 5.2,
  average_power: 287,
  calories: 712,
  splits: [
    {
      index: 1,
      distance_meters: 1000,
      elapsed_time_seconds: 262,
      moving_time_seconds: 260,
      elevation_difference_meters: -4.2,
      average_speed_mps: 3.85,
      average_heart_rate: null,
    },
  ],
  laps: [
    {
      index: 1,
      distance_meters: 10000,
      elapsed_time_seconds: 2700,
      moving_time_seconds: null,
      elevation_gain_meters: 64,
      average_speed_mps: 3.9,
      average_heart_rate: 158,
      max_heart_rate: 176,
      average_power: null,
    },
  ],
  conversation_id: 'conv-tempo',
};

describe('parseActivityDetailResponse', () => {
  it('reads the body the server sends, field for field', () => {
    expect(parseActivityDetailResponse(body)).toEqual(body);
  });

  it('reads a workout without figures, splits or laps', () => {
    const bare = {
      ...body,
      average_heart_rate: null,
      max_heart_rate: null,
      average_speed_mps: null,
      max_speed_mps: null,
      average_power: null,
      calories: null,
      splits: [],
      laps: [],
      conversation_id: null,
    };
    expect(parseActivityDetailResponse(bare)).toEqual(bare);
  });

  it('refuses a body with a malformed row, figure, split or lap', () => {
    expect(parseActivityDetailResponse({ ...body, activity: { ...body.activity, id: '' } })).toBeNull();
    expect(parseActivityDetailResponse({ ...body, average_heart_rate: '158' })).toBeNull();
    expect(parseActivityDetailResponse({ ...body, calories: -1 })).toBeNull();
    expect(parseActivityDetailResponse({ ...body, splits: [{ ...body.splits[0], index: 1.5 }] })).toBeNull();
    expect(parseActivityDetailResponse({ ...body, laps: [{ ...body.laps[0], distance_meters: 'far' }] })).toBeNull();
    expect(parseActivityDetailResponse({ ...body, splits: null })).toBeNull();
    expect(parseActivityDetailResponse({ ...body, conversation_id: 42 })).toBeNull();
    expect(parseActivityDetailResponse(null)).toBeNull();
  });
});
