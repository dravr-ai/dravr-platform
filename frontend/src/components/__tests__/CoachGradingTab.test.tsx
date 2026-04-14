// ABOUTME: Sprint C14 tests — CoachGradingTab renders grades sorted worst-first
// ABOUTME: Mocks adminApi.getCoachGradingSummary and asserts table rendering
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import CoachGradingTab from '../CoachGradingTab';
import type { CoachGradingSummary } from '../../services/api/admin';

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

function sampleSummary(overrides: Partial<CoachGradingSummary> = {}): CoachGradingSummary {
  return {
    tenant_id: 'tenant-a',
    verdicts_scanned: 100,
    grades: [
      {
        coach_id: 'coach-broscience',
        total_verdicts: 12,
        supported: 2,
        unsupported: 6,
        contradicted: 4,
        rhetorical: 0,
        unverifiable: 0,
        score: 0.12,
        grade: 'F',
      },
      {
        coach_id: 'coach-stable',
        total_verdicts: 20,
        supported: 18,
        unsupported: 1,
        contradicted: 0,
        rhetorical: 1,
        unverifiable: 0,
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

  it('renders the empty state when no coaches have grades yet', async () => {
    vi.mocked(adminApi.getCoachGradingSummary).mockResolvedValueOnce(
      sampleSummary({ grades: [] }),
    );
    renderTab();
    await waitFor(() => {
      expect(screen.getByText(/No coach grades available yet/i)).toBeInTheDocument();
    });
  });

  it('renders coach grades with badges and scores', async () => {
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
    // Wait for the table to actually mount (with the F-graded coach row)
    // so we know the query data has propagated, then assert the metric card.
    await waitFor(() => {
      expect(screen.getByText('coach-broscience')).toBeInTheDocument();
    });
    const failingLabel = screen.getByText(/Failing/i);
    const card = failingLabel.closest('.rounded-lg');
    expect(card?.textContent ?? '').toContain('1');
  });
});
