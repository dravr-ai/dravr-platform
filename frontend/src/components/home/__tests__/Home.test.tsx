// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the Home page as a whole — the header, the plan half's three answers, and the activities under them
// ABOUTME: Red if no plan shows more than one filled button, or a plan that failed to load is offered as "no plan"

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { TrainingPlanResponse } from '@pierre/shared-types';
import { ThemeProvider } from '../../../hooks/useTheme';
import Home from '../Home';
import { homePlan, recentResponse, routeView, TODAY } from './homeFixtures';

const api = vi.hoisted(() => ({
  getTrainingPlan: vi.fn<(locale?: string) => Promise<TrainingPlanResponse>>(),
  getRecentActivities: vi.fn(),
  getActivityRoute: vi.fn(),
  getProvidersStatus: vi.fn(),
}));

vi.mock('../../../services/api', () => ({
  athleteApi: {
    getTrainingPlan: api.getTrainingPlan,
    getRecentActivities: api.getRecentActivities,
    getActivityRoute: api.getActivityRoute,
  },
  providersApi: { getProvidersStatus: api.getProvidersStatus },
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
}));

function renderHome() {
  const onNavigate = vi.fn();
  const onOpenChatDraft = vi.fn();
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const view = render(
    <QueryClientProvider client={queryClient}>
      <ThemeProvider>
        <Home onNavigate={onNavigate} onOpenChatDraft={onOpenChatDraft} />
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
  api.getActivityRoute.mockResolvedValue({ route: routeView('Long ride'), reason: null });
});

describe('Home', () => {
  it('opens with the Home header, the plan for today, the week, and the recent activities', async () => {
    api.getTrainingPlan.mockResolvedValue({ plan: homePlan(), today: TODAY });
    renderHome();

    expect(screen.getByRole('heading', { level: 2, name: 'Home' })).toBeInTheDocument();
    expect(await screen.findByTestId('home-today-session')).toHaveTextContent('Tempo run');
    expect(screen.getByTestId('home-week')).toBeInTheDocument();
    expect(await screen.findByTestId('home-activity-latest')).toBeInTheDocument();
    expect(screen.getAllByTestId('home-activity-row')).toHaveLength(4);
    expect(api.getTrainingPlan).toHaveBeenCalledWith('en');
    // Called with no limit: the server's default of five is the page's.
    expect(api.getRecentActivities).toHaveBeenCalledWith();
    // With a plan on screen the page has no filled button at all.
    expect(document.querySelectorAll('.btn-primary')).toHaveLength(0);
  });

  it('answers no plan with one filled "Build my plan" button that drafts the request', async () => {
    api.getTrainingPlan.mockResolvedValue({ plan: null, today: TODAY });
    const { onOpenChatDraft } = renderHome();

    const empty = await screen.findByTestId('home-plan-empty');
    expect(within(empty).getByText('No training plan yet')).toBeInTheDocument();
    expect(
      within(empty).getByText('Tell your agent about your goal race and it will lay out your season, week by week.'),
    ).toBeInTheDocument();
    expect(document.querySelectorAll('.btn-primary')).toHaveLength(1);
    expect(screen.queryByTestId('home-week')).toBeNull();

    await userEvent.click(within(empty).getByRole('button', { name: 'Build my plan' }));
    expect(onOpenChatDraft).toHaveBeenCalledExactlyOnceWith('Build me a training plan for my goal race.');
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
