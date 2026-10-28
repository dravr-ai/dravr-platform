// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The admin Activity tab shows each recent LLM call's cache read / write split and its 1-hour share
// ABOUTME: A call with no cache activity reads as a dash, and the expanded row spells out all three counts

import { describe, it, expect, vi } from 'vitest';
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
  summary: { active_conversations: 0, llm_calls_today: 2, total_tokens_today: 24_800, estimated_cost_today: 0.02 },
};

vi.mock('../../services/api', () => ({
  dashboardApi: { getRecentActivity: () => Promise.resolve(activity) },
}));

function renderTab() {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={queryClient}>
      <ActivityTab />
    </QueryClientProvider>,
  );
}

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
