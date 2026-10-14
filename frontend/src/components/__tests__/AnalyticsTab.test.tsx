// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Analytics tab shows its three panels together, once every query behind them has settled
// ABOUTME: One by one, a panel landing above another would push it down the page — a layout shift of ~1.0

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, act, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import AnalyticsTab from '../AnalyticsTab';

/** A promise per endpoint, settled by the test when it chooses. */
const pending = vi.hoisted(() => {
  const make = () => {
    let resolve: (value: unknown) => void = () => undefined;
    let reject: (reason: unknown) => void = () => undefined;
    const promise = new Promise((res, rej) => {
      resolve = res;
      reject = rej;
    });
    return { promise, resolve, reject };
  };
  return { make, usage: make(), llm: make(), tools: make() };
});

vi.mock('../../services/api', () => ({
  dashboardApi: { getUsageAnalytics: () => pending.usage.promise },
  adminUsageApi: {
    getAdminLlmConsumption: () => pending.llm.promise,
    getAdminToolUsage: () => pending.tools.promise,
  },
}));

vi.mock('../UsageAnalytics', () => ({ default: () => <div data-testid="usage-panel" /> }));
vi.mock('../LlmConsumptionPanel', () => ({ default: () => <div data-testid="llm-panel" /> }));
vi.mock('../ToolUsagePanel', () => ({ default: () => <div data-testid="tools-panel" /> }));

let settledQueries = 0;

function renderTab() {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  settledQueries = 0;
  queryClient.getQueryCache().subscribe((event) => {
    if (event.type === 'updated' && event.action.type === 'success') settledQueries += 1;
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <AnalyticsTab />
    </QueryClientProvider>,
  );
}

const PANELS = ['usage-panel', 'llm-panel', 'tools-panel'];

describe('AnalyticsTab', () => {
  beforeEach(() => {
    pending.usage = pending.make();
    pending.llm = pending.make();
    pending.tools = pending.make();
  });

  it('holds every panel back until all three queries settle, then shows them at once', async () => {
    renderTab();
    expect(screen.getByRole('status', { name: 'Loading analytics' })).toBeInTheDocument();

    // The two lower panels' data lands first — the order that would push them down.
    await act(async () => {
      pending.tools.resolve({ tools: [] });
      pending.llm.resolve({ daily: [], breakdown: [] });
    });
    // Give React Query the ticks to record both results before checking nothing showed.
    await waitFor(() => expect(settledQueries).toBe(2));
    for (const panel of PANELS) {
      expect(screen.queryByTestId(panel)).toBeNull();
    }

    await act(async () => {
      pending.usage.resolve({ time_series: [], top_tools: [] });
    });
    for (const panel of PANELS) {
      expect(await screen.findByTestId(panel)).toBeInTheDocument();
    }
  });

  it('reveals the panels when a query fails, so each can show its own error', async () => {
    renderTab();

    await act(async () => {
      pending.usage.resolve({ time_series: [], top_tools: [] });
      pending.llm.reject(new Error('403'));
      pending.tools.reject(new Error('503'));
    });

    for (const panel of PANELS) {
      expect(await screen.findByTestId(panel)).toBeInTheDocument();
    }
  });
});
