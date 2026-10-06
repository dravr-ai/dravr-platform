// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests Home's folded Today line — today's session, the form as a share of fitness, the latest route, one tap to open
// ABOUTME: Folded away it leaves the reading order and the tab order, not only the screen

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { TodayPeek } from '../TodayPeek';

const api = vi.hoisted(() => ({
  getTrainingPlan: vi.fn(),
  getTrainingStatus: vi.fn(),
  getRecentActivities: vi.fn(),
}));

vi.mock('../../../services/api', () => ({ athleteApi: api }));

const TODAY = '2026-10-05';

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
    api.getTrainingStatus.mockResolvedValue({
      today: TODAY,
      form: { band: 'balanced', pct_of_fitness: -4 },
      trend: [],
      load_ratio: null,
      recovery_days: null,
    });
    api.getRecentActivities.mockResolvedValue({
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
        },
      ],
      as_of: '2026-10-05T09:41:00Z',
      sync_failure: null,
      stale: false,
    });
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

  it('leaves the reading order and the tab order when folded away', () => {
    renderPeek(true);
    // A test id reaches past aria-hidden; a role query, like a screen reader, does not.
    const wrapper = screen.getByTestId('home-today-peek').parentElement;
    expect(wrapper).toHaveAttribute('aria-hidden', 'true');
    expect(wrapper).toHaveAttribute('inert');
    expect(screen.queryByRole('button', { name: 'Open Today' })).toBeNull();
  });
});
