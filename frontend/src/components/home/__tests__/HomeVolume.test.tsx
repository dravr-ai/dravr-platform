// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the Home weekly volume — this week's figures, the sport filter, the bars per week, the empty and failed answers
// ABOUTME: Red if a week the server did not send is drawn, a filter leaks another sport's figures, or nothing stored reads as zeros

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { TrainingVolumeResponse } from '@pierre/shared-types';
import { HomeVolume } from '../HomeVolume';

const api = vi.hoisted(() => ({
  getTrainingVolume: vi.fn<() => Promise<TrainingVolumeResponse>>(),
}));

vi.mock('../../../services/api', () => ({
  athleteApi: { getTrainingVolume: api.getTrainingVolume },
  providersApi: {},
}));

const RUN = { sport_type: 'run', activities: 2, distance_meters: 15_000, duration_seconds: 5_400, elevation_gain_meters: 80 };
const RIDE = { sport_type: 'ride', activities: 1, distance_meters: 40_000, duration_seconds: 7_230, elevation_gain_meters: 450 };

/** Three weeks of history: runs, a quiet week, then a run week with a ride. */
function volume(overrides: Partial<TrainingVolumeResponse> = {}): TrainingVolumeResponse {
  return {
    today: '2026-10-07',
    weeks: [
      { week_start: '2026-09-21', sports: [{ ...RUN, activities: 3, distance_meters: 21_000 }] },
      { week_start: '2026-09-28', sports: [] },
      { week_start: '2026-10-05', sports: [RIDE, RUN] },
    ],
    ...overrides,
  };
}

function renderVolume() {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={queryClient}>
      <HomeVolume />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe('HomeVolume', () => {
  it("sums this week's distance, time and climbing over every sport", async () => {
    api.getTrainingVolume.mockResolvedValue(volume());
    renderVolume();

    const section = screen.getByTestId('home-volume');
    expect(within(section).getByRole('heading', { level: 3, name: 'Weekly volume' })).toBeInTheDocument();
    expect(await screen.findByTestId('home-volume-distance')).toHaveTextContent('55.00 km');
    // 12 630 s rounds to 3 h 31 min: a week's time is never read in seconds.
    expect(screen.getByTestId('home-volume-time')).toHaveTextContent('3h 31m');
    expect(screen.getByTestId('home-volume-elevation')).toHaveTextContent('530 m');
    expect(screen.getByTestId('home-volume-count')).toHaveTextContent('3 activities');
  });

  it('draws one bar per week the server sent, this week highlighted and read out', async () => {
    api.getTrainingVolume.mockResolvedValue(volume());
    renderVolume();

    const chart = await screen.findByTestId('home-volume-trend');
    const bars = within(chart).getAllByTestId('home-volume-bar');
    expect(bars).toHaveLength(3);
    // The quiet week is a bar of no height, never left out or padded.
    expect(bars[1]).toHaveAttribute('height', '0');
    expect(bars[2]).toHaveClass('fill-primary');
    expect(screen.getByTestId('home-volume-trend-label')).toHaveTextContent('Distance per week, last 3 weeks');
    expect(screen.getByTestId('home-volume-readout')).toHaveTextContent('Week of Oct 5: 55.00 km');
    expect(chart).toHaveAccessibleName(
      'Distance per week, last 3 weeks: from 21.00 km in the week of Sep 21 to 55.00 km this week',
    );
  });

  it('filters every figure and bar to one sport', async () => {
    api.getTrainingVolume.mockResolvedValue(volume());
    renderVolume();

    // Sports are offered most time first: run 4 h 30, ride 2 h.
    const tabs = await screen.findAllByRole('tab');
    expect(tabs.map((tab) => tab.textContent)).toEqual(['All sports', 'Run', 'Ride']);

    await userEvent.click(screen.getByRole('tab', { name: 'Ride' }));
    expect(screen.getByTestId('home-volume-distance')).toHaveTextContent('40.00 km');
    expect(screen.getByTestId('home-volume-elevation')).toHaveTextContent('450 m');
    expect(screen.getByTestId('home-volume-count')).toHaveTextContent('1 activity');
    const bars = within(screen.getByTestId('home-volume-trend')).getAllByTestId('home-volume-bar');
    expect(bars[0]).toHaveAttribute('height', '0');
  });

  it('draws time when the selection never recorded a distance', async () => {
    api.getTrainingVolume.mockResolvedValue(
      volume({
        weeks: [
          { week_start: '2026-09-28', sports: [{ sport_type: 'strength_training', activities: 1, distance_meters: 0, duration_seconds: 2_700, elevation_gain_meters: 0 }] },
          { week_start: '2026-10-05', sports: [{ sport_type: 'strength_training', activities: 2, distance_meters: 0, duration_seconds: 5_400, elevation_gain_meters: 0 }] },
        ],
      }),
    );
    renderVolume();

    expect(await screen.findByTestId('home-volume-trend-label')).toHaveTextContent('Time per week, last 2 weeks');
    expect(screen.getByTestId('home-volume-readout')).toHaveTextContent('Week of Oct 5: 1h 30m');
    // One sport: nothing to choose between.
    expect(screen.queryByRole('group', { name: 'Sport' })).not.toBeInTheDocument();
  });

  it('says in words that one week is not yet a trend', async () => {
    api.getTrainingVolume.mockResolvedValue(volume({ weeks: [{ week_start: '2026-10-05', sports: [RUN] }] }));
    renderVolume();

    expect(await screen.findByTestId('home-volume-trend-short')).toHaveTextContent(
      'Your weekly trend shows up here as the weeks add up.',
    );
    expect(screen.queryByTestId('home-volume-trend')).not.toBeInTheDocument();
  });

  it('says nothing is stored yet instead of drawing empty weeks', async () => {
    api.getTrainingVolume.mockResolvedValue(volume({ weeks: [] }));
    renderVolume();

    expect(await screen.findByTestId('home-volume-empty')).toHaveTextContent('No activities stored yet.');
    expect(screen.queryByTestId('home-volume-week')).not.toBeInTheDocument();
  });

  it('offers a retry when the volume could not be read', async () => {
    api.getTrainingVolume.mockRejectedValueOnce(new Error('boom')).mockResolvedValueOnce(volume());
    renderVolume();

    const failed = await screen.findByTestId('home-volume-failed');
    expect(failed).toHaveTextContent("Your weekly volume couldn't be loaded.");
    await userEvent.click(within(failed).getByRole('button', { name: 'Retry' }));
    expect(await screen.findByTestId('home-volume-distance')).toHaveTextContent('55.00 km');
  });
});
