// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests Agent info — the handle it teaches, the /agent remove it sends, and who may edit an agent
// ABOUTME: Detaching is a command, not a private client call, so the app cannot reach a state the command cannot

import React from 'react';
import { render, waitFor } from '@testing-library/react-native';
import { fireEvent } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { COMMAND_DRAFTS } from '@pierre/shared-constants';
import type { Coach } from '@pierre/shared-types';

const mockPush = jest.fn();
jest.mock('expo-router', () => ({
  useRouter: () => ({ push: mockPush, replace: jest.fn(), back: jest.fn(), navigate: jest.fn() }),
}));

const mockListCoaches = jest.fn();
jest.mock('../src/services/api', () => ({
  coachesApi: { list: (...args: unknown[]) => mockListCoaches(...args) },
}));

import { CoachInfoSheet } from '../src/screens/chat/CoachInfoSheet';
import { COACH_EDIT_ROUTE } from '../src/navigation/routes';

function coach(overrides: Partial<Coach>): Coach {
  return {
    id: 'coach-1',
    title: 'Coach Tempo',
    description: 'Threshold work and long runs.',
    system_prompt: '',
    category: 'training',
    tags: [],
    token_count: 0,
    is_favorite: false,
    use_count: 0,
    last_used_at: null,
    created_at: '2026-08-01T00:00:00Z',
    updated_at: '2026-08-01T00:00:00Z',
    is_system: false,
    handle: 'coach-tempo',
    ...overrides,
  } as Coach;
}

function renderSheet(coachId = 'coach-1') {
  const handlers = { onSendCommand: jest.fn(), onClose: jest.fn() };
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const view = render(
    <QueryClientProvider client={client}>
      <CoachInfoSheet coachId={coachId} fallbackTitle="Coach Tempo" {...handlers} />
    </QueryClientProvider>,
  );
  return { ...view, handlers };
}

describe('CoachInfoSheet', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockListCoaches.mockResolvedValue({ coaches: [coach({})] });
  });

  it('names the agent and teaches the handle a mention would use', async () => {
    const { findByTestId, getByTestId, getByText } = renderSheet();

    expect(await findByTestId('coach-info-handle')).toHaveTextContent('@coach-tempo');
    expect(getByTestId('coach-info-title')).toHaveTextContent('Coach Tempo');
    expect(getByTestId('coach-info-description')).toHaveTextContent('Threshold work and long runs.');
    expect(getByText(/Mention @coach-tempo in any chat/)).toBeTruthy();
  });

  it('detaches the agent by sending /agent remove', async () => {
    const { findByTestId, handlers } = renderSheet();

    fireEvent.press(await findByTestId('coach-info-remove'));

    expect(handlers.onClose).toHaveBeenCalledTimes(1);
    expect(handlers.onSendCommand).toHaveBeenCalledWith(COMMAND_DRAFTS.coachRemove);
  });

  it('offers Edit agent for the athlete own agent', async () => {
    const { findByTestId } = renderSheet();

    fireEvent.press(await findByTestId('coach-info-edit'));

    expect(mockPush).toHaveBeenCalledWith({
      pathname: COACH_EDIT_ROUTE,
      params: { coachId: 'coach-1' },
    });
  });

  // A system agent is shared by every tenant and the server refuses the write,
  // so offering Edit would advertise a 403.
  it('offers no Edit agent for a system agent', async () => {
    mockListCoaches.mockResolvedValue({ coaches: [coach({ is_system: true })] });
    const { findByTestId, queryByTestId } = renderSheet();

    await findByTestId('coach-info-handle');
    await waitFor(() => expect(queryByTestId('coach-info-edit')).toBeNull());
  });

  it('still names the thread agent while the list is loading', () => {
    mockListCoaches.mockReturnValue(new Promise(() => undefined));
    const { getByTestId, queryByTestId } = renderSheet();
    expect(getByTestId('coach-info-title')).toHaveTextContent('Coach Tempo');
    expect(queryByTestId('coach-info-edit')).toBeNull();
  });
});
