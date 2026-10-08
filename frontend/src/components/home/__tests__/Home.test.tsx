// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests Home's Today as a whole — the plan half's three answers, the status, and the activities under them
// ABOUTME: Red if no plan shows more than one filled button, or a plan that failed to load is offered as "no plan"

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { TrainingPlanResponse } from '@pierre/shared-types';
import { ThemeProvider } from '../../../hooks/useTheme';
import { HomeBriefing } from '../Home';
import { homePlan, recentResponse, routeView, TODAY } from './homeFixtures';
import { calendarAnswer } from './calendarFixtures';

const api = vi.hoisted(() => ({
  getTrainingPlan: vi.fn<(locale?: string) => Promise<TrainingPlanResponse>>(),
  getRecentActivities: vi.fn(),
  getActivityRoute: vi.fn(),
  getProvidersStatus: vi.fn(),
  getTrainingStatus: vi.fn(),
  getTrainingVolume: vi.fn(),
  getHomePreferences: vi.fn<() => Promise<{ plan_suggestion_hidden: boolean }>>(),
  updateHomePreferences: vi.fn<(prefs: { plan_suggestion_hidden: boolean }) => Promise<{ plan_suggestion_hidden: boolean }>>(),
  getCalendar: vi.fn(),
}));

vi.mock('../../../services/api', () => ({
  athleteApi: {
    getTrainingPlan: api.getTrainingPlan,
    getRecentActivities: api.getRecentActivities,
    getActivityRoute: api.getActivityRoute,
    getTrainingStatus: api.getTrainingStatus,
    getTrainingVolume: api.getTrainingVolume,
    getHomePreferences: api.getHomePreferences,
    updateHomePreferences: api.updateHomePreferences,
    getCalendar: api.getCalendar,
  },
  providersApi: { getProvidersStatus: api.getProvidersStatus },
  // The route-marker beta is off: these suites pin the map without its markers.
  featureFlagsApi: { getMyFeatures: async () => ({ flags: {}, known: [] }) },
}));

vi.mock('maplibre-gl', () => ({
  Map: class {
    addControl() {}
    on() {}
    setStyle() {}
    remove() {}
  },
  AttributionControl: class {},
  NavigationControl: class {},
  Marker: class {},
  setWorkerUrl: () => {},
}));

function renderHome() {
  const onNavigate = vi.fn();
  const onOpenChatDraft = vi.fn();
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const view = render(
    <QueryClientProvider client={queryClient}>
      <ThemeProvider>
        <HomeBriefing onNavigate={onNavigate} onOpenChatDraft={onOpenChatDraft} />
      </ThemeProvider>
    </QueryClientProvider>,
  );
  return { ...view, onNavigate, onOpenChatDraft };
}

beforeEach(() => {
  vi.clearAllMocks();
  window.localStorage.clear();
  api.getProvidersStatus.mockResolvedValue({ providers: [{ provider: 'strava', connected: true }] });
  api.getRecentActivities.mockResolvedValue(recentResponse());
  api.getActivityRoute.mockResolvedValue({ route: routeView(), reason: null });
  api.getCalendar.mockImplementation(async (from: string, to: string) => calendarAnswer(from, to));
  api.getTrainingStatus.mockResolvedValue({
    today: TODAY,
    form: { band: 'productive', pct_of_fitness: -14 },
    trend: [
      { date: '2026-09-22', band: 'balanced', pct_of_fitness: -6 },
      { date: TODAY, band: 'productive', pct_of_fitness: -14 },
    ],
      load_ratio: { ratio: 1.1, acute_days: 7, chronic_days: 28 },
    recovery_days: 0,
  });
  api.getTrainingVolume.mockResolvedValue({
    today: TODAY,
    weeks: [
      {
        week_start: '2026-09-21',
        sports: [{ sport_type: 'run', activities: 1, distance_meters: 8_000, duration_seconds: 2_400, elevation_gain_meters: 0 }],
      },
    ],
  });
});

