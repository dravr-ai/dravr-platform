// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Home week over a mocked calendar read — past days' workouts, paging to stored history and the plan's end, the month sheet
// ABOUTME: Red if a day before history reads as empty, no plan hides the strip, paging runs past its edges, a workout cannot be opened, or a date's second session is lost or doubled

import React from 'react';
import { fireEvent, render, waitFor, within } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { CalendarResponse, GarminContribution, HomeActivity, PlanDay, TrainingPlanResponse } from '@pierre/shared-types';

import {
  ACTIVITIES,
  NO_PLAN_RESPONSE,
  PLAN,
  PLAN_RESPONSE,
  TODAY,
  calendarAnswer,
} from '../integration/app/helpers/homeFixtures';

const mockGetCalendar = jest.fn<Promise<CalendarResponse>, [string, string, string | undefined]>();

jest.mock('../src/services/api', () => ({
  athleteApi: {
    getCalendar: (from: string, to: string, locale?: string) => mockGetCalendar(from, to, locale),
  },
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
    expect(mockGetCalendar).toHaveBeenCalledWith('2026-09-21', '2026-09-27', 'en');
    expect(screen.getByTestId('home-section-week')).toHaveTextContent(/^This week/);
    expect(screen.getByTestId('home-week-day-2026-09-24').props.accessibilityState).toEqual({ selected: true });
    expect(screen.getByTestId('home-week-previous').props.accessibilityState).toEqual({ disabled: false });
  });

  // carnet#1495 step 3: the chip is its own press target inside a row that
  // drafts a chat, and a screen reader, which reads the row as one element,
  // reaches the same sheet from the row's actions.
  describe("a planned day's evidence chip", () => {
    const evidence = { label: 'Transposed from another sport', basis: 'Rønnestad 2015 [RCT]' };
    const withEvidence: TrainingPlanResponse = {
      ...PLAN_RESPONSE,
      plan: {
        ...PLAN,
        weeks: PLAN.weeks.map((week) => ({
          ...week,
          days: week.days.map((day) => (day.date === TODAY ? { ...day, evidence } : day)),
        })),
      },
    };

    it('opens its studies on a press without drafting a chat', async () => {
      const { screen, openDraft } = renderWeek(withEvidence);
      await weekOf(screen, '2026-09-21');

      const row = within(screen.getByTestId('home-week-detail')).getByTestId(`home-plan-day-${TODAY}-0`);
      expect(screen.queryByText('Rønnestad 2015 [RCT]')).toBeNull();
      fireEvent.press(within(row).getByTestId('plan-day-evidence'));
      expect(screen.getByText('The evidence behind this session')).toBeTruthy();
      expect(screen.getByText('Rønnestad 2015 [RCT]')).toBeTruthy();
      expect(openDraft).not.toHaveBeenCalled();
    });

    it("opens the same sheet from the row's accessibility action", async () => {
      const { screen, openDraft } = renderWeek(withEvidence);
      await weekOf(screen, '2026-09-21');

      const row = within(screen.getByTestId('home-week-detail')).getByTestId(`home-plan-day-${TODAY}-0`);
      expect(row.props.accessibilityActions).toEqual([
        { name: 'evidence', label: 'The evidence behind this session' },
      ]);
      fireEvent(row, 'accessibilityAction', { nativeEvent: { actionName: 'evidence' } });
      expect(screen.getByText('Rønnestad 2015 [RCT]')).toBeTruthy();
      expect(openDraft).not.toHaveBeenCalled();
    });

    it('offers no such action on a day without a chip', async () => {
      const { screen } = renderWeek();
      await weekOf(screen, '2026-09-21');

      const row = within(screen.getByTestId('home-week-detail')).getByTestId(`home-plan-day-${TODAY}-0`);
      expect(row.props.accessibilityActions).toBeUndefined();
      expect(within(row).queryByTestId('plan-day-evidence')).toBeNull();
    });
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
    expect(mockGetCalendar).toHaveBeenLastCalledWith('2026-09-14', '2026-09-20', 'en');
    expect(screen.getByTestId('home-section-week')).toHaveTextContent(/^Week of September 14/);

    const sunday = screen.getByTestId('home-week-day-2026-09-20');
    // The shared label says the minutes done with their unit (carnet#1840).
    expect(sunday.props.accessibilityLabel).toBe(
      `Sunday, September 20 — Done: ${ACTIVITIES[0].name} · ${Math.round(ACTIVITIES[0].duration_seconds / 60)} min`,
    );
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
    expect(mockGetCalendar).toHaveBeenLastCalledWith('2026-10-12', '2026-10-18', 'en');
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
    await waitFor(() => expect(mockGetCalendar).toHaveBeenLastCalledWith('2026-08-31', '2026-10-04', 'en'));
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
    await waitFor(() => expect(mockGetCalendar).toHaveBeenLastCalledWith('2026-09-28', '2026-11-01', 'en'));
  });
});

