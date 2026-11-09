// ABOUTME: Sprint C14 tests — CoachGradingTab renders grades sorted worst-first
// ABOUTME: Mocks adminApi.getCoachGradingSummary and asserts table rendering
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import CoachGradingTab from '../CoachGradingTab';
import type { AgentGradingSummary } from '../../services/api/admin';

vi.mock('../../services/api/admin', async () => ({
  adminApi: {
    getCoachGradingSummary: vi.fn(),
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

function sampleSummary(overrides: Partial<AgentGradingSummary> = {}): AgentGradingSummary {
  return {
    scope: { kind: 'tenant', tenant_id: 'tenant-a' },
    verdicts_scanned: 100,
    judge_era_after: '2026-08-11',
    verdicts_before_judge_era: 60,
    unattributed_verdicts: 8,
    grades: [
      {
        agent_id: 'coach-broscience',
        agent_name: 'Bro Science',
        total_verdicts: 12,
        supported: 2,
        unsupported: 6,
        unsupported_prescription: 0,
        contradicted: 4,
        rhetorical: 0,
        unverifiable: 0,
        false_positives: 0,
        score: 0.12,
        grade: 'F',
      },
      {
        agent_id: 'coach-stable',
        agent_name: null,
        total_verdicts: 20,
        supported: 15,
        unsupported: 1,
        unsupported_prescription: 1,
        contradicted: 0,
        rhetorical: 1,
        unverifiable: 1,
        false_positives: 1,
        score: 0.93,
        grade: 'A',
      },
    ],
    ...overrides,
  };
}

function renderTab() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <CoachGradingTab />
    </QueryClientProvider>,
  );
}

describe('CoachGradingTab', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders the empty state when no agents have grades yet', async () => {
    vi.mocked(adminApi.getCoachGradingSummary).mockResolvedValueOnce(
      sampleSummary({ grades: [] }),
    );
    renderTab();
    await waitFor(() => {
      expect(screen.getByText(/No agent grades available yet/i)).toBeInTheDocument();
    });
  });

  it('renders agent grades with badges and scores', async () => {
    vi.mocked(adminApi.getCoachGradingSummary).mockResolvedValueOnce(sampleSummary());
    renderTab();
    await waitFor(() => {
      expect(screen.getByText('coach-broscience')).toBeInTheDocument();
    });
    expect(screen.getByText('coach-stable')).toBeInTheDocument();
    expect(screen.getByText('F')).toBeInTheDocument();
    expect(screen.getByText('A')).toBeInTheDocument();
    // Scores rendered as percentages (12% and 93%).
    expect(screen.getByText('12%')).toBeInTheDocument();
    expect(screen.getByText('93%')).toBeInTheDocument();
  });

  it('counts failing grades in the header card', async () => {
    vi.mocked(adminApi.getCoachGradingSummary).mockResolvedValueOnce(sampleSummary());
    renderTab();
    // Wait for the table to actually mount (with the F-graded agent row)
    // so we know the query data has propagated, then assert the metric card.
    await waitFor(() => {
      expect(screen.getByText('coach-broscience')).toBeInTheDocument();
    });
    const failingLabel = screen.getByText('Low grades (D / F)');
    const card = failingLabel.closest('.rounded-lg');
    expect(card?.textContent ?? '').toContain('1');
    expect(screen.queryByText(/Failing/i)).not.toBeInTheDocument();
  });

  it('names the agent, and says so when the id no longer resolves', async () => {
    vi.mocked(adminApi.getCoachGradingSummary).mockResolvedValueOnce(sampleSummary());
    renderTab();
    await waitFor(() => {
      expect(screen.getByText('Bro Science')).toBeInTheDocument();
    });
    expect(screen.getByText('Agent not found')).toBeInTheDocument();
  });

  it('makes each row add up to its total', async () => {
    vi.mocked(adminApi.getCoachGradingSummary).mockResolvedValueOnce(sampleSummary());
    renderTab();
    await waitFor(() => {
      expect(screen.getAllByTestId('not-scored')).toHaveLength(2);
    });
    const [none, four] = screen.getAllByTestId('not-scored');
    expect(none).toHaveTextContent('0');
    // 15 supported + 1 unsupported + 0 contradicted + 4 not scored = 20 total.
    expect(four).toHaveTextContent('4');
  });

  it('states the judge-era cut and what it leaves out', async () => {
    vi.mocked(adminApi.getCoachGradingSummary).mockResolvedValueOnce(sampleSummary());
    renderTab();
    await waitFor(() => {
      expect(screen.getByTestId('judge-era-after')).toHaveTextContent('2026-08-11');
    });
    expect(screen.getByTestId('judge-era-verdicts')).toHaveTextContent('40');
    expect(screen.getByTestId('older-verdicts')).toHaveTextContent('60 older not counted');
    expect(screen.getByTestId('unattributed-verdicts')).toHaveTextContent('8');
  });

  it('makes no claim about store rank or an unpublish action', async () => {
    vi.mocked(adminApi.getCoachGradingSummary).mockResolvedValueOnce(sampleSummary());
    renderTab();
    await waitFor(() => {
      expect(screen.getByTestId('judge-era-after')).toBeInTheDocument();
    });
    expect(screen.queryByText(/rank higher/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/unpublish/i)).not.toBeInTheDocument();
  });
});
