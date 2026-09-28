// ABOUTME: Tests for A2AClientList — a selected client's panel renders its calls and the budget they spend
// ABOUTME: Pins the usage counts, the per-day breakdown, the row budget and that no tier is ever shown
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, within } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import A2AClientList from '../A2AClientList';
import type { A2AClient, A2ARateLimitStatus, A2AUsageStats } from '../../types/api';

const getA2AClients = vi.fn();
const getA2AClientUsage = vi.fn();
const getA2AClientRateLimit = vi.fn();

vi.mock('../../services/api', () => ({
  a2aApi: {
    getA2AClients: (...args: unknown[]) => getA2AClients(...args),
    getA2AClientUsage: (...args: unknown[]) => getA2AClientUsage(...args),
    getA2AClientRateLimit: (...args: unknown[]) => getA2AClientRateLimit(...args),
    deactivateA2AClient: vi.fn(),
  },
}));

const client: A2AClient = {
  id: 'a2a_client_1',
  name: 'Training Planner',
  description: 'Plans the week from recent load',
  capabilities: ['fitness-data-analysis'],
  redirect_uris: [],
  is_verified: false,
  is_active: true,
  created_at: '2026-09-01T10:00:00Z',
  updated_at: '2026-09-01T10:00:00Z',
};

const usage: A2AUsageStats = {
  client_id: client.id,
  requests_today: 3,
  requests_this_month: 40,
  total_requests: 52,
  last_request_at: '2026-09-28T09:15:00Z',
  daily_usage: [
    { date: '2026-09-28', success_count: 2, error_count: 1 },
    { date: '2026-09-27', success_count: 5, error_count: 0 },
  ],
};

function budget(overrides: Partial<A2ARateLimitStatus> = {}): A2ARateLimitStatus {
  return {
    client_id: client.id,
    is_rate_limited: false,
    rate_limit_requests: 1000,
    rate_limit_window_seconds: 3600,
    current_usage: 250,
    remaining: 750,
    reset_at: '2026-09-28T14:30:00Z',
    ...overrides,
  };
}

function renderSelected() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  render(
    <QueryClientProvider client={queryClient}>
      <A2AClientList />
    </QueryClientProvider>,
  );
  return screen.findByText(client.name).then((name) => fireEvent.click(name));
}

/** The value printed beside `label` in the rate-limit panel. */
function valueBeside(label: string): string | null {
  const row = screen.getByText(label).parentElement as HTMLElement;
  return within(row).getAllByText(/.+/).at(-1)?.textContent ?? null;
}

describe('A2AClientList rate-limit panel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    getA2AClients.mockResolvedValue([client]);
    getA2AClientUsage.mockResolvedValue(usage);
  });

  it('renders the client row budget the server enforces', async () => {
    getA2AClientRateLimit.mockResolvedValue(budget());
    await renderSelected();

    await screen.findByText('Rate Limits');
    expect(getA2AClientRateLimit).toHaveBeenCalledWith(client.id);
    expect(valueBeside('Limit:')).toBe((1000).toLocaleString());
    expect(valueBeside('Window:')).toBe(
      new Intl.NumberFormat('en', { style: 'unit', unit: 'hour', unitDisplay: 'long' }).format(1),
    );
    expect(valueBeside('Remaining:')).toBe((750).toLocaleString());
    expect(valueBeside('Resets:')).toBe(
      new Intl.DateTimeFormat('en', { dateStyle: 'medium', timeStyle: 'short' }).format(
        new Date('2026-09-28T14:30:00Z'),
      ),
    );
  });

  it('names a window that is not whole hours in minutes', async () => {
    getA2AClientRateLimit.mockResolvedValue(budget({ rate_limit_window_seconds: 900 }));
    await renderSelected();

    await screen.findByText('Window:');
    expect(valueBeside('Window:')).toBe(
      new Intl.NumberFormat('en', { style: 'unit', unit: 'minute', unitDisplay: 'long' }).format(15),
    );
  });

  it('shows a spent budget as an error and never a tier', async () => {
    getA2AClientRateLimit.mockResolvedValue(
      budget({ is_rate_limited: true, current_usage: 1000, remaining: 0 }),
    );
    await renderSelected();

    await screen.findByText('Remaining:');
    const remaining = screen.getByText('Remaining:').parentElement as HTMLElement;
    expect(within(remaining).getByText('0')).toHaveClass('text-error');
    expect(screen.queryByText(/Tier/)).not.toBeInTheDocument();
    expect(screen.queryByText('Trial')).not.toBeInTheDocument();
    expect(screen.queryByText(/Monthly/)).not.toBeInTheDocument();
  });
});

describe('A2AClientList usage panel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    getA2AClients.mockResolvedValue([client]);
    getA2AClientRateLimit.mockResolvedValue(budget());
  });

  const day = (iso: string) =>
    new Intl.DateTimeFormat('en', { year: 'numeric', month: 'short', day: 'numeric', timeZone: 'UTC' }).format(
      new Date(iso),
    );

  it('renders the counts the usage route serves', async () => {
    getA2AClientUsage.mockResolvedValue(usage);
    await renderSelected();

    await screen.findByText('Usage Statistics');
    expect(getA2AClientUsage).toHaveBeenCalledWith(client.id);
    expect(valueBeside('Today:')).toBe('3');
    expect(valueBeside('This Month:')).toBe('40');
    expect(valueBeside('Total:')).toBe('52');
  });

  it('lists each day newest first with its calls and failures', async () => {
    getA2AClientUsage.mockResolvedValue(usage);
    await renderSelected();

    const list = await screen.findByRole('list', { name: 'Daily Requests' });
    const rows = within(list).getAllByRole('listitem');
    expect(rows.map((row) => row.textContent)).toEqual([
      `${day('2026-09-28')}3 · 1 failed`,
      `${day('2026-09-27')}5`,
    ]);
    expect(within(rows[0]).getByText(/1 failed/)).toHaveClass('text-error');
    expect(screen.queryByText('Top Tools')).not.toBeInTheDocument();
  });

  it('says so when the client has made no call', async () => {
    getA2AClientUsage.mockResolvedValue({
      ...usage,
      requests_today: 0,
      requests_this_month: 0,
      total_requests: 0,
      last_request_at: null,
      daily_usage: [],
    });
    await renderSelected();

    await screen.findByText('No requests yet');
    expect(screen.queryByRole('list', { name: 'Daily Requests' })).not.toBeInTheDocument();
    expect(screen.queryByText('Last Request:')).not.toBeInTheDocument();
  });
});
