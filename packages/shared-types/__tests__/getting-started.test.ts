// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the Getting started wire shape and its parser (carnet#1099)
// ABOUTME: A malformed row or a missing field is refused rather than read as a row done or open

import { describe, expect, it } from 'vitest';
import { parseGettingStartedResponse } from '../src/getting-started';

const body = {
  rows: [
    { id: 'connect', order: 1, eligible: true, done: true, current: false, postback: null },
    { id: 'meet_team', order: 2, eligible: true, done: false, current: true, postback: 'gs:1:meet_team' },
  ],
  done_count: 1,
  total: 5,
  folded_at: null,
  hidden: false,
};

describe('parseGettingStartedResponse', () => {
  it('reads the server answer as it is', () => {
    expect(parseGettingStartedResponse(body)).toEqual(body);
    const folded = { ...body, folded_at: '2026-10-21T08:00:00Z' };
    expect(parseGettingStartedResponse(folded)?.folded_at).toBe('2026-10-21T08:00:00Z');
  });

  it('refuses a body with a malformed row or field', () => {
    expect(parseGettingStartedResponse(null)).toBeNull();
    expect(parseGettingStartedResponse({ ...body, rows: 'x' })).toBeNull();
    expect(parseGettingStartedResponse({ ...body, hidden: undefined })).toBeNull();
    expect(parseGettingStartedResponse({ ...body, folded_at: 3 })).toBeNull();
    expect(
      parseGettingStartedResponse({ ...body, rows: [{ ...body.rows[0], postback: 1 }] }),
    ).toBeNull();
    expect(parseGettingStartedResponse({ ...body, rows: [{ ...body.rows[1], done: 'no' }] })).toBeNull();
  });
});
