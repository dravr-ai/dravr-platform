// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests Home's folded Today line — today's session, the form as a share of fitness, the latest route, one tap to open
// ABOUTME: It ends with Getting started's count while that section is open; folded away it leaves the reading and tab order

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { TodayPeek } from '../TodayPeek';

const api = vi.hoisted(() => ({
  getTrainingPlan: vi.fn(),
  getTrainingStatus: vi.fn(),
  getRecentActivities: vi.fn(),
  getGettingStarted: vi.fn(),
  getHomePreferences: vi.fn(),
  updateHomePreferences: vi.fn(),
}));

vi.mock('../../../services/api', () => ({
  athleteApi: api,
  gettingStartedApi: { get: api.getGettingStarted },
}));

const TODAY = '2026-10-05';

/** Getting started with two of five done, as the server answers it. */
function progress(overrides: Record<string, unknown> = {}) {
  return { rows: [], done_count: 2, total: 5, folded_at: null, hidden: false, ...overrides };
}

/** A balanced form with no Garmin data behind it, as the status route answers it. */
function status(overrides: Record<string, unknown> = {}) {
  return {
    today: TODAY,
    form: { band: 'balanced', pct_of_fitness: -4 },
    trend: [],
    load_ratio: null,
    recovery_days: null,
    garmin_contribution: null,
    ...overrides,
  };
}

/** The recent activities with one trail run on a route, as the server answers them. */
function recent(latest: Record<string, unknown> = {}) {
  return {
    activities: [
      {
        id: 'act-1',
        provider: 'strava',
        name: 'Bromont Trail Running',
        sport_type: 'trail_run',
        start_date: '2026-10-03T12:00:00Z',
        duration_seconds: 1925,
        distance_meters: 6100,
        elevation_gain_meters: 57,
        has_gps: true,
        summary_polyline: '_p~iF~ps|U_ulLnnqC_mqNvxq`@',
        attribution: null,
        ...latest,
      },
    ],
    as_of: '2026-10-05T09:41:00Z',
    sync_failure: null,
    stale: false,
  };
}

function renderPeek(hidden = false, onOpen = vi.fn()) {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={queryClient}>
      <TodayPeek onOpen={onOpen} hidden={hidden} />
    </QueryClientProvider>,
  );
  return onOpen;
}