describe('Home — Today', () => {
  it('holds the plan for today, the week, the status and the recent activities, in that order', async () => {
    api.getTrainingPlan.mockResolvedValue({ plan: homePlan(), today: TODAY });
    renderHome();

    // The page header belongs to the conversation; Today is the panel beside it.
    expect(screen.queryByRole('heading', { level: 2, name: 'Home' })).toBeNull();
    expect(await screen.findByTestId('home-today-session')).toHaveTextContent('Tempo run');
    expect(screen.getByTestId('home-week')).toBeInTheDocument();
    expect(await screen.findByTestId('home-status-band')).toHaveTextContent('Productive');
    expect(await screen.findByTestId('home-volume-distance')).toHaveTextContent('8.00 km');
    expect(await screen.findByTestId('home-activity-latest')).toBeInTheDocument();
    // The weekly volume sits between the training status and the activities.
    const volume = screen.getByTestId('home-volume');
    expect(screen.getByTestId('home-status').compareDocumentPosition(volume)).toBe(Node.DOCUMENT_POSITION_FOLLOWING);
    expect(volume.compareDocumentPosition(screen.getByTestId('home-activity-latest'))).toBe(
      Node.DOCUMENT_POSITION_FOLLOWING,
    );
    expect(screen.getAllByTestId('home-activity-row')).toHaveLength(4);
    expect(api.getTrainingPlan).toHaveBeenCalledWith('en');
    // Called with no limit: the server's default of five is the page's.
    expect(api.getRecentActivities).toHaveBeenCalledWith();
    // With a plan on screen the page has no filled button at all.
    expect(document.querySelectorAll('.btn-primary')).toHaveLength(0);
  });

  // carnet#820: not every athlete wants a plan, so the offer is a quiet link
  // they can set aside, never the page's one filled button.
  it('answers no plan with a quiet "Build my plan" link that drafts the request', async () => {
    api.getTrainingPlan.mockResolvedValue({ plan: null, today: TODAY });
    api.getHomePreferences.mockResolvedValue({ plan_suggestion_hidden: false });
    const { onOpenChatDraft } = renderHome();

    const empty = await screen.findByTestId('home-plan-empty');
    expect(within(empty).getByText('No training plan yet')).toBeInTheDocument();
    expect(
      await within(empty).findByText('Tell your agent about your goal race and it will lay out your season, week by week.'),
    ).toBeInTheDocument();
    expect(document.querySelectorAll('.btn-primary')).toHaveLength(0);
    // The week still shows what was done, without a plan to read (carnet#708).
    expect(screen.getByTestId('home-week')).toBeInTheDocument();

    await userEvent.click(within(empty).getByRole('button', { name: 'Build my plan' }));
    expect(onOpenChatDraft).toHaveBeenCalledExactlyOnceWith('Build me a training plan for my goal race.');
  });

  it('sets the plan suggestion aside on the server, keeping the one plain sentence', async () => {
    api.getTrainingPlan.mockResolvedValue({ plan: null, today: TODAY });
    api.getHomePreferences.mockResolvedValue({ plan_suggestion_hidden: false });
    api.updateHomePreferences.mockResolvedValue({ plan_suggestion_hidden: true });
    renderHome();

    const empty = await screen.findByTestId('home-plan-empty');
    await userEvent.click(await within(empty).findByRole('button', { name: 'Hide the plan suggestion' }));

    expect(api.updateHomePreferences).toHaveBeenCalledWith({ plan_suggestion_hidden: true });
    await waitFor(() => expect(within(empty).queryByRole('button', { name: 'Build my plan' })).toBeNull());
    expect(within(empty).getByText('No training plan yet')).toBeInTheDocument();
  });

  it('offers no plan to an athlete who set the suggestion aside, on load', async () => {
    api.getTrainingPlan.mockResolvedValue({ plan: null, today: TODAY });
    api.getHomePreferences.mockResolvedValue({ plan_suggestion_hidden: true });
    renderHome();

    const empty = await screen.findByTestId('home-plan-empty');
    await waitFor(() => expect(api.getHomePreferences).toHaveBeenCalled());
    expect(within(empty).getByText('No training plan yet')).toBeInTheDocument();
    expect(within(empty).queryByRole('button', { name: 'Build my plan' })).toBeNull();
    expect(within(empty).queryByRole('button', { name: 'Hide the plan suggestion' })).toBeNull();
  });

  it('says a plan that failed to load failed — never that there is none', async () => {
    api.getTrainingPlan.mockRejectedValueOnce(new Error('503'));
    renderHome();

    const failed = await screen.findByTestId('home-plan-failed');
    expect(failed).toHaveTextContent("Your plan couldn't be loaded.");
    expect(screen.queryByRole('button', { name: 'Build my plan' })).toBeNull();

    api.getTrainingPlan.mockResolvedValueOnce({ plan: homePlan(), today: TODAY });
    await userEvent.click(within(failed).getByRole('button', { name: 'Retry' }));
    expect(await screen.findByTestId('home-today-session')).toBeInTheDocument();
  });

  it('treats a today that is not a calendar day as a load failure rather than guessing a week', async () => {
    api.getTrainingPlan.mockResolvedValue({ plan: homePlan(), today: '2026-02-30' });
    renderHome();

    expect(await screen.findByTestId('home-plan-failed')).toHaveTextContent("Your plan couldn't be loaded.");
    expect(screen.queryByTestId('home-week')).toBeNull();
  });
});
