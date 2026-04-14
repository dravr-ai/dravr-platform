// ABOUTME: Sprint C12 tests — mobile MemoryScreen renders facts and triggers forget via Alert
// ABOUTME: Mocks userApi.listMemoryFacts/forgetMemoryFact and asserts list + confirm flow
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React from 'react';
import { Alert } from 'react-native';
import { render, fireEvent, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';

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
  kind: string;
  subject: string;
  predicate: string;
  object: string;
  confidence: number;
  source_msg_id: string | null;
  updated_at: string;
};

function createFact(overrides: Partial<Fact> = {}): Fact {
  return {
    id: 'fact-1',
    coach_id: null,
    kind: 'goal',
    subject: 'You',
    predicate: 'targeting',
    object: 'sub-3:30 marathon by October',
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

  it('renders facts grouped by kind', async () => {
    mockListMemoryFacts.mockResolvedValueOnce({
      facts: [
        createFact(),
        createFact({
          id: 'fact-2',
          kind: 'injury',
          subject: 'You',
          predicate: 'have',
          object: 'left achilles tendinitis',
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
    expect(getAllByText('Goals').length).toBeGreaterThan(0);
    expect(getAllByText('Injuries').length).toBeGreaterThan(0);
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
    await waitFor(() => {
      expect(getByLabelText(/Forget You targeting/i)).toBeTruthy();
    });
    fireEvent.press(getByLabelText(/Forget You targeting/i));
    await waitFor(() => {
      expect(mockForgetMemoryFact).toHaveBeenCalledWith('fact-1');
    });

    alertSpy.mockRestore();
  });
});
