// ABOUTME: Sprint C5 tests — MemoryPanel renders user_facts and forgets individual rows
// ABOUTME: Mocks userApi.listMemoryFacts/forgetMemoryFact and asserts list/forget UX
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor, within } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { MEMORY_FACT_KINDS } from '@pierre/shared-constants';
import MemoryPanel from '../MemoryPanel';
import type { MemoryFactRow } from '@pierre/api-client';

vi.mock('../../../services/api', async () => ({
  userApi: {
    listMemoryFacts: vi.fn(),
    forgetMemoryFact: vi.fn(),
  },
}));

const { userApi } = await import('../../../services/api');

function sampleFact(overrides: Partial<MemoryFactRow> = {}): MemoryFactRow {
  return {
    id: 'fact-1',
    coach_id: null,
    coach_title: null,
    kind: 'goal',
    predicate_code: 'working_toward',
    object: 'sub-3:30 marathon by October',
    sentence: 'You are working toward sub-3:30 marathon by October',
    confidence: 0.85,
    source_msg_id: null,
    updated_at: '2026-04-13T18:00:00Z',
    ...overrides,
  };
}

function renderPanel() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <MemoryPanel />
    </QueryClientProvider>,
  );
}

describe('MemoryPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders the empty state when no facts exist', async () => {
    vi.mocked(userApi.listMemoryFacts).mockResolvedValueOnce({
      facts: [],
      total: 0,
    });
    renderPanel();
    await waitFor(() => {
      expect(screen.getByText(/No facts stored yet/i)).toBeInTheDocument();
    });
  });

  it('names the coach a fact belongs to by title, never by id', async () => {
    vi.mocked(userApi.listMemoryFacts).mockResolvedValueOnce({
      facts: [
        sampleFact({
          coach_id: '7c1f7d2e-4b0a-4f0e-9d3a-0f6c2b8e9a11',
          coach_title: 'Coach Marie',
        }),
      ],
      total: 1,
    });
    renderPanel();
    expect(await screen.findByText(/Coach Marie/)).toBeInTheDocument();
    expect(screen.queryByText(/7c1f7d2e/)).not.toBeInTheDocument();
  });

  it('renders facts grouped by kind', async () => {
    vi.mocked(userApi.listMemoryFacts).mockResolvedValueOnce({
      facts: [
        sampleFact(),
        sampleFact({
          id: 'fact-2',
          kind: 'injury',
          predicate_code: 'have',
          object: 'left achilles tendinitis',
          sentence: 'You have left achilles tendinitis',
        }),
      ],
      total: 2,
    });
    renderPanel();
    await waitFor(() => {
      expect(screen.getByText(/sub-3:30 marathon/i)).toBeInTheDocument();
    });
    expect(screen.getByText(/left achilles tendinitis/i)).toBeInTheDocument();
    // Both group headers should render — the Badge is inside an article/region.
    const goalBadges = screen.getAllByText(/^Goal$/);
    expect(goalBadges.length).toBeGreaterThan(0);
    const injuryBadges = screen.getAllByText(/^Injury$/);
    expect(injuryBadges.length).toBeGreaterThan(0);
  });

  it('opens the confirm dialog when Forget is clicked', async () => {
    vi.mocked(userApi.listMemoryFacts).mockResolvedValueOnce({
      facts: [sampleFact()],
      total: 1,
    });
    renderPanel();
    await waitFor(() => {
      expect(screen.getByText(/sub-3:30 marathon/i)).toBeInTheDocument();
    });
    fireEvent.click(screen.getByRole('button', { name: /Forget/i }));
    expect(await screen.findByText(/Forget this fact/i)).toBeInTheDocument();
  });

  it('calls forgetMemoryFact when the dialog is confirmed', async () => {
    vi.mocked(userApi.listMemoryFacts).mockResolvedValue({
      facts: [sampleFact()],
      total: 1,
    });
    vi.mocked(userApi.forgetMemoryFact).mockResolvedValueOnce({ deleted: true });
    renderPanel();
    await waitFor(() => {
      expect(screen.getByText(/sub-3:30 marathon/i)).toBeInTheDocument();
    });
    fireEvent.click(screen.getByRole('button', { name: /Forget/i }));
    const confirmButtons = await screen.findAllByRole('button', { name: /Forget/i });
    // The dialog adds a second "Forget" button; the second one is the confirm.
    fireEvent.click(confirmButtons[confirmButtons.length - 1]);
    await waitFor(() => {
      expect(userApi.forgetMemoryFact).toHaveBeenCalledWith('fact-1');
    });
  });

  it('shows the sentence the server rendered, verbatim and in the athlete\'s language', async () => {
    // The sentence is rendered on the server in the athlete\'s locale; the
    // panel must not glue an English verb to it or reorder it. A French
    // athlete\'s goal reads as French here even while the chrome is English.
    vi.mocked(userApi.listMemoryFacts).mockResolvedValueOnce({
      facts: [
        sampleFact({
          id: 'fact-fr',
          kind: 'goal',
          predicate_code: 'training_for',
          object: 'un ultra de 26 km au Mont Albert',
          sentence: "Tu t'entraînes pour un ultra de 26 km au Mont Albert",
        }),
      ],
      total: 1,
    });
    renderPanel();
    await waitFor(() => {
      expect(screen.getByText("Tu t'entraînes pour un ultra de 26 km au Mont Albert")).toBeInTheDocument();
    });
    expect(screen.queryByText(/training_for/)).not.toBeInTheDocument();
    expect(screen.queryByText(/You are/)).not.toBeInTheDocument();
  });

  it('names the fact by its sentence in the forget confirmation', async () => {
    vi.mocked(userApi.listMemoryFacts).mockResolvedValueOnce({
      facts: [sampleFact({ sentence: 'You are working toward sub-3:30 marathon by October' })],
      total: 1,
    });
    renderPanel();
    await waitFor(() => {
      expect(screen.getByText(/sub-3:30 marathon/i)).toBeInTheDocument();
    });
    fireEvent.click(screen.getByRole('button', { name: /Forget/i }));
    // The sentence now appears twice: the list row and the confirmation.
    const mentions = await screen.findAllByText(/You are working toward sub-3:30 marathon by October/);
    expect(mentions).toHaveLength(2);
  });

  it('passes the kind filter into the API call', async () => {
    vi.mocked(userApi.listMemoryFacts).mockResolvedValue({
      facts: [],
      total: 0,
    });
    renderPanel();
    await waitFor(() => {
      expect(userApi.listMemoryFacts).toHaveBeenCalled();
    });
    fireEvent.click(screen.getByTestId('memory-kind-chip-injury'));
    await waitFor(() => {
      expect(userApi.listMemoryFacts).toHaveBeenLastCalledWith(
        expect.objectContaining({ kind: 'injury' }),
      );
    });
  });

  // The filter was a browser `select` with its own focus ring beside cards that
  // follow the design system, answering the same question the phone answers
  // with chips. One control, both clients.
  it('filters with chips, not a native select', async () => {
    vi.mocked(userApi.listMemoryFacts).mockResolvedValue({ facts: [], total: 0 });
    renderPanel();
    await waitFor(() => expect(screen.getByTestId('memory-kind-filter')).toBeInTheDocument());

    expect(screen.queryByRole('combobox')).not.toBeInTheDocument();
    // One chip per kind the server can send, plus "All kinds".
    const chips = within(screen.getByTestId('memory-kind-filter')).getAllByRole('button');
    expect(chips).toHaveLength(MEMORY_FACT_KINDS.length + 1);
    // The selected chip says so to a screen reader, which a styled div cannot.
    expect(screen.getByTestId('memory-kind-chip-all')).toHaveAttribute('aria-pressed', 'true');
    expect(screen.getByTestId('memory-kind-chip-injury')).toHaveAttribute('aria-pressed', 'false');
  });

  // `facts` is the FILTERED list, so the unfiltered empty card told an athlete
  // who has memory that they have none, and invited them to go earn some.
  it('says nothing of this type — not nothing at all — when a filter is applied', async () => {
    vi.mocked(userApi.listMemoryFacts).mockResolvedValue({ facts: [], total: 0 });
    renderPanel();
    await waitFor(() => expect(screen.getByTestId('memory-empty')).toBeInTheDocument());
    const neverHadAny = screen.getByTestId('memory-empty').textContent ?? '';

    fireEvent.click(screen.getByTestId('memory-kind-chip-injury'));

    await waitFor(() => expect(screen.getByTestId('memory-empty-filtered')).toBeInTheDocument());
    expect(screen.queryByTestId('memory-empty')).not.toBeInTheDocument();
    const filtered = screen.getByTestId('memory-empty-filtered').textContent ?? '';
    expect(filtered).not.toEqual(neverHadAny);
    expect(filtered).not.toContain('No facts stored yet');
  });

  it('offers the way back to all types from the filtered empty state', async () => {
    vi.mocked(userApi.listMemoryFacts).mockResolvedValue({ facts: [], total: 0 });
    renderPanel();
    await waitFor(() => expect(screen.getByTestId('memory-empty')).toBeInTheDocument());

    fireEvent.click(screen.getByTestId('memory-kind-chip-injury'));
    await waitFor(() => expect(screen.getByTestId('memory-show-all-kinds')).toBeInTheDocument());

    fireEvent.click(screen.getByTestId('memory-show-all-kinds'));

    await waitFor(() => expect(screen.getByTestId('memory-empty')).toBeInTheDocument());
    expect(userApi.listMemoryFacts).toHaveBeenLastCalledWith(
      expect.objectContaining({ kind: undefined }),
    );
  });
});
