// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the preferred-calendar parser — a chosen calendar or null, its connected flag and the calendars to choose from
// ABOUTME: Red if a body the server never sends is read as a choice, which would show the athlete a calendar they never picked

import { describe, expect, it } from 'vitest';
import { parsePreferredCalendar } from '../src/preferred-calendar';

const intervals = { provider: 'intervals_icu', name: 'Intervals.icu' };
const garmin = { provider: 'garmin', name: 'Garmin Connect' };

describe('parsePreferredCalendar', () => {
  it('reads a choice among the connected calendars, and null for an athlete who chose none', () => {
    const chosen = { chosen: intervals, chosen_connected: true, calendars: [intervals, garmin] };
    expect(parsePreferredCalendar(chosen)).toEqual(chosen);
    const none = { chosen: null, chosen_connected: false, calendars: [intervals, garmin] };
    expect(parsePreferredCalendar(none)).toEqual(none);
  });

  it('reads a choice that is no longer connected, with no calendars left', () => {
    const gone = { chosen: garmin, chosen_connected: false, calendars: [] };
    expect(parsePreferredCalendar(gone)).toEqual(gone);
  });

  it('rejects a malformed calendar, a missing key or a body that is not an object', () => {
    expect(
      parsePreferredCalendar({ chosen: { provider: '', name: 'x' }, chosen_connected: true, calendars: [] })
    ).toBeNull();
    expect(
      parsePreferredCalendar({ chosen: null, chosen_connected: false, calendars: [{ provider: 'wahoo' }] })
    ).toBeNull();
    expect(parsePreferredCalendar({ chosen: null, calendars: [] })).toBeNull();
    expect(parsePreferredCalendar({ chosen: null, chosen_connected: 'no', calendars: [] })).toBeNull();
    expect(parsePreferredCalendar({ chosen: null, chosen_connected: false })).toBeNull();
    expect(parsePreferredCalendar(null)).toBeNull();
    expect(parsePreferredCalendar([intervals])).toBeNull();
  });
});
