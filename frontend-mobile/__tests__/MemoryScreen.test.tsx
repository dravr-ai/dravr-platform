// ABOUTME: Sprint C12 tests — mobile MemoryScreen renders facts and triggers forget via Alert
// ABOUTME: Mocks userApi.listMemoryFacts/forgetMemoryFact and asserts list + confirm flow
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React from 'react';
import { Alert, ScrollView } from 'react-native';
import { render, fireEvent, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { i18n } from '@pierre/i18n';

const mockListMemoryFacts = jest.fn();
const mockForgetMemoryFact = jest.fn();

jest.mock('../src/services/api', () => ({
  userApi: {
    listMemoryFacts: (...args: unknown[]) => mockListMemoryFacts(...args),
    forgetMemoryFact: (...args: unknown[]) => mockForgetMemoryFact(...args),
  },
}));

import { MemoryScreen } from '../src/screens/memory/MemoryScreen';

type Fact = {
  id: string;
  coach_id: string | null;
  coach_title: string | null;
  kind: string;
  predicate_code: string;
  object: string;
  sentence: string;
  confidence: number;
  source_msg_id: string | null;
  updated_at: string;
};

function createFact(overrides: Partial<Fact> = {}): Fact {
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

function renderScreen(): ReturnType<typeof render> {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <MemoryScreen />
    </QueryClientProvider>,
  );
}

describe('MemoryScreen', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it('renders the empty state when no facts exist', async () => {
    mockListMemoryFacts.mockResolvedValueOnce({ facts: [], total: 0 });
    const { getByText } = renderScreen();
    await waitFor(() => {
      expect(getByText(/No facts stored yet/i)).toBeTruthy();
    });
  });

  it('names the coach a fact belongs to by title, never by id', async () => {
    mockListMemoryFacts.mockResolvedValueOnce({
      facts: [
        createFact({
          coach_id: '7c1f7d2e-4b0a-4f0e-9d3a-0f6c2b8e9a11',
          coach_title: 'Coach Marie',
        }),
      ],
      total: 1,
    });
    const { findByText, queryByText } = renderScreen();
    expect(await findByText(/Coach Marie/)).toBeTruthy();
    expect(queryByText(/7c1f7d2e/)).toBeNull();
  });

  it('renders facts grouped by kind', async () => {
    mockListMemoryFacts.mockResolvedValueOnce({
      facts: [
        createFact(),
        createFact({
          id: 'fact-2',
          kind: 'injury',
          predicate_code: 'have',
          object: 'left achilles tendinitis',
          sentence: 'You have left achilles tendinitis',
        }),
      ],
      total: 2,
    });
    const { getByText, getAllByText } = renderScreen();
    await waitFor(() => {
      expect(getByText(/sub-3:30 marathon/i)).toBeTruthy();
    });
    expect(getByText(/left achilles tendinitis/i)).toBeTruthy();
    // "Goals" and "Injuries" appear twice each — once as a filter chip and
    // once as a section header. Assert at least one match for both.
    expect(getAllByText('Goal').length).toBeGreaterThan(0);
    expect(getAllByText('Injury').length).toBeGreaterThan(0);
  });

  it('shows the sentence the server rendered, verbatim and in the athlete\'s language', async () => {
    // The sentence is rendered on the server in the athlete\'s locale; the
    // screen shows it as-is and carries no grammar of its own, so a French
    // athlete\'s goal reads as French even under English chrome.
    mockListMemoryFacts.mockResolvedValueOnce({
      facts: [
        createFact({
          id: 'fact-fr',
          kind: 'goal',
          predicate_code: 'training_for',
          object: 'un ultra de 26 km au Mont Albert',
          sentence: "Tu t'entraînes pour un ultra de 26 km au Mont Albert",
        }),
      ],
      total: 1,
    });
    const { queryByText, getByText } = renderScreen();
    await waitFor(() => {
      expect(getByText("Tu t'entraînes pour un ultra de 26 km au Mont Albert")).toBeTruthy();
    });
    expect(queryByText(/training_for/)).toBeNull();
    expect(queryByText(/You are/)).toBeNull();
  });

  // The query is filtered server-side, so an empty result under a chip is "none
  // of this type". The unfiltered card told an athlete who has memory that they
  // have none, which is the same lie the web panel told.
  it('says nothing of this type — not nothing at all — when a chip is selected', async () => {
    mockListMemoryFacts.mockResolvedValue({ facts: [], total: 0 });
    const { getByTestId, queryByTestId, getByText } = renderScreen();
    await waitFor(() => expect(getByTestId('memory-empty')).toBeTruthy());
    const neverHadAny = i18n.t('shell.memoryEmpty');
    expect(getByText(neverHadAny)).toBeTruthy();

    fireEvent.press(getByText(i18n.t('shell.memoryKindInjury')));

    await waitFor(() => expect(getByTestId('memory-empty-filtered')).toBeTruthy());
    expect(queryByTestId('memory-empty')).toBeNull();
    const filtered = i18n.t('shell.memoryEmptyFiltered');
    expect(filtered).not.toEqual(neverHadAny);
    expect(getByText(filtered)).toBeTruthy();
  });

  it('offers the way back to all types from the filtered empty state', async () => {
    mockListMemoryFacts.mockResolvedValue({ facts: [], total: 0 });
    const { getByTestId, getByText } = renderScreen();
    await waitFor(() => expect(getByTestId('memory-empty')).toBeTruthy());

    fireEvent.press(getByText(i18n.t('shell.memoryKindInjury')));
    await waitFor(() => expect(getByTestId('memory-show-all-kinds')).toBeTruthy());

    fireEvent.press(getByTestId('memory-show-all-kinds'));

    await waitFor(() => expect(getByTestId('memory-empty')).toBeTruthy());
    expect(mockListMemoryFacts).toHaveBeenLastCalledWith(
      expect.objectContaining({ kind: undefined }),
    );
  });

  // The chip row scrolls and clips "Physiologie" mid-word at the right edge. A
  // cut with nothing over it reads as a rendering fault rather than as more to
  // reach, so the fade appears exactly while there is more.
  it('fades the chip row while chips remain off the right edge, and not after', async () => {
    mockListMemoryFacts.mockResolvedValue({ facts: [], total: 0 });
    const { getByTestId, queryByTestId, UNSAFE_getAllByType } = renderScreen();
    await waitFor(() => expect(getByTestId('memory-empty')).toBeTruthy());

    // Nothing measured yet, so nothing is known to be off-screen.
    expect(queryByTestId('memory-kind-scroll-fade')).toBeNull();

    const chipRow = UNSAFE_getAllByType(ScrollView).find(
      (node) => node.props.horizontal === true,
    );
    expect(chipRow).toBeTruthy();

    fireEvent(chipRow!, 'layout', { nativeEvent: { layout: { width: 320, height: 40 } } });
    fireEvent(chipRow!, 'contentSizeChange', 900, 40);
    await waitFor(() => expect(getByTestId('memory-kind-scroll-fade')).toBeTruthy());

    // Scrolled to the far end: the last chip is fully in view, so the fade goes.
    fireEvent.scroll(chipRow!, { nativeEvent: { contentOffset: { x: 580, y: 0 } } });
    await waitFor(() => expect(queryByTestId('memory-kind-scroll-fade')).toBeNull());
  });

  it('fires forget mutation when Alert confirm is tapped', async () => {
    mockListMemoryFacts.mockResolvedValue({
      facts: [createFact()],
      total: 1,
    });
    mockForgetMemoryFact.mockResolvedValueOnce({ deleted: true });

    // Intercept Alert.alert and invoke the destructive button's onPress.
    const alertSpy = jest
      .spyOn(Alert, 'alert')
      .mockImplementation((_title, _msg, buttons) => {
        const list = buttons ?? [];
        const confirm = list.find((b) => b.style === 'destructive');
        confirm?.onPress?.();
      });

    const { getByLabelText } = renderScreen();
    // The accessibility label names the fact by the server's sentence.
    await waitFor(() => {
      expect(getByLabelText(/Forget You are working toward sub-3:30 marathon/i)).toBeTruthy();
    });
    fireEvent.press(getByLabelText(/Forget You are working toward sub-3:30 marathon/i));
    await waitFor(() => {
      expect(mockForgetMemoryFact).toHaveBeenCalledWith('fact-1');
    });

    alertSpy.mockRestore();
  });
});