// carnet#1716: a date may hold several entries, in the order they are done.
describe('dates that hold several entries', () => {
  const CORE: PlanDay = {
    date: '2026-09-29',
    position: 1,
    sport: 'strength',
    workout: 'Core circuit',
    duration_min: 20,
    intensity: 'easy',
    rest: false,
  };
  const OPTIONAL: PlanDay = {
    date: '2026-09-22',
    position: 1,
    sport: 'swim',
    workout: "Optional: 20' easy swim",
    duration_min: 20,
    intensity: 'easy',
    rest: false,
  };
  /** Tuesday's rest with an optional session beside it, and next Tuesday's hills with a core circuit after them. */
  const response: TrainingPlanResponse = {
    ...PLAN_RESPONSE,
    plan: {
      ...PLAN,
      weeks: PLAN.weeks.map((week) => {
        const days = week.days.flatMap((day) => {
          if (day.date === OPTIONAL.date) return [day, OPTIONAL];
          if (day.date === CORE.date) return [day, CORE];
          return [day];
        });
        return { ...week, days };
      }),
    },
  };

  it("lists each of next week's sessions once, in order", async () => {
    const { screen, openDraft } = renderWeek(response);
    await weekOf(screen, '2026-09-21');

    fireEvent.press(screen.getByTestId('home-next-week'));
    const days = within(screen.getByTestId('home-next-week-days'));
    expect(days.getAllByText('Hill repeats')).toHaveLength(1);
    expect(days.getAllByText('Core circuit')).toHaveLength(1);
    expect(days.getAllByText('Rest')).toHaveLength(1);
    expect(
      days.getAllByTestId(/^home-plan-day-/).map((row) => row.props.testID),
    ).toEqual(['home-plan-day-2026-09-28-0', 'home-plan-day-2026-09-29-0', 'home-plan-day-2026-09-29-1']);

    fireEvent.press(days.getByTestId('home-plan-day-2026-09-29-1'));
    expect(openDraft).toHaveBeenCalledWith('Walk me through my session on Tuesday, September 29: Core circuit');
  });

  it('opens a day on each of its entries, the rest included, and marks it a session day', async () => {
    const { screen, openDraft } = renderWeek(response);
    await weekOf(screen, '2026-09-21');

    const tuesday = screen.getByTestId('home-week-day-2026-09-22');
    // A session beside the rest makes it a session day: filled, not hollow.
    expect(within(tuesday).getByTestId('home-week-mark-session')).toBeTruthy();
    expect(within(tuesday).queryByTestId('home-week-mark-rest')).toBeNull();
    expect(tuesday.props.accessibilityLabel).toBe(
      "Tuesday, September 22 — Rest; Optional: 20' easy swim · Swim · 20 min · easy",
    );

    fireEvent.press(tuesday);
    const detail = within(screen.getByTestId('home-week-detail'));
    expect(detail.getByTestId('home-plan-day-2026-09-22-0')).toHaveTextContent(/Rest/);
    expect(detail.getByTestId('home-plan-day-2026-09-22-1')).toHaveTextContent(/^Optional: 20' easy swim/);
    fireEvent.press(detail.getByTestId('home-plan-day-2026-09-22-0'));
    expect(openDraft).toHaveBeenLastCalledWith('Why is Tuesday, September 22 a rest day in my plan?');
    fireEvent.press(detail.getByTestId('home-plan-day-2026-09-22-1'));
    expect(openDraft).toHaveBeenLastCalledWith(
      "Walk me through my session on Tuesday, September 22: Optional: 20' easy swim",
    );
  });

  it('marks the same day a session day on the month', async () => {
    const { screen } = renderWeek(response);
    await weekOf(screen, '2026-09-21');

    fireEvent.press(screen.getByTestId('home-week-month'));
    await screen.findByTestId('home-month-title');
    await waitFor(() =>
      expect(within(screen.getByTestId('home-month-day-2026-09-22')).getByTestId('home-week-mark-session')).toBeTruthy(),
    );
    expect(within(screen.getByTestId('home-month-day-2026-09-22')).queryByTestId('home-week-mark-rest')).toBeNull();
  });
});
