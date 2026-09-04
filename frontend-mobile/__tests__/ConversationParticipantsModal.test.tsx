// ABOUTME: Tests for the mobile ConversationParticipantsModal — list, add by user id, remove a member
// ABOUTME: Mocks chatApi.listParticipants/addParticipant/removeParticipant and asserts the calls
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React from 'react';
import { render, fireEvent, waitFor } from '@testing-library/react-native';

const mockListParticipants = jest.fn();
const mockAddParticipant = jest.fn();
const mockRemoveParticipant = jest.fn();

jest.mock('../src/services/api', () => ({
  chatApi: {
    listParticipants: (...args: unknown[]) => mockListParticipants(...args),
    addParticipant: (...args: unknown[]) => mockAddParticipant(...args),
    removeParticipant: (...args: unknown[]) => mockRemoveParticipant(...args),
  },
}));

import { ConversationParticipantsModal } from '../src/screens/chat/ConversationParticipantsModal';

const CONVERSATION_ID = 'conv-1';
const OWNER_ID = '11111111-1111-4111-8111-111111111111';
const MEMBER_ID = '22222222-2222-4222-8222-222222222222';
const NEWCOMER_ID = '33333333-3333-4333-8333-333333333333';

function participant(overrides: Record<string, unknown> = {}) {
  return {
    user_id: OWNER_ID,
    role: 'owner',
    added_by: OWNER_ID,
    added_at: '2026-08-26T10:00:00Z',
    ...overrides,
  };
}

describe('ConversationParticipantsModal', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockListParticipants.mockResolvedValue([
      participant(),
      participant({ user_id: MEMBER_ID, role: 'member' }),
    ]);
  });

  it('lists owner and member, with a remove control only on the member', async () => {
    const { findByTestId, queryByTestId } = render(
      <ConversationParticipantsModal visible conversationId={CONVERSATION_ID} onClose={jest.fn()} />,
    );

    expect(await findByTestId(`participant-${OWNER_ID}`)).toBeTruthy();
    expect(await findByTestId(`participant-${MEMBER_ID}`)).toBeTruthy();
    expect(mockListParticipants).toHaveBeenCalledWith(CONVERSATION_ID);
    expect(queryByTestId(`remove-${OWNER_ID}`)).toBeNull();
    expect(queryByTestId(`remove-${MEMBER_ID}`)).toBeTruthy();
  });

  it('adds a participant by user id and reloads', async () => {
    mockAddParticipant.mockResolvedValue(participant({ user_id: NEWCOMER_ID, role: 'member' }));
    const { findByTestId, getByTestId } = render(
      <ConversationParticipantsModal visible conversationId={CONVERSATION_ID} onClose={jest.fn()} />,
    );
    await findByTestId(`participant-${OWNER_ID}`);

    fireEvent.changeText(getByTestId('participant-user-id-input'), `  ${NEWCOMER_ID} `);
    fireEvent.press(getByTestId('participant-add-button'));

    await waitFor(() =>
      expect(mockAddParticipant).toHaveBeenCalledWith(CONVERSATION_ID, NEWCOMER_ID),
    );
    await waitFor(() => expect(mockListParticipants).toHaveBeenCalledTimes(2));
    expect(getByTestId('participant-user-id-input').props.value).toBe('');
  });

  it('removes a member and reloads', async () => {
    mockRemoveParticipant.mockResolvedValue(undefined);
    const { findByTestId, getByTestId } = render(
      <ConversationParticipantsModal visible conversationId={CONVERSATION_ID} onClose={jest.fn()} />,
    );
    await findByTestId(`participant-${MEMBER_ID}`);

    fireEvent.press(getByTestId(`remove-${MEMBER_ID}`));

    await waitFor(() =>
      expect(mockRemoveParticipant).toHaveBeenCalledWith(CONVERSATION_ID, MEMBER_ID),
    );
    await waitFor(() => expect(mockListParticipants).toHaveBeenCalledTimes(2));
  });

  it('shows the server refusal when an add is rejected', async () => {
    // The server answers this one 403 with the sentence below, so the fixture
    // wears the refusal's real shape: a bare Error carries no response and
    // reads as a dead network, which is a different message entirely.
    mockAddParticipant.mockRejectedValue({
      response: {
        status: 403,
        data: {
          code: 'PermissionDenied',
          message: 'Cannot add a user who is not a member of this tenant',
        },
      },
    });
    const { findByTestId, getByTestId } = render(
      <ConversationParticipantsModal visible conversationId={CONVERSATION_ID} onClose={jest.fn()} />,
    );
    await findByTestId(`participant-${OWNER_ID}`);

    fireEvent.changeText(getByTestId('participant-user-id-input'), NEWCOMER_ID);
    fireEvent.press(getByTestId('participant-add-button'));

    const error = await findByTestId('participants-error');
    expect(error.props.children).toBe('Cannot add a user who is not a member of this tenant');
    expect(mockListParticipants).toHaveBeenCalledTimes(1);
  });

  it('does not load while hidden', () => {
    render(
      <ConversationParticipantsModal visible={false} conversationId={CONVERSATION_ID} onClose={jest.fn()} />,
    );
    expect(mockListParticipants).not.toHaveBeenCalled();
  });
});
