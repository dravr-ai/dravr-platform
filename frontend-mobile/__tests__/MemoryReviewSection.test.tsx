// ABOUTME: Tests the drafts the agent proposed on mobile: quoted evidence, Keep approves, Discard forgets
// ABOUTME: A draft a newer fact superseded says so and can no longer be kept
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React from 'react';
import { render, fireEvent, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';

const mockListMemoryFacts = jest.fn();
const mockForgetMemoryFact = jest.fn();
const mockApproveMemoryFact = jest.fn();

jest.mock('../src/services/api', () => ({
  userApi: {
    listMemoryFacts: (...args: unknown[]) => mockListMemoryFacts(...args),
    forgetMemoryFact: (...args: unknown[]) => mockForgetMemoryFact(...args),
    approveMemoryFact: (...args: unknown[]) => mockApproveMemoryFact(...args),
  },
}));

import { MemoryReviewSection } from '../src/screens/memory/MemoryReviewSection';

const DRAFT = {
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
};

function renderSection(): ReturnType<typeof render> {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: Infinity }, mutations: { gcTime: Infinity } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <MemoryReviewSection />
    </QueryClientProvider>,
  );
}

describe('MemoryReviewSection', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it('asks only for drafts and renders nothing when there are none', async () => {
    mockListMemoryFacts.mockResolvedValue({ facts: [], total: 0 });
    const { queryByTestId } = renderSection();
    await waitFor(() => {
      expect(mockListMemoryFacts).toHaveBeenCalledWith(expect.objectContaining({ status: 'draft' }));
    });
    expect(queryByTestId('memory-review')).toBeNull();
  });

  it('quotes the words a draft came from', async () => {
    mockListMemoryFacts.mockResolvedValue({ facts: [DRAFT], total: 1 });
    const { findByText, getByTestId } = renderSection();
    expect(await findByText('Your FTP is around 250 W')).toBeTruthy();
    expect(getByTestId('memory-review-draft-1-evidence')).toHaveTextContent(/I held 250 for the whole 20-min test/);
  });

  it('keeps a draft by approving it and discards one by forgetting it', async () => {
    mockListMemoryFacts.mockResolvedValue({ facts: [DRAFT], total: 1 });
    mockApproveMemoryFact.mockResolvedValueOnce({ fact: { ...DRAFT, status: 'approved' } });
    mockForgetMemoryFact.mockResolvedValueOnce({ deleted: true });
    const { findByTestId, getByTestId } = renderSection();
    fireEvent.press(await findByTestId('memory-review-draft-1-keep'));
    await waitFor(() => expect(mockApproveMemoryFact).toHaveBeenCalledWith('draft-1'));
    await waitFor(() => expect(getByTestId('memory-review-draft-1-discard')).not.toBeDisabled());
    fireEvent.press(getByTestId('memory-review-draft-1-discard'));
    await waitFor(() => expect(mockForgetMemoryFact).toHaveBeenCalledWith('draft-1'));
  });

  it('a superseded draft says so and can no longer be kept', async () => {
    mockListMemoryFacts.mockResolvedValue({ facts: [DRAFT], total: 1 });
    mockApproveMemoryFact.mockRejectedValueOnce({ response: { status: 409 } });
    const { findByTestId, findByRole } = renderSection();
    fireEvent.press(await findByTestId('memory-review-draft-1-keep'));
    expect(await findByRole('alert')).toBeTruthy();
    expect(await findByTestId('memory-review-draft-1-keep')).toBeDisabled();
  });
});
