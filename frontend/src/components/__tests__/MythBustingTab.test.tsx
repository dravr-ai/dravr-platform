// ABOUTME: Sprint C13 tests — MythBustingTab renders aggregates and empty state
// ABOUTME: Mocks adminApi.getMythBustingSummary and asserts pattern lists render correctly
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import MythBustingTab from '../MythBustingTab';
import { ADMIN_LOCALE } from '../../utils/adminLocale';
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
    scope: { kind: 'tenant', tenant_id: 'tenant-a' },
    verdicts_scanned: 200,
    window_start: '2026-03-01T12:00:00Z',
    window_end: '2026-04-13T18:00:00Z',
    flagged_total: 12,
    distinct_claims: 37,
    recent_claims: [
      {
        claim_excerpt: 'Drink a gallon of water per day',
        status: 'contradicted',
        layer_fired: 'judge',
        category: 'nutrition',
        occurrences: 4,
        agent_count: 2,
        first_seen_at: '2026-03-01T18:00:00Z',
        last_seen_at: '2026-04-13T18:00:00Z',
      },
      {
        claim_excerpt: 'Cardio kills your gains',
        status: 'unsupported',
        layer_fired: 'evidence',
        category: 'physiological',
        occurrences: 1,
        agent_count: 0,
        first_seen_at: '2026-04-12T18:00:00Z',
        last_seen_at: '2026-04-12T18:00:00Z',
      },
    ],
    top_agents: [
      {
        agent_id: 'coach-broscience',
        agent_name: 'Bro Science',
        unsupported_total: 5,
        categories: ['nutrition', 'training_prescription'],
      },
    ],
    top_categories: [
      { category: 'nutrition', flagged_total: 6, agent_count: 3 },
      { category: 'supplement', flagged_total: 3, agent_count: 1 },
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

  it('renders the empty state when no claim drew a warning', async () => {
    vi.mocked(adminApi.getMythBustingSummary).mockResolvedValueOnce(
      sampleSummary({
        flagged_total: 0,
        distinct_claims: 0,
        recent_claims: [],
        top_agents: [],
        top_categories: [],
      }),
    );
    renderTab();
    await waitFor(() => {
      expect(
        screen.getByText(/No claims athletes were warned about/i),
      ).toBeInTheDocument();
    });
  });

  it('renders recent warned claims, agents, and categories', async () => {
    vi.mocked(adminApi.getMythBustingSummary).mockResolvedValueOnce(sampleSummary());
    renderTab();
    await waitFor(() => {
      expect(screen.getByText(/Drink a gallon of water per day/)).toBeInTheDocument();
    });
    expect(screen.getByText(/Cardio kills your gains/)).toBeInTheDocument();
    expect(screen.getByText('coach-broscience')).toBeInTheDocument();
    expect(screen.getByText('Bro Science')).toBeInTheDocument();
    // "Nutrition" appears both in the coach categories list and the
    // top-categories table, and "Supplement" only in the table.
    expect(screen.getAllByText(/Nutrition/).length).toBeGreaterThan(0);
    expect(screen.getByText('Supplement')).toBeInTheDocument();
  });

  it('offers no way to block a topic from a recurring claim', async () => {
    vi.mocked(adminApi.getMythBustingSummary).mockResolvedValueOnce(sampleSummary());
    renderTab();
    await waitFor(() => {
      expect(screen.getByText(/Drink a gallon of water per day/)).toBeInTheDocument();
    });
    expect(screen.queryByRole('button', { name: /block/i })).toBeNull();
  });

  it('says warned, the set it counts, and never flagged', async () => {
    vi.mocked(adminApi.getMythBustingSummary).mockResolvedValueOnce(sampleSummary());
    const { container } = renderTab();
    await screen.findByText('Warned claims');
    expect(container.textContent).not.toMatch(/flagged/i);
  });

  it('renders the scanned-verdicts and warned counters', async () => {
    vi.mocked(adminApi.getMythBustingSummary).mockResolvedValueOnce(sampleSummary());
    renderTab();
    await waitFor(() => {
      // "200" appears twice — in the header sentence and in the metric card.
      expect(screen.getAllByText('200').length).toBeGreaterThan(0);
    });
    expect(screen.getByText('12')).toBeInTheDocument();
  });

  it('states the capped list as capped, using the real distinct count', async () => {
    vi.mocked(adminApi.getMythBustingSummary).mockResolvedValueOnce(sampleSummary());
    renderTab();
    await waitFor(() => {
      expect(screen.getByTestId('recent-claims-cap')).toHaveTextContent(
        'Showing 2 of 37 distinct claims',
      );
    });
    expect(screen.getByText('37')).toBeInTheDocument();
  });

  it('dates the window and each claim, and never promises recurrence', async () => {
    vi.mocked(adminApi.getMythBustingSummary).mockResolvedValueOnce(sampleSummary());
    renderTab();
    await waitFor(() => {
      expect(screen.getByTestId('window-start')).toHaveTextContent(
        new Date('2026-03-01T12:00:00Z').toLocaleDateString(ADMIN_LOCALE),
      );
    });
    expect(screen.getByText(/first seen/)).toBeInTheDocument();
    expect(screen.getByText(/no agent on record/)).toBeInTheDocument();
    expect(screen.queryByText(/top recurring/i)).not.toBeInTheDocument();
  });

  it('reads the same verdict window as Agent Grades', async () => {
    vi.mocked(adminApi.getMythBustingSummary).mockResolvedValueOnce(sampleSummary());
    renderTab();
    await waitFor(() => {
      expect(adminApi.getMythBustingSummary).toHaveBeenCalledWith(undefined, 500);
    });
  });
});
