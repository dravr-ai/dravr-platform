// ABOUTME: Tests for the Tier 5.5 ClaimVerdictsTab admin triage surface
// ABOUTME: Mocks the admin API and asserts filter + drawer behavior
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import ClaimVerdictsTab from '../ClaimVerdictsTab';
import type { ClaimVerdictRow } from '../../services/api/admin';

// Mock the admin API before component import time
vi.mock('../../services/api/admin', async () => {
  return {
    adminApi: {
      listClaimVerdicts: vi.fn(),
    },
  };
});

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

function sampleVerdict(overrides: Partial<ClaimVerdictRow> = {}): ClaimVerdictRow {
  return {
    id: 'v1',
    tenant_id: 'tenant-a',
    user_id: 'user-1',
    coach_id: 'coach-1',
    conversation_id: 'conv-1',
    message_id: null,
    claim_text: 'Take 5 g of creatine per day for high-intensity performance.',
    category: 'supplement',
    status: 'supported',
    evidence_strength: 'strong',
    confidence: 0.8,
    layer_fired: 'evidence',
    explanation: 'Backed by ISSN 2017 position stand on creatine.',
    evidence_refs: 'issn:2017-creatine',
    created_at: '2026-04-13T18:00:00Z',
    ...overrides,
  };
}

function renderTab() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <ClaimVerdictsTab />
    </QueryClientProvider>,
  );
}

describe('ClaimVerdictsTab', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders the empty state when no verdicts are returned', async () => {
    vi.mocked(adminApi.listClaimVerdicts).mockResolvedValueOnce({
      verdicts: [],
      total: 0,
    });
    renderTab();
    await waitFor(() => {
      expect(screen.getByText(/No claim verdicts matching/i)).toBeInTheDocument();
    });
  });

  it('lists verdicts returned from the API', async () => {
    vi.mocked(adminApi.listClaimVerdicts).mockResolvedValueOnce({
      verdicts: [
        sampleVerdict(),
        sampleVerdict({ id: 'v2', status: 'unsupported', claim_text: 'Unsupported claim' }),
      ],
      total: 2,
    });
    renderTab();
    await waitFor(() => {
      expect(screen.getByText(/Take 5 g of creatine/i)).toBeInTheDocument();
    });
    expect(screen.getByText('Unsupported claim')).toBeInTheDocument();
    expect(screen.getByText(/Showing 2 verdicts/)).toBeInTheDocument();
  });

  it('opens the drawer when a row is clicked', async () => {
    vi.mocked(adminApi.listClaimVerdicts).mockResolvedValueOnce({
      verdicts: [sampleVerdict()],
      total: 1,
    });
    renderTab();
    await waitFor(() => {
      expect(screen.getByText(/Take 5 g of creatine/i)).toBeInTheDocument();
    });
    fireEvent.click(screen.getByText(/Take 5 g of creatine/i));
    await waitFor(() => {
      expect(screen.getByText(/Verdict detail/i)).toBeInTheDocument();
      expect(screen.getByText(/ISSN 2017 position stand/i)).toBeInTheDocument();
    });
  });

  it('passes status filter into the API call', async () => {
    vi.mocked(adminApi.listClaimVerdicts).mockResolvedValue({
      verdicts: [],
      total: 0,
    });
    renderTab();
    await waitFor(() => {
      expect(adminApi.listClaimVerdicts).toHaveBeenCalled();
    });
    const select = screen.getAllByRole('combobox')[0];
    fireEvent.change(select, { target: { value: 'unsupported' } });
    await waitFor(() => {
      expect(adminApi.listClaimVerdicts).toHaveBeenLastCalledWith(
        expect.objectContaining({ status: 'unsupported' }),
      );
    });
  });
});
