// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Home week over a mocked calendar read — past days' workouts, paging to stored history and the plan's end, the month sheet
// ABOUTME: Red if a day before history reads as empty, no plan hides the strip, paging runs past its edges, or a workout cannot be opened

import React from 'react';
import { fireEvent, render, waitFor, within } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { CalendarResponse, GarminContribution, HomeActivity, TrainingPlanResponse } from '@pierre/shared-types';

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

/** Garmin's line for each contribution, as the English corpus words it (carnet#1154). */
const GARMIN_LINES = [
  ['sole', 'Created using data provided by Garmin devices.'],
  ['part', 'Created in part using data provided by Garmin devices.'],
] as const;

function renderWeek(
  response: TrainingPlanResponse = PLAN_RESPONSE,
  {
    activities,
    garminContribution = null,
  }: { activities?: HomeActivity[]; garminContribution?: GarminContribution | null } = {},
) {
  const openDraft = jest.fn();
  const openActivity = jest.fn<void, [HomeActivity]>();
  const plan = response.plan;
  mockGetCalendar.mockImplementation(async (from, to) => ({
    ...calendarAnswer(from, to, { plan, activities }),
    garmin_contribution: garminContribution,
  }));
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

  it.each(GARMIN_LINES)(
    "carries Garmin's line for a %s contribution beneath the week's title, ahead of the plan's focus",
    async (contribution, line) => {
      const { screen } = renderWeek(PLAN_RESPONSE, { garminContribution: contribution });
      await weekOf(screen, '2026-09-21');

      const [title, description] = within(screen.getByTestId('home-section-week')).getAllByText(/./);
      expect(title.props.children).toBe('This week');
      expect(description.props.children).toBe(`${line}\nThreshold`);
    },
  );

  it("carries no Garmin line when no workout in the week's answer carries Garmin data", async () => {
    const { screen } = renderWeek();
    await weekOf(screen, '2026-09-21');

    const [, description] = within(screen.getByTestId('home-section-week')).getAllByText(/./);
    expect(description.props.children).toBe('Threshold');
    expect(screen.queryByText(/Garmin/)).toBeNull();
  });

  it("shows last week's workouts on their days and opens each one's own view", async () => {
    const { screen, openActivity } = renderWeek();
    await weekOf(screen, '2026-09-21');

    fireEvent.press(screen.getByTestId('home-week-previous'));
    await weekOf(screen, '2026-09-14');
    expect(mockGetCalendar).toHaveBeenLastCalledWith('2026-09-14', '2026-09-20');
    expect(screen.getByTestId('home-section-week')).toHaveTextContent(/^Week of September 14/);

    const sunday = screen.getByTestId('home-week-day-2026-09-20');
    expect(sunday.props.accessibilityLabel).toBe(`Sunday, September 20 — Done: ${ACTIVITIES[0].name}`);
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
      'Tuesday, September 8 — This day is older than your stored activities.',
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

  it.each(GARMIN_LINES)(
    "carries Garmin's line for a %s contribution beneath the month's title, ahead of the grid",
    async (contribution, line) => {
      const { screen } = renderWeek(PLAN_RESPONSE, { garminContribution: contribution });
      await weekOf(screen, '2026-09-21');

      fireEvent.press(screen.getByTestId('home-week-month'));
      const title = await screen.findByTestId('home-month-title');
      const garmin = await screen.findByTestId('home-month-garmin');
      expect(garmin.props.children).toBe(line);
      // After the title row, and the last thing read before the grid (its weekday letters are hidden from readers).
      const texts = within(screen.getByTestId('home-month-scroll')).getAllByText(/./);
      const at = texts.indexOf(garmin);
      expect(at).toBeGreaterThan(texts.indexOf(title));
      expect(texts[at + 1]).toBe(within(screen.getByTestId('home-month-grid')).getAllByText(/./)[0]);
    },
  );

  it('carries no Garmin line on the month when no workout in its answer carries Garmin data', async () => {
    const { screen } = renderWeek();
    await weekOf(screen, '2026-09-21');

    fireEvent.press(screen.getByTestId('home-week-month'));
    await screen.findByTestId('home-month-title');
    await waitFor(() =>
      expect(within(screen.getByTestId('home-month-day-2026-09-20')).getByTestId('home-week-mark-done')).toBeTruthy(),
    );
    expect(screen.queryByTestId('home-month-garmin')).toBeNull();
  });

  it("names a workout's device beside its sport in the open day, as served, and nothing for a row without one", async () => {
    const activities = ACTIVITIES.map((activity) =>
      activity.id === 'i77' ? { ...activity, attribution: 'Garmin Forerunner 965' } : activity,
    );
    const { screen } = renderWeek(PLAN_RESPONSE, { activities });
    await weekOf(screen, '2026-09-21');

    fireEvent.press(screen.getByTestId('home-week-month'));
    await screen.findByTestId('home-month-title');
    await waitFor(() =>
      expect(within(screen.getByTestId('home-month-day-2026-09-16')).getByTestId('home-week-mark-done')).toBeTruthy(),
    );
    fireEvent.press(screen.getByTestId('home-month-day-2026-09-16'));
    const row = within(screen.getByTestId('home-month-detail')).getByTestId('calendar-activity-intervals_icu-i77');
    expect(within(row).getByTestId('calendar-activity-attribution').props.children).toBe(' · Garmin Forerunner 965');
    expect(row).toHaveTextContent(/^Morning trail\s*Trail running · Garmin Forerunner 965 · 12\.50 km/);

    fireEvent.press(screen.getByTestId('home-month-day-2026-09-17'));
    const plain = within(screen.getByTestId('home-month-detail')).getByTestId('calendar-activity-strava-9000');
    expect(within(plain).queryByTestId('calendar-activity-attribution')).toBeNull();
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
