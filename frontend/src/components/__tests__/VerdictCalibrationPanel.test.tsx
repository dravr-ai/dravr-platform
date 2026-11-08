// ABOUTME: Tests for the verdict calibration panel on the Claim Verdicts screen
// ABOUTME: Mocks adminApi and asserts the pass rate, the status pills and the daily bars
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi } from 'vitest';
import { render, screen, within } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import VerdictCalibrationPanel from '../VerdictCalibrationPanel';
import type { VerdictCalibrationStats } from '../../services/api/admin';

vi.mock('../../hooks/useAuth', () => ({
  useAuth: () => ({ user: { role: 'super_admin' } }),
}));

vi.mock('../../services/api/admin', async () => ({
  adminApi: {
    getVerdictCalibrationStats: vi.fn(),
  },
}));

import { adminApi } from '../../services/api/admin';

function sampleCalibration(
  overrides: Partial<VerdictCalibrationStats> = {},
): VerdictCalibrationStats {
  return {
    window_start: '2026-03-14T12:00:00Z',
    window_days: 30,
    totals: {
      supported: 12,
      unsupported: 3,
      contradicted: 1,
      rhetorical: 4,
      unverifiable: 2,
    },
    daily: [
      {
        date: '2026-04-12',
        counts: {
          supported: 5,
          unsupported: 1,
          contradicted: 0,
          rhetorical: 1,
          unverifiable: 0,
        },
      },
      {
        date: '2026-04-13',
        counts: {
          supported: 7,
          unsupported: 2,
          contradicted: 1,
          rhetorical: 3,
          unverifiable: 2,
        },
      },
    ],
    ...overrides,
  };
}

function emptyCalibration(): VerdictCalibrationStats {
  return {
    window_start: '2026-03-14T12:00:00Z',
    window_days: 30,
    totals: {
      supported: 0,
      unsupported: 0,
      contradicted: 0,
      rhetorical: 0,
      unverifiable: 0,
    },
    daily: [],
  };
}

function renderPanel() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <VerdictCalibrationPanel />
    </QueryClientProvider>,
  );
}

describe('VerdictCalibrationPanel', () => {
  it('renders the pass rate and status pills over the viewer scope', async () => {
    vi.mocked(adminApi.getVerdictCalibrationStats).mockResolvedValueOnce(sampleCalibration());
    renderPanel();

    const panel = await screen.findByTestId('verdict-calibration-panel');
    // Pass rate = supported / (supported + unsupported + contradicted)
    //           = 12 / (12 + 3 + 1) = 75 %
    expect(within(panel).getByTestId('calibration-pass-rate')).toHaveTextContent('75%');
    expect(within(panel).getByText('Supported')).toBeInTheDocument();
    expect(within(panel).getByText('Contradicted')).toBeInTheDocument();
    expect(adminApi.getVerdictCalibrationStats).toHaveBeenCalledWith('all', 30);
  });

  it('draws every day with a pass rate in one colour, with no pass-rate bands', async () => {
    vi.mocked(adminApi.getVerdictCalibrationStats).mockResolvedValueOnce(sampleCalibration());
    renderPanel();

    const drift = await screen.findByTestId('calibration-daily-drift');
    const bars = Array.from(drift.querySelectorAll('div > div'));
    expect(bars.length).toBeGreaterThan(0);
    for (const bar of bars) {
      expect(bar.className).not.toMatch(/bg-(success|warning|error)/);
    }
  });

  it('shows an empty-window note and a dash when no verdicts exist in the window', async () => {
    vi.mocked(adminApi.getVerdictCalibrationStats).mockResolvedValueOnce(emptyCalibration());
    renderPanel();

    const panel = await screen.findByTestId('verdict-calibration-panel');
    expect(
      within(panel).getByText(/No verdicts recorded in the selected window/i),
    ).toBeInTheDocument();
    expect(within(panel).getByTestId('calibration-pass-rate')).toHaveTextContent('—');
  });

  it('reports a failed load', async () => {
    vi.mocked(adminApi.getVerdictCalibrationStats).mockRejectedValueOnce(new Error('503'));
    renderPanel();

    expect(await screen.findByText(/Failed to load verdict calibration stats/i)).toBeInTheDocument();
  });
});
