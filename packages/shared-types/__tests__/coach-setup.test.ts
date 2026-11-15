// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the pending coach's checklist wire shape and its parser (carnet#1529)
// ABOUTME: A malformed row, group or athlete is refused rather than read as a step done or open

import { describe, expect, it } from 'vitest';
import { parseCoachSetupResponse } from '../src/coach-setup';

const body = {
  rows: [
    { id: 'seat', order: 1, done: false, current: true },
    { id: 'invite', order: 2, done: true, current: false },
    { id: 'platform', order: 3, done: false, current: false },
  ],
  done_count: 1,
  total: 3,
  groups: [{ id: 'group-1', name: 'Léa · Marathon' }],
  athlete: { name: 'Léa Gagnon', group_id: 'group-1', joined_at: '2026-11-10T09:00:00Z' },
  platform: null,
};

describe('parseCoachSetupResponse', () => {
  it('reads the server answer as it is', () => {
    expect(parseCoachSetupResponse(body)).toEqual(body);
    const fresh = { ...body, groups: [], athlete: null, platform: 'intervals_icu' };
    expect(parseCoachSetupResponse(fresh)).toEqual(fresh);
    const unnamed = { ...body, athlete: { ...body.athlete, name: null } };
    expect(parseCoachSetupResponse(unnamed)?.athlete?.name).toBeNull();
  });

  it('refuses a body with a malformed row, group, athlete or field', () => {
    expect(parseCoachSetupResponse(null)).toBeNull();
    expect(parseCoachSetupResponse({ ...body, rows: 'x' })).toBeNull();
    expect(parseCoachSetupResponse({ ...body, groups: undefined })).toBeNull();
    expect(parseCoachSetupResponse({ ...body, rows: [{ ...body.rows[0], id: 'connect' }] })).toBeNull();
    expect(parseCoachSetupResponse({ ...body, rows: [{ ...body.rows[0], done: 'no' }] })).toBeNull();
    expect(parseCoachSetupResponse({ ...body, groups: [{ id: 1, name: 'x' }] })).toBeNull();
    expect(parseCoachSetupResponse({ ...body, athlete: { name: 'Léa' } })).toBeNull();
    expect(parseCoachSetupResponse({ ...body, athlete: undefined })).toBeNull();
    expect(parseCoachSetupResponse({ ...body, platform: 3 })).toBeNull();
    expect(parseCoachSetupResponse({ ...body, total: '3' })).toBeNull();
  });
});
