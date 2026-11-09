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
  healthStatus,
} from '../MemoryExtractionMonitorTab';
import type {
  ExtractionRunCounts,
  ExtractionRunStats,
  MemoryWorkerMetricsResponse,
  UserFactMetrics,
} from '../../services/api/admin';

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

function counts(overrides: Partial<ExtractionRunCounts> = {}): ExtractionRunCounts {
  return { persisted: 0, empty: 0, failed: 0, facts_persisted: 0, ...overrides };
}

function sampleRuns(overrides: Partial<ExtractionRunStats> = {}): ExtractionRunStats {
  return {
    last_24h: counts({ persisted: 3, empty: 5, facts_persisted: 7 }),
    last_7d: counts({ persisted: 20, empty: 30, facts_persisted: 41 }),
    last_run_at_ms: Date.parse('2026-04-13T18:00:00Z'),
    last_failure_at_ms: null,
    backlog: 0,
    oldest_backlog_at_ms: null,
    ...overrides,
  };
}

function sampleResponse(
  runs: ExtractionRunStats = sampleRuns(),
  metrics: UserFactMetrics = sampleMetrics(),
): MemoryWorkerMetricsResponse {
  return { scope: { kind: 'tenant', tenant_id: 'tenant-a' }, runs, metrics };
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

  it('renders the healthy status from runs and the store figures beside them', async () => {
    vi.mocked(adminApi.getMemoryWorkerMetrics).mockResolvedValueOnce(sampleResponse());
    renderTab();
    await waitFor(() => {
      expect(screen.getByText('Healthy')).toBeInTheDocument();
    });
    expect(screen.getByText(/^120$/)).toBeInTheDocument();
    // 3 persisted + 5 empty: a run that found nothing still counts as a run.
    expect(screen.getByText('Runs, last 24 hours').parentElement).toHaveTextContent('8');
  });

  it('asks for no tenant as a plain admin and states the server-reported scope', async () => {
    vi.mocked(adminApi.getMemoryWorkerMetrics).mockResolvedValueOnce(sampleResponse());
    renderTab();
    await waitFor(() => {
      expect(screen.getByTestId('admin-scope-statement')).toHaveTextContent(
        'Showing your tenant only.',
      );
    });
    expect(adminApi.getMemoryWorkerMetrics).toHaveBeenCalledWith(undefined);
  });

  it('reads a zero fact counter as healthy when the worker ran and found nothing', async () => {
    vi.mocked(adminApi.getMemoryWorkerMetrics).mockResolvedValueOnce(
      sampleResponse(
        sampleRuns({ last_24h: counts({ empty: 4 }) }),
        sampleMetrics({ total_facts: 100, facts_last_24h: 0, facts_last_7d: 0 }),
      ),
    );
    renderTab();
    await waitFor(() => {
      expect(screen.getByText('Healthy')).toBeInTheDocument();
    });
  });
});

describe('healthStatus', () => {
  it('names each state from the run ledger', () => {
    const label = (overrides: Partial<ExtractionRunStats>) =>
      healthStatus(sampleRuns(overrides)).label;
    const none = { last_24h: counts(), last_7d: counts() };
    expect(label({ last_24h: counts({ failed: 3 }) })).toBe('Failing');
    expect(label({ last_24h: counts({ failed: 1, persisted: 2 }) })).toBe('Degraded');
    expect(label({ backlog: 2 })).toBe('Backlog');
    expect(label({ last_24h: counts({ empty: 1 }) })).toBe('Healthy');
    expect(label({ last_24h: counts() })).toBe('Quiet');
    expect(label(none)).toBe('Idle');
    expect(label({ ...none, backlog: 1 })).toBe('Backlog');
  });
});

describe('MemoryExtractionMonitorTab breakdown', () => {
  beforeEach(() => {
    vi.clearAllMocks();
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
