// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the chat "+" — new chat, new group chat by name, add someone to the open thread
// ABOUTME: Covers the header "+" and the empty list's call to action, the platform menu both present, and the flows each action opens
// ABOUTME: New chat creates a conversation beside the personal thread (carnet#1473), never a topic inside it

import React from 'react';
import { ActionSheetIOS, Alert } from 'react-native';
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
const mockOpenPersonalTopic = jest.fn();
const mockGetPersonalConversation = jest.fn();
const mockListParticipants = jest.fn();
jest.mock('../src/services/api', () => ({
  chatApi: {
    getConversations: (...args: unknown[]) => mockGetConversations(...args),
    createConversation: (...args: unknown[]) => mockCreateConversation(...args),
    openPersonalTopic: (...args: unknown[]) => mockOpenPersonalTopic(...args),
    getPersonalConversation: () => mockGetPersonalConversation(),
    listParticipants: (...args: unknown[]) => mockListParticipants(...args),
    addParticipant: jest.fn(),
    removeParticipant: jest.fn(),
    updateConversation: jest.fn(),
    deleteConversation: jest.fn(),
  },
  coachesApi: { list: jest.fn().mockResolvedValue({ agents: [] }) },
  notificationsApi: { getUnreadCount: jest.fn().mockResolvedValue({ unread_count: 0 }) },
}));

import { ConversationsScreen } from '../src/screens/conversations/ConversationsScreen';
import { ChatHeaderTitle } from '../src/screens/chat/ChatHeaderTitle';
import { ChatPlusFlows } from '../src/screens/chat/ChatPlusFlows';
import { presentMenu } from '../src/utils/presentMenu';
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
    presentMenu(chatPlus.actions, { title: 'New', cancelLabel: 'Cancel' });
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

    // The empty state's link presents the same rows, in the same order.
    fireEvent.press(await findByTestId('conversations-empty-start'));
    expect(presentedMenu().labels).toEqual(header.labels);
    expect(getByTestId('conversations-screen')).toBeTruthy();
  });

  // carnet#1473: new chat is a conversation of its own, beside the personal
  // thread, and the app lands there.
  // A look forges nothing: an athlete who has not begun gets the empty
  // thread, whose first send opens their personal thread.
  it('new chat opens an empty thread for an athlete who has not begun', async () => {
    mockGetPersonalConversation.mockResolvedValue(null);
    const { findByTestId } = render(withClient(<ConversationsScreen />));

    fireEvent.press(await findByTestId('conversations-empty-start'));
    presentedMenu().pick('New chat');

    await waitFor(() =>
      expect(mockRouter.push).toHaveBeenCalledWith({
        pathname: CHAT_THREAD_ROUTE,
        params: { conversationId: 'new' },
      }),
    );
    expect(mockOpenPersonalTopic).not.toHaveBeenCalled();
    expect(mockCreateConversation).not.toHaveBeenCalled();
  });

  it('new chat creates a conversation beside the personal thread and lands in it', async () => {
    mockGetPersonalConversation.mockResolvedValue({ id: 'conv-personal', title: 'Dravr', personal: true });
    mockCreateConversation.mockResolvedValue({ id: 'conv-new', title: 'Dravr' });
    const { findByTestId } = render(withClient(<ConversationsScreen />));

    fireEvent.press(await findByTestId('conversations-empty-start'));
    presentedMenu().pick('New chat');

    await waitFor(() =>
      expect(mockRouter.push).toHaveBeenCalledWith({
        pathname: CHAT_THREAD_ROUTE,
        params: { conversationId: 'conv-new' },
      }),
    );
    // The server binds the selected agent and posts its welcome: the athlete
    // does not speak first, and the personal thread gets no divider or rebind.
    expect(mockCreateConversation).toHaveBeenCalledTimes(1);
    expect(mockCreateConversation).toHaveBeenCalledWith(undefined);
    expect(mockOpenPersonalTopic).not.toHaveBeenCalled();
  });

  // Turns red if "+" collapses back into one page: each press is a new thread.
  it('two new chats in a row are two distinct conversations', async () => {
    mockGetPersonalConversation.mockResolvedValue({ id: 'conv-personal', title: 'Dravr', personal: true });
    mockCreateConversation
      .mockResolvedValueOnce({ id: 'conv-first', title: 'Dravr' })
      .mockResolvedValueOnce({ id: 'conv-second', title: 'Dravr' });
    const { findByTestId } = render(withClient(<ConversationsScreen />));

    fireEvent.press(await findByTestId('conversations-empty-start'));
    presentedMenu().pick('New chat');
    await waitFor(() => expect(mockRouter.push).toHaveBeenCalledTimes(1));

    fireEvent.press(await findByTestId('conversations-empty-start'));
    presentedMenu().pick('New chat');
    await waitFor(() => expect(mockRouter.push).toHaveBeenCalledTimes(2));

    expect(mockRouter.push.mock.calls.map(([href]) => href.params.conversationId)).toEqual([
      'conv-first',
      'conv-second',
    ]);
    expect(mockCreateConversation).toHaveBeenCalledTimes(2);
    expect(mockOpenPersonalTopic).not.toHaveBeenCalled();
  });

  // At the plan's cap the server refuses one more thread with a 429 naming the
  // limit: the athlete is told which limit, with its counts, and goes nowhere.
  it('new chat at the conversation cap says so and opens nothing', async () => {
    const alert = jest.spyOn(Alert, 'alert').mockImplementation(() => undefined);
    mockGetPersonalConversation.mockResolvedValue({ id: 'conv-personal', title: 'Dravr', personal: true });
    mockCreateConversation.mockRejectedValue({
      response: {
        status: 429,
        data: {
          error: 'max_active_conversations quota exceeded: 10/10',
          details: { limit_type: 'max_active_conversations', current: 10, limit: 10 },
        },
      },
    });
    const { findByTestId } = render(withClient(<ConversationsScreen />));

    fireEvent.press(await findByTestId('conversations-empty-start'));
    presentedMenu().pick('New chat');

    await waitFor(() =>
      expect(alert).toHaveBeenCalledWith(
        'Conversation limit reached',
        'Conversation limit reached (10/10). Delete an existing conversation to start a new one.',
      ),
    );
    expect(mockRouter.push).not.toHaveBeenCalled();
    expect(mockOpenPersonalTopic).not.toHaveBeenCalled();
  });

  // Turns red if "New group chat" regrows a picker or a createGroup call:
  // the group is created by the command, in a fresh thread, exactly as it is
  // on web and in messaging.
  it('new group chat asks for a name and sends /group create in a fresh thread', async () => {
    const { findByTestId, getByTestId } = render(withClient(<ConversationsScreen />));

    fireEvent.press(await findByTestId('conversations-empty-start'));
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

    fireEvent.press(await findByTestId('conversations-empty-start'));
    presentedMenu().pick('New group chat');

    fireEvent.changeText(await findByTestId('new-group-name-dialog-input'), '   ');
    fireEvent.press(getByTestId('new-group-name-dialog-submit'));

    expect(mockRouter.push).not.toHaveBeenCalled();
  });

  it('cancel runs nothing', async () => {
    const { findByTestId } = render(withClient(<ConversationsScreen />));

    fireEvent.press(await findByTestId('conversations-empty-start'));
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
