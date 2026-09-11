// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the chat "+" — new chat, new group chat by name, add someone to the open thread
// ABOUTME: Covers the header "+" and the empty list's call to action, the platform menu both present, and the flows each action opens

import React from 'react';
import { ActionSheetIOS } from 'react-native';
import { fireEvent, render, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';

const mockRouter = {
  push: jest.fn(),
  replace: jest.fn(),
  back: jest.fn(),
  navigate: jest.fn(),
  canGoBack: () => true,
};
jest.mock('expo-router', () =>
  require('../jest.expo-router').createExpoRouterMock({
    useRouter: () => mockRouter,
  }),
);

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
import { ChatHeaderTitle } from '../src/screens/chat/ChatHeaderTitle';
import { ChatPlusFlows } from '../src/screens/chat/ChatPlusFlows';
import { presentChatPlusMenu } from '../src/screens/chat/presentChatPlusMenu';
import { useChatPlusActions } from '../src/screens/chat/useChatPlusActions';
import { CHAT_THREAD_ROUTE } from '../src/navigation/routes';
import { COMMAND_DRAFTS } from '@pierre/shared-constants';

function withClient(ui: React.ReactElement) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return <QueryClientProvider client={client}>{ui}</QueryClientProvider>;
}

type SheetOptions = { options: string[]; cancelButtonIndex?: number };
type SheetCallback = (index: number) => void;

/** The rows the platform menu last offered, and a way to pick one. */
function presentedMenu() {
  const spy = ActionSheetIOS.showActionSheetWithOptions as unknown as jest.Mock;
  expect(spy).toHaveBeenCalled();
  const [options, callback] = spy.mock.calls[spy.mock.calls.length - 1] as [SheetOptions, SheetCallback];
  return {
    labels: options.options,
    cancelButtonIndex: options.cancelButtonIndex,
    pick: (label: string) => callback(options.options.indexOf(label)),
  };
}

/** The thread's "+": the same menu, with a conversation open. */
function ThreadPlus({ conversationId }: { conversationId: string }) {
  const chatPlus = useChatPlusActions(conversationId);
  React.useEffect(() => {
    presentChatPlusMenu({ actions: chatPlus.actions, cancelLabel: 'Cancel', title: 'New' });
  }, [chatPlus.actions]);
  return <ChatPlusFlows flows={chatPlus.flows} />;
}

describe('the chat "+"', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    jest.spyOn(ActionSheetIOS, 'showActionSheetWithOptions').mockImplementation(() => undefined);
    mockGetConversations.mockResolvedValue({ conversations: [], total: 0, limit: 50, offset: 0 });
    mockListParticipants.mockResolvedValue([]);
  });

  afterEach(() => {
    jest.restoreAllMocks();
  });

  // The "+" is the header's (Boreal v2.2 D1): the tab bar that used to carry
  // it is the system's now. On an empty list the call-to-action "+" presents
  // the same menu, which is what these tests are about.
  it('offers exactly new chat and new group chat from the conversation list', async () => {
    const { findByTestId, getByTestId } = render(withClient(<ConversationsScreen />));

    fireEvent.press(await findByTestId('new-chat-button'));

    const header = presentedMenu();
    expect(header.labels).toEqual(['New chat', 'New group chat', 'Cancel']);
    expect(header.cancelButtonIndex).toBe(2);

    // The empty state's "+" presents the same rows, in the same order.
    fireEvent.press(await findByTestId('conversations-empty-plus'));
    expect(presentedMenu().labels).toEqual(header.labels);
    expect(getByTestId('conversations-screen')).toBeTruthy();
  });

  it('new chat opens an empty thread', async () => {
    const { findByTestId } = render(withClient(<ConversationsScreen />));

    fireEvent.press(await findByTestId('conversations-empty-plus'));
    presentedMenu().pick('New chat');

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

    fireEvent.press(await findByTestId('conversations-empty-plus'));
    presentedMenu().pick('New group chat');

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

    fireEvent.press(await findByTestId('conversations-empty-plus'));
    presentedMenu().pick('New group chat');

    fireEvent.changeText(await findByTestId('new-group-name-dialog-input'), '   ');
    fireEvent.press(getByTestId('new-group-name-dialog-submit'));

    expect(mockRouter.push).not.toHaveBeenCalled();
  });

  it('cancel runs nothing', async () => {
    const { findByTestId } = render(withClient(<ConversationsScreen />));

    fireEvent.press(await findByTestId('conversations-empty-plus'));
    presentedMenu().pick('Cancel');

    expect(mockRouter.push).not.toHaveBeenCalled();
  });

  // Turns red if "add someone" stops opening the participants control from
  // ws4, or opens it for the wrong conversation.
  it('adds someone to the open discussion through the participants sheet', async () => {
    mockListParticipants.mockResolvedValue([
      { user_id: 'owner-1', role: 'owner', added_by: 'owner-1', added_at: '2026-08-26T00:00:00Z' },
      { user_id: 'friend-2', role: 'member', added_by: 'owner-1', added_at: '2026-08-26T00:00:00Z' },
    ]);
    const { findByTestId } = render(withClient(<ThreadPlus conversationId="conv-1" />));

    const menu = presentedMenu();
    expect(menu.labels).toEqual(['New chat', 'New group chat', 'Add someone to this discussion', 'Cancel']);
    menu.pick('Add someone to this discussion');

    expect(await findByTestId('conversation-participants-modal')).toBeTruthy();
    expect(await findByTestId('participant-friend-2')).toBeTruthy();
    expect(mockListParticipants).toHaveBeenCalledWith('conv-1');
  });

  // The thread showed two "+" at once — one in its header, one in the tab bar
  // — and both opened this same sheet (carnet#213). The header's title view
  // carries the thread and nothing else; the bar is the system's.
  it('the thread title view carries no add control, only the avatar and the title', () => {
    const { getAllByTestId, queryByTestId } = render(
      withClient(
        <ChatHeaderTitle currentConversation={null} providerStatus={null} onTitlePress={jest.fn()} />,
      ),
    );

    // The whole title view, named: a control that grows back here fails this.
    const rendered = getAllByTestId(/./).map((node) => node.props.testID);
    expect(rendered).toEqual(['chat-title-button', 'chat-title']);
    expect(queryByTestId('chat-plus-button')).toBeNull();
    expect(queryByTestId('history-button')).toBeNull();
  });
});
