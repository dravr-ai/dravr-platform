// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The admin Activity tab shows each recent LLM call's cache read / write split and its 1-hour share
// ABOUTME: A call with no cache activity reads as a dash, and the expanded row spells out all three counts

import { beforeEach, describe, it, expect, vi } from 'vitest';
import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import ActivityTab from '../ActivityTab';
import type { RecentActivityResponse, RecentLlmCall } from '../../services/api/dashboard';

function llmCall(overrides: Partial<RecentLlmCall>): RecentLlmCall {
  return {
    id: 'call',
    provider: 'anthropic',
    model: 'claude-sonnet-4-5',
    total_tokens: 12_400,
    cached_tokens: 0,
    cached_write_tokens: 0,
    cached_write_1h_tokens: 0,
    cost_usd: 0.01,
    call_type: 'chat',
    execution_time_ms: 900,
    created_at: new Date(Date.now() - 60_000).toISOString(),
    ...overrides,
  };
}

const activity: RecentActivityResponse = {
  recent_llm_calls: [
    llmCall({ id: 'cached', cached_tokens: 9_000, cached_write_tokens: 2_500, cached_write_1h_tokens: 1_500 }),
    llmCall({ id: 'uncached', model: 'gpt-4o-mini', created_at: new Date(Date.now() - 120_000).toISOString() }),
  ],
  recent_conversations: [],
  recent_limit: 20,
  scope: { kind: 'all_tenants' },
  summary: { active_conversations: 0, llm_calls_today: 2, total_tokens_today: 24_800, cost_today_usd: 0.02 },
};

const getRecentActivity = vi.fn<(tenantId?: string) => Promise<RecentActivityResponse>>();
const role = vi.hoisted(() => ({ current: 'admin' }));

vi.mock('../../services/api', () => ({
  dashboardApi: { getRecentActivity: (tenantId?: string) => getRecentActivity(tenantId) },
}));

vi.mock('../../hooks/useAuth', () => ({
  useAuth: () => ({ user: { role: role.current } }),
}));

vi.mock('../../services/api/admin', () => ({
  adminApi: { listScopeTenants: vi.fn().mockResolvedValue([]) },
}));

beforeEach(() => {
  localStorage.clear();
  role.current = 'admin';
  getRecentActivity.mockReset();
  getRecentActivity.mockResolvedValue(activity);
});

function renderTab() {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={queryClient}>
      <ActivityTab />
    </QueryClientProvider>,
  );
}

describe('ActivityTab scope', () => {
  it('states the scope the server read, and a plain admin asks for no tenant', async () => {
    getRecentActivity.mockResolvedValue({
      ...activity,
      scope: { kind: 'tenant', tenant_id: 'mine' },
    });
    renderTab();

    expect(await screen.findByTestId('admin-scope-statement')).toHaveTextContent(
      'Showing your tenant only.',
    );
    expect(getRecentActivity).toHaveBeenCalledWith(undefined);
  });

  it('asks for the tenant a super admin picked', async () => {
    role.current = 'super_admin';
    localStorage.setItem('pierre.admin_scope', 'picked-tenant');
    getRecentActivity.mockResolvedValue({
      ...activity,
      scope: { kind: 'tenant', tenant_id: 'picked-tenant' },
    });
    renderTab();

    expect(await screen.findByTestId('admin-scope-statement')).toHaveTextContent(
      'Showing one tenant: picked-tenant.',
    );
    expect(getRecentActivity).toHaveBeenCalledWith('picked-tenant');
  });
});

describe('ActivityTab cache column', () => {
  it('shows read / write and the 1-hour share of the write', async () => {
    renderTab();

    expect(await screen.findByText('Cache')).toBeInTheDocument();
    const cell = screen.getByTestId('llm-cache-cell');
    expect(cell).toHaveTextContent('9.0K / 2.5K');
    expect(within(cell).getByText('1h: 1.5K')).toBeInTheDocument();
  });

  it('shows a dash for a call with no cache activity', async () => {
    renderTab();

    expect(await screen.findByLabelText('No cache activity')).toHaveTextContent('—');
    expect(screen.getAllByTestId('llm-cache-cell')).toHaveLength(1);
  });

  it('spells out all three counts in the expanded row', async () => {
    renderTab();

    await userEvent.click(await screen.findByText('anthropic/claude-sonnet-4-5'));

    expect(screen.getByText('Cache Read').nextElementSibling).toHaveTextContent((9_000).toLocaleString());
    expect(screen.getByText('Cache Write').nextElementSibling).toHaveTextContent((2_500).toLocaleString());
    expect(screen.getByText('Cache Write (1h)').nextElementSibling).toHaveTextContent((1_500).toLocaleString());
  });
});

describe('ActivityTab says what its numbers are', () => {
  it('states the lists are capped, and never reads their length as a total', async () => {
    renderTab();

    expect(await screen.findByText('LLM Calls (latest 2)')).toBeInTheDocument();
    expect(screen.getByText('All recent')).toBeInTheDocument();
    expect(screen.getByTestId('activity-cap')).toHaveTextContent('Each list stops at 20 rows');
    expect(screen.getByTestId('activity-cap')).toHaveTextContent('search only reads these rows');
  });

  it('labels today as UTC and the cost as a sum at current prices', async () => {
    renderTab();

    expect(await screen.findByText('LLM Calls Today (UTC)')).toBeInTheDocument();
    expect(screen.getByText('Cost Today (UTC, at current prices)')).toBeInTheDocument();
    expect(screen.queryByText(/Est\. Cost/)).not.toBeInTheDocument();
  });

  it('shows a failed read as a failure, not as zero activity', async () => {
    getRecentActivity.mockRejectedValue(new Error('database down'));
    renderTab();

    expect(await screen.findByRole('alert')).toHaveTextContent('could not be loaded');
    expect(screen.queryByText('LLM Calls Today (UTC)')).not.toBeInTheDocument();
    expect(screen.queryByText('No activity yet')).not.toBeInTheDocument();
  });

  it('shows a conversation owner once, and keeps them searchable', async () => {
    getRecentActivity.mockResolvedValue({
      ...activity,
      recent_conversations: [
        {
          id: 'c1',
          title: 'Taper week',
          updated_at: new Date(Date.now() - 120_000).toISOString(),
          user_email: 'athlete@example.com',
        },
      ],
    });
    renderTab();

    expect(await screen.findAllByText('athlete@example.com')).toHaveLength(1);
    await userEvent.type(screen.getByLabelText('Search activity'), 'athlete@');
    expect(screen.getByText('Taper week')).toBeInTheDocument();
    expect(screen.queryByText('anthropic/claude-sonnet-4-5')).not.toBeInTheDocument();
  });
});
