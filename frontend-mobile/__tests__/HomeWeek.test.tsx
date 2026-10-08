// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Home week over a mocked calendar read — past days' workouts, paging to stored history and the plan's end, the month sheet
// ABOUTME: Red if a day before history reads as empty, no plan hides the strip, paging runs past its edges, or a workout cannot be opened

import React from 'react';
import { fireEvent, render, waitFor, within } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { CalendarResponse, HomeActivity, TrainingPlanResponse } from '@pierre/shared-types';

import {
  ACTIVITIES,
  NO_PLAN_RESPONSE,
  PLAN_RESPONSE,
  calendarAnswer,
} from '../integration/app/helpers/homeFixtures';

const mockGetCalendar = jest.fn<Promise<CalendarResponse>, [string, string]>();

jest.mock('../src/services/api', () => ({
  athleteApi: { getCalendar: (from: string, to: string) => mockGetCalendar(from, to) },
}));

import { HomeWeek } from '../src/screens/home/HomeWeek';

function renderWeek(response: TrainingPlanResponse = PLAN_RESPONSE) {
  const openDraft = jest.fn();
  const openActivity = jest.fn<void, [HomeActivity]>();
  const plan = response.plan;
  mockGetCalendar.mockImplementation(async (from, to) => calendarAnswer(from, to, { plan }));
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } });
  const screen = render(
    <QueryClientProvider client={client}>
      <HomeWeek response={response} openDraft={openDraft} openActivity={openActivity} />
    </QueryClientProvider>,
  );
  return { screen, openDraft, openActivity };
}

/** Wait until the calendar answer for `monday`'s week is drawn on the strip. */
async function weekOf(screen: ReturnType<typeof render>, monday: string) {
  await waitFor(() => {
    expect(screen.getByTestId(`home-week-day-${monday}`)).toBeTruthy();
    expect(screen.getByTestId('home-week-strip').props.accessibilityState).toEqual({ busy: false });
  });
}

beforeEach(() => {
  jest.clearAllMocks();
});

