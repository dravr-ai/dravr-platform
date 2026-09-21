// ABOUTME: Tests for the ClaimVerdictsTab admin triage surface
// ABOUTME: Mocks the admin API and asserts filters, the health card, the message lookup and the disposition write
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor, within } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import ClaimVerdictsTab from '../ClaimVerdictsTab';
import type { ClaimVerdict } from '@pierre/shared-types';
import type { VerdictDetailResponse, VerdictHealthStats } from '../../services/api/admin';

// Mock the admin API before component import time
vi.mock('../../services/api/admin', async () => {
  return {
    adminApi: {
      listClaimVerdicts: vi.fn(),
      getClaimVerdict: vi.fn(),
      setClaimVerdictDisposition: vi.fn(),
      listVerdictsForMessage: vi.fn(),
      getClaimVerdictHealth: vi.fn(),
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

function sampleVerdict(overrides: Partial<ClaimVerdict> = {}): ClaimVerdict {
  return {
    id: 'v1',
    tenant_id: 'tenant-a',
    user_id: 'user-1',
    agent_id: 'coach-1',
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
    disposition: null,
    disposition_reason: null,
    disposition_note: null,
    disposed_by: null,
    disposed_at: null,
    ...overrides,
  };
}

function sampleHealth(overrides: Partial<VerdictHealthStats> = {}): VerdictHealthStats {
  return {
    window_start: '2026-08-22T00:00:00Z',
    window_days: 30,
    totals: { flagged: 6, disposed: 4, true_catches: 1, false_positives: 3, unsure: 0 },
    false_positive_rate: 0.75,
    by_layer: [
      { layer: 'deterministic', flagged: 2, disposed: 1, false_positives: 1, rate: 1 },
      { layer: 'evidence', flagged: 4, disposed: 3, false_positives: 2, rate: 2 / 3 },
    ],
    by_category: [],
    by_agent: [],
    by_reason: [
      { reason: 'bound_too_tight', count: 1 },
      { reason: 'missing_keyword', count: 2 },
    ],
    daily: [],
    ...overrides,
  };
}

function sampleDetail(verdict: ClaimVerdict): VerdictDetailResponse {
  return {
    verdict,
    knob: {
      layer: verdict.layer_fired,
      kind: 'evidence_corpus',
      location: 'evidence/sports_science/supplement/',
      detail: '1 proposition(s) under evidence/sports_science/supplement/ keyword-match the claim.',
      propositions: [
        {
          id: 'issn:2017-creatine',
          category: 'supplement',
          slug: 'kreider-2017-creatine',
          path: 'evidence/sports_science/supplement/kreider-2017-creatine.md',
          strength: 'strong',
          score: 3,
          cited: true,
        },
      ],
    },
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
    vi.mocked(adminApi.getClaimVerdictHealth).mockResolvedValue(sampleHealth());
    vi.mocked(adminApi.getClaimVerdict).mockImplementation(async (id) =>
      sampleDetail(sampleVerdict({ id })),
    );
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

  // The admin table and the chat chip open the SAME drawer now — one component,
  // one `humanizeCategory`, one `formatTimestamp`. Its heading is the chat
  // one's, so an assertion on the deleted admin heading is what turns red if
  // the second drawer ever comes back.
  it('opens the shared verdict drawer with the knob when a row is clicked', async () => {
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
      expect(screen.getByText('About this claim')).toBeInTheDocument();
      expect(screen.getByText(/ISSN 2017 position stand/i)).toBeInTheDocument();
    });
    // The admin read carries provenance the chat read does not, and the one
    // drawer shows it only when it is there.
    expect(screen.getByText('Agent')).toBeInTheDocument();
    // The knob comes from the detail read, keyed by the row's id.
    await waitFor(() => {
      expect(adminApi.getClaimVerdict).toHaveBeenCalledWith('v1', 'tenant-a');
      expect(screen.getByText('evidence/sports_science/supplement/')).toBeInTheDocument();
    });
    expect(
      screen.getByText('evidence/sports_science/supplement/kreider-2017-creatine.md'),
    ).toBeInTheDocument();
    expect(screen.getByText('cited')).toBeInTheDocument();
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
    fireEvent.change(screen.getByLabelText('Status'), { target: { value: 'unsupported' } });
    await waitFor(() => {
      expect(adminApi.listClaimVerdicts).toHaveBeenLastCalledWith(
        expect.objectContaining({ status: 'unsupported' }),
      );
    });
  });

  it('passes the layer and disposition filters into the API call', async () => {
    vi.mocked(adminApi.listClaimVerdicts).mockResolvedValue({
      verdicts: [],
      total: 0,
    });
    renderTab();
    await waitFor(() => {
      expect(adminApi.listClaimVerdicts).toHaveBeenCalled();
    });
    fireEvent.change(screen.getByLabelText('Layer'), { target: { value: 'deterministic' } });
    await waitFor(() => {
      expect(adminApi.listClaimVerdicts).toHaveBeenLastCalledWith(
        expect.objectContaining({ layer_fired: 'deterministic' }),
      );
    });
    fireEvent.change(screen.getByLabelText('Disposition'), { target: { value: 'undisposed' } });
    await waitFor(() => {
      expect(adminApi.listClaimVerdicts).toHaveBeenLastCalledWith(
        expect.objectContaining({ layer_fired: 'deterministic', disposition: 'undisposed' }),
      );
    });
  });

  it('renders the health card from the aggregate read', async () => {
    vi.mocked(adminApi.listClaimVerdicts).mockResolvedValue({ verdicts: [], total: 0 });
    renderTab();
    await waitFor(() => {
      expect(adminApi.getClaimVerdictHealth).toHaveBeenCalledWith('tenant-a', 30);
      expect(screen.getByTestId('verdict-health-rate')).toHaveTextContent('75%');
    });
    const byLayer = within(screen.getByTestId('verdict-health-by-layer'));
    expect(byLayer.getByText('Deterministic')).toBeInTheDocument();
    expect(byLayer.getByText('Evidence')).toBeInTheDocument();
    expect(byLayer.getByText('67%')).toBeInTheDocument();
    const byReason = within(screen.getByTestId('verdict-health-by-reason'));
    expect(byReason.getByText('Missing Keyword')).toBeInTheDocument();
    expect(byReason.getByText('2')).toBeInTheDocument();
    // The window select re-reads with the chosen span.
    fireEvent.change(screen.getByLabelText('Window'), { target: { value: '90' } });
    await waitFor(() => {
      expect(adminApi.getClaimVerdictHealth).toHaveBeenLastCalledWith('tenant-a', 90);
    });
  });

  it('looks a message up and opens the drawer on its verdicts', async () => {
    vi.mocked(adminApi.listClaimVerdicts).mockResolvedValue({ verdicts: [], total: 0 });
    vi.mocked(adminApi.listVerdictsForMessage).mockResolvedValue({
      verdicts: [
        sampleVerdict({
          id: 'v9',
          message_id: 'msg-42',
          status: 'contradicted',
          claim_text: 'Your max heart rate is 250 bpm.',
          layer_fired: 'deterministic',
        }),
      ],
      total: 1,
    });
    renderTab();
    await waitFor(() => {
      expect(screen.getByText(/No claim verdicts matching/i)).toBeInTheDocument();
    });
    fireEvent.change(screen.getByLabelText('Message id'), { target: { value: ' msg-42 ' } });
    fireEvent.click(screen.getByRole('button', { name: 'Look up' }));
    await waitFor(() => {
      expect(adminApi.listVerdictsForMessage).toHaveBeenCalledWith('msg-42', 'tenant-a');
      expect(screen.getByText('Your max heart rate is 250 bpm.')).toBeInTheDocument();
    });
    expect(screen.getByTestId('verdict-drawer')).toBeInTheDocument();
    expect(screen.getByText('msg-42')).toBeInTheDocument();
  });

  it('says so when a looked-up message has no verdict', async () => {
    vi.mocked(adminApi.listClaimVerdicts).mockResolvedValue({ verdicts: [], total: 0 });
    vi.mocked(adminApi.listVerdictsForMessage).mockResolvedValue({ verdicts: [], total: 0 });
    renderTab();
    fireEvent.change(screen.getByLabelText('Message id'), { target: { value: 'msg-0' } });
    fireEvent.click(screen.getByRole('button', { name: 'Look up' }));
    await waitFor(() => {
      expect(screen.getByTestId('verdict-lookup-empty')).toHaveTextContent('msg-0');
    });
    expect(screen.queryByTestId('verdict-drawer')).not.toBeInTheDocument();
  });

  it('saves a disposition through the API with the body the route expects', async () => {
    vi.mocked(adminApi.listClaimVerdicts).mockResolvedValue({
      verdicts: [sampleVerdict({ id: 'v1', status: 'unsupported' })],
      total: 1,
    });
    vi.mocked(adminApi.setClaimVerdictDisposition).mockImplementation(async (id, body) =>
      sampleDetail(
        sampleVerdict({
          id,
          status: 'unsupported',
          disposition: body.disposition,
          disposition_reason: body.reason ?? null,
          disposition_note: body.note ?? null,
          disposed_by: 'admin@example.com',
          disposed_at: '2026-09-21T12:00:00Z',
        }),
      ),
    );
    renderTab();
    await waitFor(() => {
      expect(screen.getByText(/Take 5 g of creatine/i)).toBeInTheDocument();
    });
    fireEvent.click(screen.getByText(/Take 5 g of creatine/i));
    await waitFor(() => {
      expect(screen.getByTestId('verdict-disposition')).toBeInTheDocument();
    });

    const saveButton = screen.getByRole('button', { name: 'Save disposition' });
    // Nothing chosen yet: there is nothing to write.
    expect(saveButton).toBeDisabled();

    fireEvent.change(screen.getByLabelText('Disposition call'), {
      target: { value: 'false_positive' },
    });
    fireEvent.change(screen.getByLabelText('Disposition reason'), {
      target: { value: 'missing_keyword' },
    });
    fireEvent.change(screen.getByLabelText('Disposition note'), {
      target: { value: '  corpus has it under a different word  ' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Save disposition' }));

    await waitFor(() => {
      expect(adminApi.setClaimVerdictDisposition).toHaveBeenCalledWith('v1', {
        tenant_id: 'tenant-a',
        disposition: 'false_positive',
        reason: 'missing_keyword',
        note: 'corpus has it under a different word',
      });
    });
    // The drawer re-renders the row the write returned.
    await waitFor(() => {
      expect(screen.getByText(/Disposed by admin@example.com/)).toBeInTheDocument();
    });
    // And the list is re-read so the table's disposition column follows.
    await waitFor(() => {
      expect(vi.mocked(adminApi.listClaimVerdicts).mock.calls.length).toBeGreaterThan(1);
    });
  });
});
