// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Engagement tab reads every figure in the one scope the viewer picked and states it
// ABOUTME: A super admin's figures are the platform by default; a plain admin's are their tenant's, users included

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import EngagementTab from '../EngagementTab';
import type { EngagementSummary } from '../../services/api/admin';

const TENANT = '11111111-1111-4111-8111-111111111111';
const role = vi.hoisted(() => ({ current: 'super_admin' }));

vi.mock('../../hooks/useAuth', () => ({
  useAuth: () => ({ user: { role: role.current } }),
}));

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
    role.current = 'super_admin';
    vi.mocked(adminApi.getEngagement).mockResolvedValue(engagement());
  });

  it('states that a super admin reads the series across every tenant', async () => {
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

  it('keeps a plain admin on their own tenant and sends no tenant', async () => {
    role.current = 'admin';
    vi.mocked(dashboardApi.getUsageAnalytics).mockResolvedValue({
      scope: { kind: 'tenant', tenant_id: TENANT },
      time_series: [],
    });
    vi.mocked(adminApi.getEngagement).mockResolvedValue(
      engagement({ scope: { kind: 'tenant', tenant_id: TENANT } }),
    );
    renderTab();

    expect(await screen.findByTestId('admin-scope-statement')).toHaveTextContent(
      'Showing your tenant only.',
    );
    await waitFor(() =>
      expect(dashboardApi.getUsageAnalytics).toHaveBeenCalledWith(expect.anything(), undefined),
    );
    expect(screen.queryByLabelText('Tenant scope')).not.toBeInTheDocument();
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
  });

  it('shows a plain admin their own tenant\'s users, asked for in the same scope as the series', async () => {
    role.current = 'admin';
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
    expect(adminApi.getEngagement).toHaveBeenCalledWith(undefined);
    expect(screen.getByText('Total Users').nextSibling).toHaveTextContent('3');
  });
});
