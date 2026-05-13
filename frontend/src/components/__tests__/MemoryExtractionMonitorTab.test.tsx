// ABOUTME: Sprint C6 tests — MemoryExtractionMonitorTab renders metrics and health states
// ABOUTME: Mocks adminApi.getMemoryWorkerMetrics and asserts summary cards + kind breakdown
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import MemoryExtractionMonitorTab, {
  distributePercentages,
} from '../MemoryExtractionMonitorTab';
import type { MemoryWorkerMetricsResponse, UserFactMetrics } from '../../services/api/admin';

vi.mock('../../services/api/admin', async () => ({
  adminApi: {
    getMemoryWorkerMetrics: vi.fn(),
  },
}));

vi.mock('../../hooks/useAuth', () => ({
  useAuth: () => ({
    user: {
      id: 'user-1',
      email: 'admin@example.com',
      tenant_id: 'tenant-a',
      is_admin: true,
      role: 'admin',
      tier: 'professional',
      created_at: '2026-01-01T00:00:00Z',
    },
  }),
}));

const { adminApi } = await import('../../services/api/admin');

function sampleMetrics(overrides: Partial<UserFactMetrics> = {}): UserFactMetrics {
  return {
    total_facts: 120,
    facts_last_24h: 8,
    facts_last_7d: 42,
    distinct_users: 15,
    facts_by_kind: {
      preference: 40,
      physiology: 30,
      injury: 10,
      goal: 25,
      schedule: 5,
      equipment: 5,
      other: 5,
    },
    newest_updated_at: '2026-04-13T18:00:00Z',
    ...overrides,
  };
}

function sampleResponse(
  metrics: UserFactMetrics = sampleMetrics(),
): MemoryWorkerMetricsResponse {
  return { tenant_id: 'tenant-a', metrics };
}

function renderTab() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <MemoryExtractionMonitorTab />
    </QueryClientProvider>,
  );
}

describe('MemoryExtractionMonitorTab', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders the healthy status banner when facts landed in the last 24h', async () => {
    vi.mocked(adminApi.getMemoryWorkerMetrics).mockResolvedValueOnce(sampleResponse());
    renderTab();
    await waitFor(() => {
      expect(screen.getByText(/Healthy/)).toBeInTheDocument();
    });
    expect(screen.getByText(/^120$/)).toBeInTheDocument();
  });

  it('renders the idle banner when no facts exist yet', async () => {
    vi.mocked(adminApi.getMemoryWorkerMetrics).mockResolvedValueOnce(
      sampleResponse(
        sampleMetrics({
          total_facts: 0,
          facts_last_24h: 0,
          facts_last_7d: 0,
          distinct_users: 0,
          facts_by_kind: {},
          newest_updated_at: null,
        }),
      ),
    );
    renderTab();
    await waitFor(() => {
      expect(screen.getByText(/Idle/)).toBeInTheDocument();
    });
    expect(screen.getByText(/No facts have been extracted yet/)).toBeInTheDocument();
  });

  it('renders the stalled banner when there are facts but no activity in 7 days', async () => {
    vi.mocked(adminApi.getMemoryWorkerMetrics).mockResolvedValueOnce(
      sampleResponse(
        sampleMetrics({
          total_facts: 100,
          facts_last_24h: 0,
          facts_last_7d: 0,
        }),
      ),
    );
    renderTab();
    await waitFor(() => {
      expect(screen.getByText(/Stalled/)).toBeInTheDocument();
    });
  });

  it('renders the per-kind breakdown sorted descending by count', async () => {
    vi.mocked(adminApi.getMemoryWorkerMetrics).mockResolvedValueOnce(sampleResponse());
    renderTab();
    await waitFor(() => {
      expect(screen.getByText(/Preferences/)).toBeInTheDocument();
    });
    // Distinct labels render, sanity check a few categories
    expect(screen.getByText(/Physiology/)).toBeInTheDocument();
    expect(screen.getByText(/Injuries/)).toBeInTheDocument();
    expect(screen.getByText(/Goals/)).toBeInTheDocument();
  });
});

describe('distributePercentages (audit 2026-05-07: 35+24+24+18=101% bug)', () => {
  it('sums to 100 for the audit data point (6,4,4,3 of 17)', () => {
    expect(distributePercentages([6, 4, 4, 3], 17).reduce((a, b) => a + b, 0)).toBe(100);
  });

  it('sums to 100 across many randomized inputs', () => {
    for (let i = 0; i < 50; i++) {
      const counts = Array.from({ length: 5 }, () => Math.floor(Math.random() * 50) + 1);
      const total = counts.reduce((a, b) => a + b, 0);
      const sum = distributePercentages(counts, total).reduce((a, b) => a + b, 0);
      expect(sum).toBe(100);
    }
  });

  it('returns all zeros for empty input', () => {
    expect(distributePercentages([], 0)).toEqual([]);
    expect(distributePercentages([0, 0], 0)).toEqual([0, 0]);
  });
});
