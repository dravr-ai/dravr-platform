// ABOUTME: The drafts the agent proposed: quoted evidence, Keep approves, Discard forgets
// ABOUTME: A draft a newer fact superseded says so and can no longer be kept
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import MemoryReviewSection from '../MemoryReviewSection';
import type { MemoryFactRow } from '@pierre/api-client';

vi.mock('../../../services/api', async () => ({
  userApi: {
    listMemoryFacts: vi.fn(),
    forgetMemoryFact: vi.fn(),
    approveMemoryFact: vi.fn(),
  },
}));

const { userApi } = await import('../../../services/api');

function draft(overrides: Partial<MemoryFactRow> = {}): MemoryFactRow {
  return {
    id: 'draft-1',
    agent_id: null,
    agent_title: null,
    kind: 'physiology',
    predicate_code: 'states',
    object: 'FTP around 250 W',
    sentence: 'Your FTP is around 250 W',
    confidence: 0.7,
    source_msg_id: null,
    updated_at: '2026-10-19T18:00:00Z',
    status: 'draft',
    evidence: 'I held 250 for the whole 20-min test',
    evidence_source: 'athlete',
    ...overrides,
  };
}

function renderSection() {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={queryClient}>
      <MemoryReviewSection />
    </QueryClientProvider>,
  );
}

describe('MemoryReviewSection', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('asks only for drafts and renders nothing when there are none', async () => {
    vi.mocked(userApi.listMemoryFacts).mockResolvedValue({ facts: [], total: 0 });
    const { container } = renderSection();
    await waitFor(() => {
      expect(userApi.listMemoryFacts).toHaveBeenCalledWith(expect.objectContaining({ status: 'draft' }));
    });
    expect(container).toBeEmptyDOMElement();
  });

  it('quotes the words each draft came from', async () => {
    vi.mocked(userApi.listMemoryFacts).mockResolvedValue({ facts: [draft()], total: 1 });
    renderSection();
    expect(await screen.findByText('Your FTP is around 250 W')).toBeInTheDocument();
    expect(screen.getByTestId('memory-review-evidence')).toHaveTextContent('I held 250 for the whole 20-min test');
  });

  it('keeps a draft by approving it', async () => {
    vi.mocked(userApi.listMemoryFacts).mockResolvedValue({ facts: [draft()], total: 1 });
    vi.mocked(userApi.approveMemoryFact).mockResolvedValueOnce({ fact: draft({ status: 'approved' }) });
    renderSection();
    fireEvent.click(await screen.findByTestId('memory-review-keep'));
    await waitFor(() => expect(userApi.approveMemoryFact).toHaveBeenCalledWith('draft-1'));
  });

  it('discards a draft by forgetting it', async () => {
    vi.mocked(userApi.listMemoryFacts).mockResolvedValue({ facts: [draft()], total: 1 });
    vi.mocked(userApi.forgetMemoryFact).mockResolvedValueOnce({ deleted: true });
    renderSection();
    fireEvent.click(await screen.findByTestId('memory-review-discard'));
    await waitFor(() => expect(userApi.forgetMemoryFact).toHaveBeenCalledWith('draft-1'));
  });

  it('a superseded draft says so and can no longer be kept', async () => {
    vi.mocked(userApi.listMemoryFacts).mockResolvedValue({ facts: [draft()], total: 1 });
    vi.mocked(userApi.approveMemoryFact).mockRejectedValueOnce({ response: { status: 409 } });
    renderSection();
    fireEvent.click(await screen.findByTestId('memory-review-keep'));
    expect(await screen.findByRole('alert')).toBeInTheDocument();
    expect(screen.getByTestId('memory-review-keep')).toBeDisabled();
  });
});
