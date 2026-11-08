// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A failed Usage Analytics read says so instead of drawing the empty state
// ABOUTME: "No conversations yet" is a claim about the platform, so only a successful empty read may make it

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import UsageAnalytics from '../UsageAnalytics';
import { ThemeProvider } from '../../hooks/useTheme';

const getUsageAnalytics = vi.hoisted(() => vi.fn());

vi.mock('../../services/api', () => ({
  dashboardApi: { getUsageAnalytics },
  adminUsageApi: {},
}));

vi.mock('../../hooks/useAuth', () => ({
  useAuth: () => ({ user: { role: 'admin' } }),
}));

vi.mock('react-chartjs-2', () => ({ Line: () => <div data-testid="usage-chart" /> }));

function renderPanel() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <ThemeProvider>
        <UsageAnalytics />
      </ThemeProvider>
    </QueryClientProvider>,
  );
}

describe('UsageAnalytics', () => {
  beforeEach(() => {
    localStorage.clear();
    getUsageAnalytics.mockReset();
  });

  it('reports a failed read and never draws it as an empty platform', async () => {
    getUsageAnalytics.mockRejectedValue(new Error('500'));
    renderPanel();

    expect(await screen.findByRole('alert')).toHaveTextContent(
      'Usage analytics could not be loaded.',
    );
    expect(screen.queryByText('No conversations yet')).not.toBeInTheDocument();
  });

  it('keeps the empty state for a read that succeeded with nothing', async () => {
    getUsageAnalytics.mockResolvedValue({ time_series: [] });
    renderPanel();

    expect(await screen.findByText('No conversations yet')).toBeInTheDocument();
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });
});