describe('TodayPeek', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    api.getHomePreferences.mockResolvedValue({ plan_suggestion_hidden: false, getting_started_hidden: false });
    api.getGettingStarted.mockResolvedValue(progress());
    api.getTrainingPlan.mockResolvedValue({
      today: TODAY,
      plan: {
        phases: [],
        current_phase_index: 0,
        weeks: [
          {
            week_start: TODAY,
            focus: 'aerobic volume',
            phase_index: 0,
            current: true,
            days: [{ date: TODAY, sport: 'run', workout: 'Easy trail', duration_min: 45, intensity: 'Z2', rest: false }],
          },
        ],
        weeks_deferred: 0,
      },
    });
    api.getTrainingStatus.mockResolvedValue(status());
    api.getRecentActivities.mockResolvedValue(recent());
  });

  it('says today’s session and the form as a share of fitness, with the latest route, and opens on a tap', async () => {
    const user = userEvent.setup();
    const onOpen = renderPeek();

    const line = await screen.findByRole('button', { name: 'Open Today' });
    expect(await screen.findByText('Easy trail · 45 min')).toBeInTheDocument();
    expect(await screen.findByText(/^Balanced · /)).toHaveTextContent(/fitness/);
    expect(await screen.findByTestId('route-sketch')).toBeInTheDocument();

    await user.click(line);
    expect(onOpen).toHaveBeenCalledTimes(1);
  });

  // carnet#1154: the peek's own figures carry their attribution beneath them.
  it.each([
    ['sole', 'Created using data provided by Garmin devices.'],
    ['part', 'Created in part using data provided by Garmin devices.'],
  ] as const)("carries Garmin's line for a %s contribution beneath the form", async (contribution, line) => {
    api.getTrainingStatus.mockResolvedValue(status({ garmin_contribution: contribution }));
    renderPeek();

    const attribution = await screen.findByTestId('home-today-peek-attribution');
    expect(attribution.textContent).toBe(line);
    const form = await screen.findByText(/^Balanced · /);
    expect(form.nextElementSibling).toBe(attribution);
  });

  it("names the sketched route's device when no Garmin data went into the form", async () => {
    api.getRecentActivities.mockResolvedValue(recent({ attribution: 'Garmin Forerunner 965' }));
    renderPeek();

    expect(await screen.findByTestId('route-sketch')).toBeInTheDocument();
    expect((await screen.findByTestId('home-today-peek-attribution')).textContent).toBe('Garmin Forerunner 965');
  });

  it("puts the form's Garmin line ahead of the sketched route's device: one attribution line, never two", async () => {
    api.getTrainingStatus.mockResolvedValue(status({ garmin_contribution: 'part' }));
    api.getRecentActivities.mockResolvedValue(recent({ attribution: 'Garmin Forerunner 965' }));
    renderPeek();

    expect(await screen.findByTestId('route-sketch')).toBeInTheDocument();
    await screen.findByText(/^Balanced · /);
    await waitFor(() =>
      expect(screen.getByTestId('home-today-peek-attribution').textContent).toBe(
        'Created in part using data provided by Garmin devices.',
      ),
    );
    expect(screen.getAllByTestId('home-today-peek-attribution')).toHaveLength(1);
  });

  it('carries no attribution when neither the form nor the sketched route owes one', async () => {
    renderPeek();

    expect(await screen.findByTestId('route-sketch')).toBeInTheDocument();
    await screen.findByText(/^Balanced · /);
    expect(screen.queryByTestId('home-today-peek-attribution')).toBeNull();
  });

  it('leaves a device unnamed when the peek draws no route for it', async () => {
    api.getRecentActivities.mockResolvedValue(recent({ summary_polyline: null, attribution: 'Garmin Forerunner 965' }));
    renderPeek();

    await screen.findByText(/^Balanced · /);
    await waitFor(() => expect(api.getRecentActivities).toHaveBeenCalled());
    expect(screen.queryByTestId('route-sketch')).toBeNull();
    expect(screen.queryByTestId('home-today-peek-attribution')).toBeNull();
  });

  it('leaves the reading order and the tab order when folded away', () => {
    renderPeek(true);
    // A test id reaches past aria-hidden; a role query, like a screen reader, does not.
    const wrapper = screen.getByTestId('home-today-peek').parentElement;
    expect(wrapper).toHaveAttribute('aria-hidden', 'true');
    expect(wrapper).toHaveAttribute('inert');
    expect(screen.queryByRole('button', { name: 'Open Today' })).toBeNull();
  });

  // carnet#1098: below lg the line ends with Getting started's count, read
  // from the query the section in the sheet reads.
  it('ends with the Getting started count while that section is open', async () => {
    renderPeek();
    expect(await screen.findByTestId('home-today-peek-progress')).toHaveTextContent('2 of 5');
  });

  it('drops the count once Getting started is hidden, folded or gone', async () => {
    api.getHomePreferences.mockResolvedValue({ plan_suggestion_hidden: false, getting_started_hidden: true });
    renderPeek();
    await screen.findByText('Easy trail · 45 min');
    await waitFor(() => expect(api.getGettingStarted).toHaveBeenCalled());
    expect(screen.queryByTestId('home-today-peek-progress')).toBeNull();
  });

  it('drops the count once every flow is done and the section folded', async () => {
    api.getGettingStarted.mockResolvedValue(progress({ done_count: 5, folded_at: new Date().toISOString() }));
    renderPeek();
    await screen.findByText('Easy trail · 45 min');
    await waitFor(() => expect(api.getGettingStarted).toHaveBeenCalled());
    expect(screen.queryByTestId('home-today-peek-progress')).toBeNull();
  });
});
