// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the Home training status — the band and figure the server sent, the trend, the load and recovery lines
// ABOUTME: Red if a thin history is drawn as a reading, a failed read is shown as thin history, or a band is derived client-side

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { fireEvent, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { TrainingStatusResponse } from '@pierre/shared-types';
import { HomeStatus } from '../HomeStatus';

const api = vi.hoisted(() => ({
  getTrainingStatus: vi.fn<() => Promise<TrainingStatusResponse>>(),
}));

vi.mock('../../../services/api', () => ({
  athleteApi: { getTrainingStatus: api.getTrainingStatus },
  providersApi: {},
}));

/** A deep history: three days of trend, a load ratio, no lighter day called for. */
function status(overrides: Partial<TrainingStatusResponse> = {}): TrainingStatusResponse {
  return {
    today: '2026-09-20',
    form: { band: 'heavy_block', pct_of_fitness: -22 },
    trend: [
      { date: '2026-09-18', band: 'productive', pct_of_fitness: -12 },
      { date: '2026-09-19', band: 'fresh', pct_of_fitness: 6 },
      { date: '2026-09-20', band: 'heavy_block', pct_of_fitness: -22 },
    ],
    load_ratio: { ratio: 1.37, acute_days: 7, chronic_days: 28 },
    recovery_days: 0,
    ...overrides,
  };
}

function renderStatus() {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={queryClient}>
      <HomeStatus />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe('HomeStatus', () => {
  it('names the band the server sent, form as a share of fitness, the load ratio and the recovery days', async () => {
    api.getTrainingStatus.mockResolvedValue(status());
    renderStatus();

    const section = screen.getByTestId('home-status');
    expect(within(section).getByRole('heading', { level: 3, name: 'Training status' })).toBeInTheDocument();
    expect(await screen.findByTestId('home-status-band')).toHaveTextContent('Heavy block');
    expect(screen.getByTestId('home-status-form')).toHaveTextContent('Form -22% of your fitness');
    expect(section).toHaveTextContent('The deep end of the productive zone.');
    expect(screen.getByTestId('home-status-load')).toHaveTextContent('Last 7 days: 1.4× your 28-day average');
    expect(screen.getByTestId('home-status-recovery')).toHaveTextContent('Your form calls for no extra lighter day.');
  });

  it('draws the trend as one line and says in words what it shows', async () => {
    api.getTrainingStatus.mockResolvedValue(status());
    renderStatus();

    const chart = await screen.findByRole('img', {
      name: 'Form as a share of your fitness from Sep 18 to Sep 20: from -12% to -22%',
    });
    // Three days across the 320 box inside its 6 padding; +6 at the top, -22 at the bottom.
    expect(chart.querySelector('path')).toHaveAttribute('d', 'M6.00 39.43L160.00 6.00L314.00 58.00');
    // Three points are two days end to end, whatever window the server aims for.
    expect(screen.getByTestId('home-status')).toHaveTextContent('Your form over the last 2 days');
    expect(screen.getByTestId('home-status-trend-readout')).toHaveTextContent(
      'Above the line you are fresher than your fitness; below it you are carrying fatigue.',
    );
  });

  it('labels the trend with the days the served series covers, not the window it aims for', async () => {
    // A thin history: nine days served out of the window the server aims for.
    const trend = Array.from({ length: 9 }, (_, index) => ({
      date: `2026-09-${String(12 + index).padStart(2, '0')}`,
      band: 'productive' as const,
      pct_of_fitness: -10 - index,
    }));
    api.getTrainingStatus.mockResolvedValue(status({ trend }));
    renderStatus();

    await screen.findByTestId('home-status-trend');
    expect(screen.getByTestId('home-status')).toHaveTextContent('Your form over the last 8 days');
    expect(screen.getByTestId('home-status')).not.toHaveTextContent('42 days');
  });

  it('labels a full window with its own span: 43 points are 42 days', async () => {
    const trend = Array.from({ length: 43 }, (_, index) => ({
      date: new Date(Date.UTC(2026, 7, 9 + index)).toISOString().slice(0, 10),
      band: 'balanced' as const,
      pct_of_fitness: -3,
    }));
    api.getTrainingStatus.mockResolvedValue(status({ trend }));
    renderStatus();

    await screen.findByTestId('home-status-trend');
    expect(screen.getByTestId('home-status')).toHaveTextContent('Your form over the last 42 days');
  });

  it('reads out the day under the pointer, and the hint again once the pointer leaves', async () => {
    api.getTrainingStatus.mockResolvedValue(status());
    renderStatus();

    const chart = await screen.findByTestId('home-status-trend');
    const frame = chart.parentElement as HTMLElement;
    frame.getBoundingClientRect = () => ({ left: 0, top: 0, width: 640, height: 64 }) as DOMRect;
    // jsdom has no PointerEvent, and the event it builds in its place drops
    // clientX; a MouseEvent of the same type carries it to the same handler.
    fireEvent(frame, new MouseEvent('pointermove', { bubbles: true, clientX: 330 }));
    expect(screen.getByTestId('home-status-trend-readout')).toHaveTextContent('Sat, Sep 19 · +6% · Fresh');
    fireEvent(frame, new MouseEvent('pointerout', { bubbles: true }));
    expect(screen.getByTestId('home-status-trend-readout')).toHaveTextContent('Above the line');
  });

  it('words several recovery days, and one day with no figure when form cannot be scaled', async () => {
    api.getTrainingStatus.mockResolvedValue(
      status({
        form: { band: 'insufficient_history', pct_of_fitness: null },
        trend: [{ date: '2026-09-20', band: 'insufficient_history', pct_of_fitness: null }],
        load_ratio: null,
        recovery_days: null,
      }),
    );
    renderStatus();

    expect(await screen.findByTestId('home-status-band')).toHaveTextContent('Not enough history');
    expect(screen.queryByTestId('home-status-form')).not.toBeInTheDocument();
    expect(screen.queryByTestId('home-status-trend')).not.toBeInTheDocument();
    expect(screen.getByTestId('home-status-trend-short')).toHaveTextContent(
      'Your trend shows up here as the days add up.',
    );
    expect(screen.queryByTestId('home-status-load')).not.toBeInTheDocument();
    expect(screen.queryByTestId('home-status-recovery')).not.toBeInTheDocument();
  });

  it('says how many lighter days deep fatigue calls for', async () => {
    api.getTrainingStatus.mockResolvedValue(
      status({ form: { band: 'deep_fatigue', pct_of_fitness: -45 }, recovery_days: 2 }),
    );
    renderStatus();

    expect(await screen.findByTestId('home-status-band')).toHaveTextContent('Deep fatigue');
    expect(screen.getByTestId('home-status-recovery')).toHaveTextContent('Your form calls for 2 lighter days.');
  });

  it('answers a thin history with a sentence — no band, no figure, no chart', async () => {
    api.getTrainingStatus.mockResolvedValue(
      status({ form: null, trend: [], load_ratio: null, recovery_days: null }),
    );
    renderStatus();

    expect(await screen.findByTestId('home-status-empty')).toHaveTextContent(
      'Not enough training history yet to read your form.',
    );
    expect(screen.queryByTestId('home-status-reading')).not.toBeInTheDocument();
    expect(screen.getByTestId('home-status')).not.toHaveTextContent('0%');
  });

  it('says a failed read failed, never that the history is thin, and reads again on retry', async () => {
    api.getTrainingStatus.mockRejectedValueOnce(new Error('offline'));
    renderStatus();

    const failed = await screen.findByTestId('home-status-failed');
    expect(failed).toHaveTextContent("Your training status couldn't be loaded.");
    expect(screen.queryByTestId('home-status-empty')).not.toBeInTheDocument();

    api.getTrainingStatus.mockResolvedValue(status());
    await userEvent.click(within(failed).getByRole('button', { name: 'Retry' }));
    expect(await screen.findByTestId('home-status-band')).toHaveTextContent('Heavy block');
    expect(api.getTrainingStatus).toHaveBeenCalledTimes(2);
  });
});
