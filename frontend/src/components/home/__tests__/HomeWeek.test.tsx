// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the Home week — the strip with today marked, past days' workouts, paging to stored history and the plan's end, the month sheet
// ABOUTME: Red if a gap in the plan reads as rest, a day before history reads as empty, no plan hides the strip, or a workout cannot be opened

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { CalendarResponse, TrainingPlanResponse, WorkoutPlan } from '@pierre/shared-types';
import { HomeWeek } from '../HomeWeek';
import { activityViewRoute } from '../../activity/activityRoute';
import { DRAFT_DATE, formatCivilDate } from '../homeFormat';
import { homePlan, TODAY } from './homeFixtures';
import { calendarAnswer, HISTORY_START } from './calendarFixtures';

const api = vi.hoisted(() => ({
  getTrainingPlan: vi.fn<(locale?: string) => Promise<TrainingPlanResponse>>(),
  getCalendar: vi.fn<(from: string, to: string) => Promise<CalendarResponse>>(),
}));

vi.mock('../../../services/api', () => ({
  athleteApi: { getTrainingPlan: api.getTrainingPlan, getCalendar: api.getCalendar },
}));

function renderWeek(plan: WorkoutPlan | null = homePlan()) {
  const onOpenChatDraft = vi.fn();
  const onNavigate = vi.fn();
  api.getTrainingPlan.mockResolvedValue({ plan, today: TODAY });
  api.getCalendar.mockImplementation(async (from, to) => calendarAnswer(from, to, { plan }));
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={queryClient}>
      <HomeWeek onOpenChatDraft={onOpenChatDraft} onNavigate={onNavigate} />
    </QueryClientProvider>,
  );
  return { onOpenChatDraft, onNavigate };
}

/** The strip once the week's calendar answer is on it. */
async function loadedStrip(): Promise<HTMLElement> {
  const strip = await screen.findByRole('list');
  await waitFor(() => expect(strip).toHaveAttribute('aria-busy', 'false'));
  return strip;
}

const WEEK = ['2026-09-21', '2026-09-22', '2026-09-23', '2026-09-24', '2026-09-25', '2026-09-26', '2026-09-27'];

beforeEach(() => {
  vi.clearAllMocks();
});

