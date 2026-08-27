// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the chat "+" — new chat, new group chat by name, add someone to the open thread
// ABOUTME: Covers the conversation list's sheet, the thread header's sheet, and the flows each action opens

import React from 'react';
import { fireEvent, render, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';

const mockRouter = {
  push: jest.fn(),
  replace: jest.fn(),
  back: jest.fn(),
  navigate: jest.fn(),
  canGoBack: () => true,
};
jest.mock('expo-router', () => {
  const React = require('react');
  return {
    useRouter: () => mockRouter,
    useLocalSearchParams: () => ({}),
    useFocusEffect: (cb: () => void | (() => void)) => {
      React.useEffect(() => cb(), [cb]);
    },
  };
});
jest.mock('../src/contexts/AuthContext', () => ({
  useAuth: () => ({ isAuthenticated: true }),
}));

const mockGetConversations = jest.fn();
const mockCreateConversation = jest.fn();
const mockListParticipants = jest.fn();
jest.mock('../src/services/api', () => ({
  chatApi: {
    getConversations: (...args: unknown[]) => mockGetConversations(...args),
    createConversation: (...args: unknown[]) => mockCreateConversation(...args),
    listParticipants: (...args: unknown[]) => mockListParticipants(...args),
    addParticipant: jest.fn(),
    removeParticipant: jest.fn(),
    updateConversation: jest.fn(),
    deleteConversation: jest.fn(),
  },
  coachesApi: { list: jest.fn().mockResolvedValue({ coaches: [] }) },
  notificationsApi: { getUnreadCount: jest.fn().mockResolvedValue({ unread_count: 0 }) },
}));

import { ConversationsScreen } from '../src/screens/conversations/ConversationsScreen';
import { ChatHeader } from '../src/screens/chat/ChatHeader';
import { ChatPlusSheet } from '../src/screens/chat/ChatPlusSheet';
import { ChatPlusFlows } from '../src/screens/chat/ChatPlusFlows';
import { useChatPlusActions } from '../src/screens/chat/useChatPlusActions';
import { CHAT_THREAD_ROUTE } from '../src/navigation/routes';
import { COMMAND_DRAFTS } from '@pierre/shared-constants';

function withClient(ui: React.ReactElement) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return <QueryClientProvider client={client}>{ui}</QueryClientProvider>;
}

/** The thread's "+": the same sheet, with a conversation open. */
function ThreadPlus({ conversationId }: { conversationId: string }) {
  const chatPlus = useChatPlusActions(conversationId);
  const [visible, setVisible] = React.useState(true);
  return (
    <>
      <ChatPlusSheet visible={visible} onClose={() => setVisible(false)} actions={chatPlus.actions} />
      <ChatPlusFlows flows={chatPlus.flows} />
    </>
  );
}

describe('the chat "+"', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockGetConversations.mockResolvedValue({ conversations: [], total: 0, limit: 50, offset: 0 });
    mockListParticipants.mockResolvedValue([]);
  });

  it('offers exactly new chat and new group chat from the conversation list', async () => {
    const { findByTestId, getByTestId, queryByTestId, getByText } = render(withClient(<ConversationsScreen />));

    fireEvent.press(await findByTestId('chat-plus-button'));

    expect(getByTestId('chat-plus-sheet')).toBeTruthy();
    expect(getByText('New chat')).toBeTruthy();
    expect(getByText('New group chat')).toBeTruthy();
    expect(getByTestId('chat-plus-action-new-chat')).toBeTruthy();
    expect(getByTestId('chat-plus-action-new-group-chat')).toBeTruthy();
    // No thread is open on the list, so there is nothing to add someone to.
    expect(queryByTestId('chat-plus-action-add-participant')).toBeNull();
  });

  it('new chat opens an empty thread', async () => {
    const { findByTestId, getByTestId } = render(withClient(<ConversationsScreen />));

    fireEvent.press(await findByTestId('chat-plus-button'));
    fireEvent.press(getByTestId('chat-plus-action-new-chat'));

    expect(mockRouter.push).toHaveBeenCalledWith({
      pathname: CHAT_THREAD_ROUTE,
      params: { conversationId: 'new' },
    });
  });

  // Turns red if "New group chat" regrows a picker or a createGroup call:
  // the group is created by the command, in a fresh thread, exactly as it is
  // on web and in messaging.
  it('new group chat asks for a name and sends /group create in a fresh thread', async () => {
    const { findByTestId, getByTestId } = render(withClient(<ConversationsScreen />));

    fireEvent.press(await findByTestId('chat-plus-button'));
    fireEvent.press(getByTestId('chat-plus-action-new-group-chat'));

    const dialog = await findByTestId('new-group-name-dialog-input');
    fireEvent.changeText(dialog, 'Marathon Squad');
    fireEvent.press(getByTestId('new-group-name-dialog-submit'));

    await waitFor(() => {
      expect(mockRouter.push).toHaveBeenCalledWith({
        pathname: CHAT_THREAD_ROUTE,
        params: { conversationId: 'new', send: COMMAND_DRAFTS.groupCreate('Marathon Squad') },
      });
    });
    // Nothing is created client-side: the command is the one implementation.
    expect(mockCreateConversation).not.toHaveBeenCalled();
  });

  it('an empty name creates nothing', async () => {
    const { findByTestId, getByTestId } = render(withClient(<ConversationsScreen />));

    fireEvent.press(await findByTestId('chat-plus-button'));
    fireEvent.press(getByTestId('chat-plus-action-new-group-chat'));

    fireEvent.changeText(await findByTestId('new-group-name-dialog-input'), '   ');
    fireEvent.press(getByTestId('new-group-name-dialog-submit'));

    expect(mockRouter.push).not.toHaveBeenCalled();
  });

  // Turns red if "add someone" stops opening the participants control from
  // ws4, or opens it for the wrong conversation.
  it('adds someone to the open discussion through the participants sheet', async () => {
    mockListParticipants.mockResolvedValue([
      { user_id: 'owner-1', role: 'owner', added_by: 'owner-1', added_at: '2026-08-26T00:00:00Z' },
      { user_id: 'friend-2', role: 'member', added_by: 'owner-1', added_at: '2026-08-26T00:00:00Z' },
    ]);
    const { getByTestId, getByText, findByTestId } = render(withClient(<ThreadPlus conversationId="conv-1" />));

    expect(getByText('Add someone to this discussion')).toBeTruthy();
    fireEvent.press(getByTestId('chat-plus-action-add-participant'));

    expect(await findByTestId('conversation-participants-modal')).toBeTruthy();
    expect(await findByTestId('participant-friend-2')).toBeTruthy();
    expect(mockListParticipants).toHaveBeenCalledWith('conv-1');
  });

  it('the thread header carries the "+" and a back button, not the old history button', () => {
    const onPlusPress = jest.fn();
    const onBackPress = jest.fn();
    const { getByTestId, queryByTestId } = render(
      withClient(
        <ChatHeader
          currentConversation={null}
          insetTop={0}
          onBackPress={onBackPress}
          onPlusPress={onPlusPress}
          onTitlePress={jest.fn()}
        />,
      ),
    );

    fireEvent.press(getByTestId('chat-plus-button'));
    fireEvent.press(getByTestId('back-button'));
    expect(onPlusPress).toHaveBeenCalledTimes(1);
    expect(onBackPress).toHaveBeenCalledTimes(1);
    expect(queryByTestId('history-button')).toBeNull();
  });
});
