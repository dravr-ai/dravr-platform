// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Engagement tab reads every figure in the one scope the viewer picked and states it
// ABOUTME: An admin's figures are the platform by default; picking a tenant narrows them to it, users included

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import EngagementTab from '../EngagementTab';
import type { EngagementSummary } from '../../services/api/admin';

const TENANT = '11111111-1111-4111-8111-111111111111';

vi.mock('../../services/api', () => ({
  adminApi: {
    getEngagement: vi.fn(),
  },
  dashboardApi: { getUsageAnalytics: vi.fn() },
  adminUsageApi: {},
}));

vi.mock('../../services/api/admin', () => ({
  adminApi: { listScopeTenants: vi.fn().mockResolvedValue([]) },
}));

const { dashboardApi, adminApi } = await import('../../services/api');

function engagement(overrides: Partial<EngagementSummary> = {}): EngagementSummary {
  return {
    scope: { kind: 'all_tenants' },
    users: { total: 1, daily: 0, weekly: 0, monthly: 0, dormant: 1 },
    total_agents: 0,
    published_agents: 0,
    leaderboard: [],
    ...overrides,
  };
}

function renderTab() {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={queryClient}>
      <EngagementTab />
    </QueryClientProvider>,
  );
}

describe('EngagementTab', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
    vi.mocked(adminApi.getEngagement).mockResolvedValue(engagement());
  });

  it('states that an admin reads the series across every tenant', async () => {
    vi.mocked(dashboardApi.getUsageAnalytics).mockResolvedValue({
      scope: { kind: 'all_tenants' },
      time_series: [],
    });
    renderTab();

    expect(await screen.findByTestId('admin-scope-statement')).toHaveTextContent(
      'Showing every tenant on the platform.',
    );
    expect(
      screen.getByText('This scope applies to every figure. The time range moves only the Requests figure; everything else reads as of now.'),
    ).toBeInTheDocument();
    expect(dashboardApi.getUsageAnalytics).toHaveBeenCalledWith(expect.anything(), 'all');
    expect(adminApi.getEngagement).toHaveBeenCalledWith('all');
  });

  it('ranks the leaderboard by uses, not by prompt size', async () => {
    vi.mocked(dashboardApi.getUsageAnalytics).mockResolvedValue({
      scope: { kind: 'all_tenants' },
      time_series: [],
    });
    vi.mocked(adminApi.getEngagement).mockResolvedValueOnce(
      engagement({
        total_agents: 2,
        leaderboard: [
          { id: 'b', title: 'Short Prompt', category: 'training', use_count: 40 },
          { id: 'a', title: 'Long Prompt', category: 'training', use_count: 2 },
        ],
      }),
    );
    renderTab();

    await screen.findByText('Short Prompt');
    const rows = screen.getAllByRole('row').slice(1).map((row) => row.textContent);
    expect(rows[0]).toContain('Short Prompt');
    expect(rows[0]).toContain('40');
    expect(rows[1]).toContain('Long Prompt');
    expect(screen.getByRole('columnheader', { name: 'Uses' })).toBeInTheDocument();
    expect(screen.queryByRole('columnheader', { name: /Prompt Size/ })).toBeNull();
    // The category tag takes its pillar's tone, as on every admin screen.
    for (const tag of screen.getAllByText('training')) {
      expect(tag).toHaveClass('badge-activity');
    }
  });

  it('shows the picked tenant\'s users, asked for in the same scope as the series', async () => {
    localStorage.setItem('pierre.admin_scope', TENANT);
    vi.mocked(dashboardApi.getUsageAnalytics).mockResolvedValue({
      scope: { kind: 'tenant', tenant_id: TENANT },
      time_series: [],
    });
    vi.mocked(adminApi.getEngagement).mockResolvedValue(
      engagement({
        scope: { kind: 'tenant', tenant_id: TENANT },
        users: { total: 3, daily: 2, weekly: 3, monthly: 3, dormant: 0 },
      }),
    );
    renderTab();

    await screen.findByText('Total Users');
    expect(screen.getByTestId('admin-scope-statement')).toHaveTextContent(
      `Showing one tenant: ${TENANT}.`,
    );
    expect(screen.getByLabelText('Tenant scope')).toHaveValue(TENANT);
    expect(adminApi.getEngagement).toHaveBeenCalledWith(TENANT);
    expect(dashboardApi.getUsageAnalytics).toHaveBeenCalledWith(expect.anything(), TENANT);
    expect(screen.getByText('Total Users').nextSibling).toHaveTextContent('3');
  });
});