describe('HomeWeek — this week', () => {
  it('lays out Monday to Sunday of the current week with today marked', async () => {
    renderWeek();

    const strip = await loadedStrip();
    const cells = within(strip).getAllByRole('button');
    expect(cells.map((cell) => cell.getAttribute('data-testid'))).toEqual(WEEK.map((date) => `home-week-day-${date}`));
    const current = cells.filter((cell) => cell.getAttribute('aria-current') === 'date');
    expect(current).toHaveLength(1);
    expect(current[0]).toHaveAttribute('data-testid', `home-week-day-${TODAY}`);
    expect(screen.getByRole('heading', { name: 'This week' })).toBeInTheDocument();
    expect(screen.getByText('threshold volume')).toBeInTheDocument();
    expect(api.getCalendar).toHaveBeenCalledWith('2026-09-21', '2026-09-27');
  });

  it('keeps a rest day and a day the plan never reached apart', async () => {
    renderWeek();
    await loadedStrip();

    expect(screen.getByTestId('home-week-day-2026-09-25')).toHaveAccessibleName(/— Rest$/);
    expect(screen.getByTestId('home-week-day-2026-09-26')).toHaveAccessibleName(/— Your plan doesn't cover this day\.$/);
    expect(screen.getByTestId('home-week-day-2026-09-24')).toHaveTextContent('50 min');
  });

  it("shows a past day's workouts in the cell and opens each one's own view", async () => {
    const { onNavigate } = renderWeek();
    await loadedStrip();

    // 35 + 30 minutes done on Tuesday, where the plan had 35 min of strides.
    const tuesday = screen.getByTestId('home-week-day-2026-09-22');
    expect(tuesday).toHaveTextContent('65 min');
    expect(tuesday).toHaveAccessibleName(/— Done: Strides session, Evening spin — Strides/);
    await userEvent.click(tuesday);

    const detail = screen.getByTestId('home-week-detail');
    expect(within(detail).getByText('Done')).toBeInTheDocument();
    expect(within(detail).getByText('Planned')).toBeInTheDocument();
    await userEvent.click(within(detail).getByTestId('calendar-activity-garmin-cal-3'));
    expect(onNavigate).toHaveBeenCalledExactlyOnceWith(activityViewRoute('garmin', 'cal-3'));
  });

  it("opens today to the plan card's own row — steps and fuel — and drafts the question from it", async () => {
    const { onOpenChatDraft } = renderWeek();
    await loadedStrip();

    const cell = screen.getByTestId(`home-week-day-${TODAY}`);
    expect(cell).toHaveAttribute('aria-expanded', 'false');
    await userEvent.click(cell);
    expect(cell).toHaveAttribute('aria-expanded', 'true');

    const detail = screen.getByTestId('home-week-detail');
    expect(cell).toHaveAttribute('aria-controls', 'home-week-detail');
    expect(within(detail).getByText('No activity this day.')).toBeInTheDocument();
    expect(within(detail).getByText('Tempo run')).toBeInTheDocument();
    expect(within(detail).getByText('Steps')).toBeInTheDocument();
    expect(within(detail).getByText('Warm-up · 15m · Z1')).toBeInTheDocument();
    expect(within(detail).getByText('Fuel')).toBeInTheDocument();

    const named = formatCivilDate(TODAY, 'en', DRAFT_DATE);
    const draft = `Walk me through my session on ${named}: Tempo run`;
    await userEvent.click(within(detail).getByRole('button', { name: draft }));
    expect(onOpenChatDraft).toHaveBeenCalledExactlyOnceWith(draft);
  });

  it('closes the open day on a second tap', async () => {
    renderWeek();
    await loadedStrip();

    const cell = screen.getByTestId('home-week-day-2026-09-27');
    await userEvent.click(cell);
    expect(within(screen.getByTestId('home-week-detail')).getByText('Long ride')).toBeInTheDocument();
    await userEvent.click(cell);
    expect(screen.queryByTestId('home-week-detail')).toBeNull();
  });

  it('opens an uncovered future day to the sentence saying so, with nothing to send', async () => {
    const { onOpenChatDraft } = renderWeek();
    await loadedStrip();

    await userEvent.click(screen.getByTestId('home-week-day-2026-09-26'));

    const detail = screen.getByTestId('home-week-detail');
    expect(detail).toHaveTextContent("Your plan doesn't cover this day.");
    expect(within(detail).queryByRole('button')).toBeNull();
    expect(onOpenChatDraft).not.toHaveBeenCalled();
  });

  it("names next week with the plan card's heading and its focus", async () => {
    renderWeek();
    await loadedStrip();

    const next = screen.getByTestId('home-next-week');
    expect(within(next).getByText('Next week')).toBeInTheDocument();
    expect(within(next).getByText('2026-09-28')).toBeInTheDocument();
    expect(within(next).getByText('absorb the block')).toBeInTheDocument();
  });

  it('says a week that failed to load failed, and reads it again on retry', async () => {
    renderWeek();
    api.getCalendar.mockRejectedValueOnce(new Error('503'));

    const failed = await screen.findByTestId('home-week-failed');
    expect(failed).toHaveTextContent("These days couldn't be loaded.");
    await userEvent.click(within(failed).getByRole('button', { name: 'Retry' }));
    await loadedStrip();
    expect(screen.getByTestId('home-week-day-2026-09-22')).toHaveTextContent('65 min');
  });
});

describe('HomeWeek — paging', () => {
  it('pages back to the week holding the first stored day, says where history starts, and stops', async () => {
    renderWeek();
    await loadedStrip();

    await userEvent.click(screen.getByTestId('home-week-previous'));
    await loadedStrip();
    expect(api.getCalendar).toHaveBeenLastCalledWith('2026-09-14', '2026-09-20');
    const named = formatCivilDate('2026-09-14', 'en', { day: 'numeric', month: 'long' });
    expect(screen.getByRole('heading', { name: `Week of ${named}` })).toBeInTheDocument();
    expect(screen.getByTestId('home-week-day-2026-09-19')).toHaveTextContent('90 min');
    expect(screen.queryByTestId('home-next-week')).toBeNull();

    await userEvent.click(screen.getByTestId('home-week-previous'));
    await loadedStrip();
    expect(api.getCalendar).toHaveBeenLastCalledWith('2026-09-07', '2026-09-13');
    // The 8th is older than the cache: unknown, never an empty day.
    expect(screen.getByTestId('home-week-day-2026-09-08')).toHaveAccessibleName(
      /— This day is older than your stored activities\.$/,
    );
    expect(screen.getByTestId('home-week-history-start')).toHaveTextContent(
      `Your stored activities start on ${formatCivilDate(HISTORY_START, 'en', DRAFT_DATE)}; earlier days aren't kept.`,
    );
    expect(screen.getByTestId('home-week-previous')).toBeDisabled();

    await userEvent.click(screen.getByTestId('home-week-today'));
    await loadedStrip();
    expect(screen.getByRole('heading', { name: 'This week' })).toBeInTheDocument();
  });

  it("pages forward to the week of the plan's season end, and no further", async () => {
    renderWeek();
    await loadedStrip();

    // The fixture's season ends Monday 23 November, past its deferred weeks.
    let steps = 0;
    while (!screen.getByTestId('home-week-next').hasAttribute('disabled') && steps < 20) {
      await userEvent.click(screen.getByTestId('home-week-next'));
      await loadedStrip();
      steps += 1;
    }
    expect(steps).toBe(9);
    expect(api.getCalendar).toHaveBeenLastCalledWith('2026-11-23', '2026-11-29');
  });

  it('renders without a plan: what was done, one quiet line, and the week ahead', async () => {
    renderWeek(null);
    await loadedStrip();

    expect(screen.getByTestId('home-week-day-2026-09-22')).toHaveTextContent('65 min');
    expect(screen.getByTestId('home-week-no-plan')).toHaveTextContent(
      "No plan yet, so only what you've done shows here.",
    );
    expect(screen.queryByTestId('home-next-week')).toBeNull();

    await userEvent.click(screen.getByTestId('home-week-next'));
    await loadedStrip();
    expect(api.getCalendar).toHaveBeenLastCalledWith('2026-09-28', '2026-10-04');
    expect(screen.getByTestId('home-week-next')).toBeDisabled();
  });
});

describe('HomeWeek — month sheet', () => {
  it("expands to the month: workouts done and sessions planned on one grid, a day opening to both", async () => {
    const { onNavigate } = renderWeek();
    await loadedStrip();

    await userEvent.click(screen.getByTestId('home-week-month'));
    const sheet = await screen.findByRole('dialog', { name: 'September 2026' });
    const grid = within(sheet).getByTestId('home-month-grid');
    await waitFor(() => expect(grid).toHaveAttribute('aria-busy', 'false'));
    expect(api.getCalendar).toHaveBeenLastCalledWith('2026-08-31', '2026-10-04');
    expect(within(sheet).getByTestId('home-month-done-2026-09-22')).toBeInTheDocument();
    expect(within(sheet).getByTestId('home-month-planned-2026-09-24')).toBeInTheDocument();
    expect(within(sheet).getByTestId('home-month-history-start')).toBeInTheDocument();
    expect(within(sheet).getByTestId('home-month-previous')).toBeDisabled();

    await userEvent.click(within(sheet).getByTestId('home-month-day-2026-09-22'));
    await userEvent.click(within(sheet).getByTestId('calendar-activity-strava-cal-2'));
    expect(onNavigate).toHaveBeenCalledExactlyOnceWith(activityViewRoute('strava', 'cal-2'));
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('pages to the next month', async () => {
    renderWeek();
    await loadedStrip();

    await userEvent.click(screen.getByTestId('home-week-month'));
    const sheet = await screen.findByRole('dialog', { name: 'September 2026' });
    await userEvent.click(within(sheet).getByTestId('home-month-next'));
    expect(await screen.findByRole('dialog', { name: 'October 2026' })).toBeInTheDocument();
    expect(api.getCalendar).toHaveBeenLastCalledWith('2026-09-28', '2026-11-01');
  });
});
