// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for ConversationParticipants — list, add by user id, remove a member
// ABOUTME: Mocks chatApi.listParticipants/addParticipant/removeParticipant and asserts the calls

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import ConversationParticipants from '../ConversationParticipants';
import { ToastProvider } from '../../ui';
import type { ConversationParticipant } from '@pierre/shared-types';

const listParticipants = vi.fn();
const addParticipant = vi.fn();
const removeParticipant = vi.fn();

vi.mock('../../../services/api', () => ({
  chatApi: {
    listParticipants: (...a: unknown[]) => listParticipants(...a),
    addParticipant: (...a: unknown[]) => addParticipant(...a),
    removeParticipant: (...a: unknown[]) => removeParticipant(...a),
  },
}));

const CONVERSATION_ID = 'conv-1';
const OWNER_ID = '11111111-1111-4111-8111-111111111111';
const MEMBER_ID = '22222222-2222-4222-8222-222222222222';
const NEWCOMER_ID = '33333333-3333-4333-8333-333333333333';

function participant(overrides: Partial<ConversationParticipant>): ConversationParticipant {
  return {
    user_id: OWNER_ID,
    role: 'owner',
    added_by: OWNER_ID,
    added_at: '2026-08-26T10:00:00Z',
    ...overrides,
  };
}

function renderComponent() {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={queryClient}>
      <ToastProvider>
        <ConversationParticipants conversationId={CONVERSATION_ID} />
      </ToastProvider>
    </QueryClientProvider>,
  );
}

describe('ConversationParticipants', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    listParticipants.mockResolvedValue([
      participant({}),
      participant({ user_id: MEMBER_ID, role: 'member', added_by: OWNER_ID }),
    ]);
  });

  it('shows the participant count and lists owner and member when opened', async () => {
    renderComponent();

    expect(await screen.findByText('Participants (2)')).toBeInTheDocument();
    expect(listParticipants).toHaveBeenCalledWith(CONVERSATION_ID);

    fireEvent.click(screen.getByRole('button', { name: /participants/i }));
    const list = screen.getByRole('list', { name: 'Participant list' });
    expect(list).toHaveTextContent(`${OWNER_ID} · owner`);
    expect(list).toHaveTextContent(MEMBER_ID);

    // The owner has no remove control; the member does.
    expect(screen.queryByRole('button', { name: `Remove ${OWNER_ID}` })).toBeNull();
    expect(screen.getByRole('button', { name: `Remove ${MEMBER_ID}` })).toBeInTheDocument();
  });

  it('adds a participant by user id and refetches the list', async () => {
    addParticipant.mockResolvedValue(
      participant({ user_id: NEWCOMER_ID, role: 'member', added_by: OWNER_ID }),
    );
    renderComponent();
    await screen.findByText('Participants (2)');
    fireEvent.click(screen.getByRole('button', { name: /participants/i }));

    const input = screen.getByLabelText('User id to add');
    fireEvent.change(input, { target: { value: `  ${NEWCOMER_ID}  ` } });
    fireEvent.click(screen.getByRole('button', { name: 'Add' }));

    await waitFor(() => expect(addParticipant).toHaveBeenCalledWith(CONVERSATION_ID, NEWCOMER_ID));
    await waitFor(() => expect(listParticipants).toHaveBeenCalledTimes(2));
    expect((input as HTMLInputElement).value).toBe('');
  });

  it('removes a member and refetches the list', async () => {
    removeParticipant.mockResolvedValue(undefined);
    renderComponent();
    await screen.findByText('Participants (2)');
    fireEvent.click(screen.getByRole('button', { name: /participants/i }));

    fireEvent.click(screen.getByRole('button', { name: `Remove ${MEMBER_ID}` }));

    await waitFor(() => expect(removeParticipant).toHaveBeenCalledWith(CONVERSATION_ID, MEMBER_ID));
    await waitFor(() => expect(listParticipants).toHaveBeenCalledTimes(2));
  });

  it('surfaces the server refusal when an add is rejected', async () => {
    addParticipant.mockRejectedValue(new Error('Cannot add a user who is not a member of this tenant'));
    renderComponent();
    await screen.findByText('Participants (2)');
    fireEvent.click(screen.getByRole('button', { name: /participants/i }));

    fireEvent.change(screen.getByLabelText('User id to add'), { target: { value: NEWCOMER_ID } });
    fireEvent.click(screen.getByRole('button', { name: 'Add' }));

    expect(
      await screen.findByText('Cannot add a user who is not a member of this tenant'),
    ).toBeInTheDocument();
    expect(listParticipants).toHaveBeenCalledTimes(1);
  });
});