describe('the week strip', () => {
  it('asks for this week and opens on today', async () => {
    const { screen } = renderWeek();

    await weekOf(screen, '2026-09-21');
    expect(mockGetCalendar).toHaveBeenCalledWith('2026-09-21', '2026-09-27');
    expect(screen.getByTestId('home-section-week')).toHaveTextContent(/^This week/);
    expect(screen.getByTestId('home-week-day-2026-09-24').props.accessibilityState).toEqual({ selected: true });
    expect(screen.getByTestId('home-week-previous').props.accessibilityState).toEqual({ disabled: false });
  });

  it("shows last week's workouts on their days and opens each one's own view", async () => {
    const { screen, openActivity } = renderWeek();
    await weekOf(screen, '2026-09-21');

    fireEvent.press(screen.getByTestId('home-week-previous'));
    await weekOf(screen, '2026-09-14');
    expect(mockGetCalendar).toHaveBeenLastCalledWith('2026-09-14', '2026-09-20');
    expect(screen.getByTestId('home-section-week')).toHaveTextContent(/^Week of September 14/);

    const sunday = screen.getByTestId('home-week-day-2026-09-20');
    expect(sunday.props.accessibilityLabel).toBe(`Sunday 20, Done: ${ACTIVITIES[0].name}`);
    expect(within(sunday).getByTestId('home-week-mark-done')).toBeTruthy();

    fireEvent.press(sunday);
    const detail = screen.getByTestId('home-week-detail');
    fireEvent.press(within(detail).getByTestId(`calendar-activity-${ACTIVITIES[0].provider}-${ACTIVITIES[0].id}`));
    expect(openActivity).toHaveBeenCalledWith(ACTIVITIES[0]);
  });

  it('stops at the week holding the first stored day and says where history starts', async () => {
    const { screen } = renderWeek();
    await weekOf(screen, '2026-09-21');

    fireEvent.press(screen.getByTestId('home-week-previous'));
    await weekOf(screen, '2026-09-14');
    fireEvent.press(screen.getByTestId('home-week-previous'));
    await weekOf(screen, '2026-09-07');

    expect(screen.getByTestId('home-week-day-2026-09-08').props.accessibilityLabel).toBe(
      'Tuesday 8, This day is older than your stored activities.',
    );
    expect(screen.getByTestId('home-week-history-start')).toHaveTextContent(
      "Your stored activities start on Wednesday, September 9; earlier days aren't kept.",
    );
    expect(screen.getByTestId('home-week-previous').props.accessibilityState).toEqual({ disabled: true });

    fireEvent.press(screen.getByTestId('home-week-today'));
    await weekOf(screen, '2026-09-21');
    expect(screen.getByTestId('home-section-week')).toHaveTextContent(/^This week/);
  });

  it("pages forward only as far as the plan's deferred weeks reach", async () => {
    const { screen } = renderWeek();
    await weekOf(screen, '2026-09-21');

    // Two weeks shown and two deferred after them: the last is 12 October.
    for (const monday of ['2026-09-28', '2026-10-05', '2026-10-12']) {
      fireEvent.press(screen.getByTestId('home-week-next'));
      await weekOf(screen, monday);
    }
    expect(mockGetCalendar).toHaveBeenLastCalledWith('2026-10-12', '2026-10-18');
    expect(screen.getByTestId('home-week-next').props.accessibilityState).toEqual({ disabled: true });
  });

  it('renders without a plan: what was done, one quiet line, the week ahead', async () => {
    const { screen } = renderWeek(NO_PLAN_RESPONSE);
    await weekOf(screen, '2026-09-21');

    expect(screen.getByTestId('home-week-no-plan')).toHaveTextContent(
      "No plan yet, so only what you've done shows here.",
    );
    expect(screen.queryByTestId('home-next-week')).toBeNull();
    fireEvent.press(screen.getByTestId('home-week-next'));
    await weekOf(screen, '2026-09-28');
    expect(screen.getByTestId('home-week-next').props.accessibilityState).toEqual({ disabled: true });
  });

  it('says a week that failed to load failed, and reads it again on retry', async () => {
    mockGetCalendar.mockRejectedValueOnce(new Error('offline'));
    const { screen } = renderWeek();

    expect(await screen.findByTestId('home-week-failed')).toHaveTextContent(/These days couldn't be loaded\./);
    fireEvent.press(screen.getByTestId('home-week-retry'));
    await weekOf(screen, '2026-09-21');
  });
});

describe('the month sheet', () => {
  it('lays the month out with workouts done and sessions planned, and a day opens to both', async () => {
    const { screen, openActivity } = renderWeek();
    await weekOf(screen, '2026-09-21');

    fireEvent.press(screen.getByTestId('home-week-month'));
    expect(await screen.findByTestId('home-month-title')).toHaveTextContent('September 2026');
    await waitFor(() => expect(mockGetCalendar).toHaveBeenLastCalledWith('2026-08-31', '2026-10-04'));
    await waitFor(() =>
      expect(within(screen.getByTestId('home-month-day-2026-09-20')).getByTestId('home-week-mark-done')).toBeTruthy(),
    );
    expect(within(screen.getByTestId('home-month-day-2026-09-24')).getByTestId('home-week-mark-session')).toBeTruthy();
    expect(screen.getByTestId('home-month-history-start')).toBeTruthy();
    expect(screen.getByTestId('home-month-previous').props.accessibilityState).toEqual({ disabled: true });

    fireEvent.press(screen.getByTestId('home-month-day-2026-09-17'));
    fireEvent.press(
      within(screen.getByTestId('home-month-detail')).getByTestId(
        `calendar-activity-${ACTIVITIES[1].provider}-${ACTIVITIES[1].id}`,
      ),
    );
    expect(openActivity).toHaveBeenCalledWith(ACTIVITIES[1]);
  });

  it('pages to the next month', async () => {
    const { screen } = renderWeek();
    await weekOf(screen, '2026-09-21');

    fireEvent.press(screen.getByTestId('home-week-month'));
    await screen.findByTestId('home-month-title');
    fireEvent.press(screen.getByTestId('home-month-next'));
    expect(screen.getByTestId('home-month-title')).toHaveTextContent('October 2026');
    await waitFor(() => expect(mockGetCalendar).toHaveBeenLastCalledWith('2026-09-28', '2026-11-01'));
  });
});
