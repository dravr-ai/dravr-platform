// ABOUTME: Sprint C13 tests — MythBustingTab renders aggregates and empty state
// ABOUTME: Mocks adminApi.getMythBustingSummary and asserts pattern lists render correctly
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import MythBustingTab from '../MythBustingTab';
import type { MythBustingSummary } from '../../services/api/admin';

vi.mock('../../services/api/admin', async () => ({
  adminApi: {
    getMythBustingSummary: vi.fn(),
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

function sampleSummary(overrides: Partial<MythBustingSummary> = {}): MythBustingSummary {
  return {
    tenant_id: 'tenant-a',
    verdicts_scanned: 200,
    flagged_total: 12,
    top_claims: [
      {
        claim_excerpt: 'Drink a gallon of water per day',
        occurrences: 4,
        coach_count: 2,
        last_seen_at: '2026-04-13T18:00:00Z',
      },
      {
        claim_excerpt: 'Cardio kills your gains',
        occurrences: 3,
        coach_count: 1,
        last_seen_at: '2026-04-12T18:00:00Z',
      },
    ],
    top_coaches: [
      {
        coach_id: 'coach-broscience',
        unsupported_total: 5,
        categories: ['nutrition', 'training_prescription'],
      },
    ],
    top_categories: [
      { category: 'nutrition', flagged_total: 6, coach_count: 3 },
      { category: 'supplement', flagged_total: 3, coach_count: 1 },
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
      <MythBustingTab />
    </QueryClientProvider>,
  );
}

describe('MythBustingTab', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders the empty state when no claims are flagged', async () => {
    vi.mocked(adminApi.getMythBustingSummary).mockResolvedValueOnce(
      sampleSummary({
        flagged_total: 0,
        top_claims: [],
        top_coaches: [],
        top_categories: [],
      }),
    );
    renderTab();
    await waitFor(() => {
      expect(
        screen.getByText(/No unsupported or contradicted claims/i),
      ).toBeInTheDocument();
    });
  });

  it('renders top recurring claims, coaches, and categories', async () => {
    vi.mocked(adminApi.getMythBustingSummary).mockResolvedValueOnce(sampleSummary());
    renderTab();
    await waitFor(() => {
      expect(screen.getByText(/Drink a gallon of water per day/)).toBeInTheDocument();
    });
    expect(screen.getByText(/Cardio kills your gains/)).toBeInTheDocument();
    expect(screen.getByText('coach-broscience')).toBeInTheDocument();
    // "Nutrition" appears both in the coach categories list and the
    // top-categories table, and "Supplement" only in the table.
    expect(screen.getAllByText(/Nutrition/).length).toBeGreaterThan(0);
    expect(screen.getByText('Supplement')).toBeInTheDocument();
  });

  it('renders the scanned-verdicts and flagged counters', async () => {
    vi.mocked(adminApi.getMythBustingSummary).mockResolvedValueOnce(sampleSummary());
    renderTab();
    await waitFor(() => {
      // "200" appears twice — in the header sentence and in the metric card.
      expect(screen.getAllByText('200').length).toBeGreaterThan(0);
    });
    expect(screen.getByText('12')).toBeInTheDocument();
  });
});
