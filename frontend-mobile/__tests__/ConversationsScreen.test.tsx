// ABOUTME: Tests for the mobile conversation list — flat rows, unread state, swipe actions, pull-to-refresh, search
// ABOUTME: Drives the real React Query hook over a mocked chatApi so every row action proves the request it makes
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React from 'react';
import { render as rtlRender, fireEvent, waitFor, act } from '@testing-library/react-native';
import { Alert } from 'react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';

const mockGetConversations = jest.fn();
const mockUpdateConversation = jest.fn();
const mockDeleteConversation = jest.fn();
const mockMarkConversationRead = jest.fn();
const mockMarkConversationUnread = jest.fn();
const mockPush = jest.fn();

jest.mock('../src/services/api', () => ({
  chatApi: {
    getConversations: (...args: unknown[]) => mockGetConversations(...args),
    updateConversation: (...args: unknown[]) => mockUpdateConversation(...args),
    deleteConversation: (...args: unknown[]) => mockDeleteConversation(...args),
    markConversationRead: (...args: unknown[]) => mockMarkConversationRead(...args),
    markConversationUnread: (...args: unknown[]) => mockMarkConversationUnread(...args),
    listParticipants: jest.fn().mockResolvedValue([]),
    addParticipant: jest.fn(),
    removeParticipant: jest.fn(),
  },
  notificationsApi: { getUnreadCount: jest.fn().mockResolvedValue({ unread_count: 0 }) },
}));

jest.mock('expo-router', () => ({
  useRouter: () => ({ push: mockPush, back: jest.fn(), navigate: jest.fn(), replace: jest.fn() }),
  useFocusEffect: (cb: () => void) => {
    // Run the effect body once on mount, the way a screen is focused when it appears.
    const React = require('react');
    React.useEffect(() => {
      const cleanup = cb();
      return cleanup;
    }, [cb]);
  },
}));

import { ConversationsScreen, EMPTY_LIST_LINE } from '../src/screens/conversations/ConversationsScreen';
import { threadHref } from '../src/navigation/routes';

/** The list is the chat tab's landing screen; its header bell and its rows read a react-query cache. */
function render(ui: React.ReactElement) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  return rtlRender(<QueryClientProvider client={client}>{ui}</QueryClientProvider>);
}

type Conv = {
  id: string;
  title: string | null;
  coach_id?: string | null;
  coach_handle?: string | null;
  coach_title?: string | null;
  group_id?: string | null;
  group_name?: string | null;
  channel_type?: string | null;
  message_count: number;
  unread_count?: number;
  last_message?: { preview: string; role: 'user' | 'assistant'; created_at: string } | null;
  created_at: string;
  updated_at: string;
};

function makeConv(overrides: Partial<Conv> = {}): Conv {
  return {
    id: 'c1',
    title: 'Hello',
    coach_id: null,
    message_count: 4,
    unread_count: 0,
    created_at: '2026-04-10T10:00:00Z',
    updated_at: '2026-04-13T10:00:00Z',
    ...overrides,
  };
}

function page(conversations: Conv[], total = conversations.length) {
  return { conversations, total, limit: 50, offset: 0 };
}

/**
 * Serve the list from a mutable server the row mutations also write to.
 *
 * Every mutation on this screen patches the cache and then invalidates, so a
 * mock that always replays the pre-mutation page would revert the row and
 * hide the very thing the test is asserting. This models the server instead:
 * the request is proven by the mock's arguments, and the row is proven by
 * what the next read returns.
 */
function serveMutableList(initial: Conv[]) {
  let rows = [...initial];
  mockGetConversations.mockImplementation(() => Promise.resolve(page(rows, rows.length)));
  mockMarkConversationUnread.mockImplementation((id: string) => {
    rows = rows.map((row) => (row.id === id ? { ...row, unread_count: row.message_count } : row));
    return Promise.resolve(undefined);
  });
  mockMarkConversationRead.mockImplementation((id: string) => {
    rows = rows.map((row) => (row.id === id ? { ...row, unread_count: 0 } : row));
    return Promise.resolve(undefined);
  });
  mockDeleteConversation.mockImplementation((id: string) => {
    rows = rows.filter((row) => row.id !== id);
    return Promise.resolve(undefined);
  });
  mockUpdateConversation.mockImplementation((id: string, patch: { title: string }) => {
    rows = rows.map((row) => (row.id === id ? { ...row, title: patch.title } : row));
    const updated = rows.find((row) => row.id === id);
    return Promise.resolve({ id, title: updated?.title ?? patch.title, updated_at: '2026-04-14T10:00:00Z' });
  });
}

describe('ConversationsScreen — one flat list', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    jest.spyOn(Alert, 'alert').mockImplementation(() => undefined);
  });

  afterEach(() => {
    jest.restoreAllMocks();
  });

  it('shows one line and the "+" when there is no conversation', async () => {
    mockGetConversations.mockResolvedValueOnce(page([]));

    const { findByText, getByTestId } = render(<ConversationsScreen />);

    expect(await findByText(EMPTY_LIST_LINE)).toBeTruthy();
    expect(getByTestId('conversations-empty-plus')).toBeTruthy();
    // The list asks for its first page, and only that.
    expect(mockGetConversations).toHaveBeenCalledTimes(1);
    expect(mockGetConversations).toHaveBeenCalledWith(50, 0);
  });

  // Turns red if the list regrows coach headers: every conversation is one
  // row, whatever created it, newest activity first.
  it('renders every conversation as a flat row, newest activity first, with no grouping', async () => {
    mockGetConversations.mockResolvedValueOnce(
      page([
        makeConv({ id: 'c1', title: 'Training plan', coach_id: 'coach-1', coach_handle: 'coach-tempo', updated_at: '2026-04-11T10:00:00Z' }),
        makeConv({ id: 'c2', title: 'Race strategy', coach_id: 'coach-1', updated_at: '2026-04-13T10:00:00Z' }),
        makeConv({ id: 'c3', title: 'Orphan chat', coach_id: null, updated_at: '2026-04-12T10:00:00Z' }),
        makeConv({ id: 'c4', title: 'Harricana', group_id: 'group-1', group_name: 'Harricana', updated_at: '2026-04-10T10:00:00Z' }),
      ]),
    );

    const { findByTestId, getAllByTestId, queryByTestId, queryByText, getByTestId } = render(<ConversationsScreen />);

    await findByTestId('conversation-row-c1');
    const order = getAllByTestId(/^conversation-row-/).map((node) => node.props.testID);
    expect(order).toEqual(['conversation-row-c2', 'conversation-row-c3', 'conversation-row-c1', 'conversation-row-c4']);
    expect(queryByTestId(/^session-group-header-/)).toBeNull();
    expect(queryByText('Without a coach')).toBeNull();
    // The coach handle and the group glyph ride the row, not a header.
    expect(getByTestId('conversation-handle-c1')).toHaveTextContent('@coach-tempo');
    expect(getByTestId('conversation-kind-c4')).toBeTruthy();
  });

  it('badges an unread row with its count and bolds its title', async () => {
    mockGetConversations.mockResolvedValueOnce(
      page([
        makeConv({ id: 'c1', title: 'Training plan', unread_count: 3, last_message: { preview: 'Easy Thursday', role: 'assistant', created_at: '2026-04-13T10:00:00Z' } }),
        makeConv({ id: 'c2', title: 'Read already', unread_count: 0 }),
      ]),
    );

    const { findByTestId, queryByTestId, getByTestId } = render(<ConversationsScreen />);

    expect(await findByTestId('conversation-unread-c1')).toHaveTextContent('3');
    expect(getByTestId('conversation-preview-c1')).toHaveTextContent('Easy Thursday');
    expect(queryByTestId('conversation-unread-c2')).toBeNull();
  });

  it('opens a thread on press and advances the read marker when the row was unread', async () => {
    mockGetConversations.mockResolvedValue(page([makeConv({ id: 'c1', title: 'Training plan', unread_count: 2 })]));
    mockMarkConversationRead.mockResolvedValue(undefined);

    const { findByTestId } = render(<ConversationsScreen />);
    const row = await findByTestId('conversation-row-c1');

    await act(async () => {
      fireEvent.press(row);
    });

    expect(mockPush).toHaveBeenCalledWith(threadHref('c1'));
    expect(mockMarkConversationRead).toHaveBeenCalledWith('c1');
  });

  it('leaves the marker alone when the opened row has nothing unread', async () => {
    mockGetConversations.mockResolvedValue(page([makeConv({ id: 'c1', title: 'Training plan', unread_count: 0 })]));

    const { findByTestId } = render(<ConversationsScreen />);
    await act(async () => {
      fireEvent.press(await findByTestId('conversation-row-c1'));
    });

    expect(mockPush).toHaveBeenCalledWith(threadHref('c1'));
    expect(mockMarkConversationRead).not.toHaveBeenCalled();
  });

  // Turns red if the swipe stops clearing the marker, or the row does not
  // show every message as unread again before the server confirms.
  it('swipe → Mark unread clears the marker and badges the row with its message count', async () => {
    serveMutableList([makeConv({ id: 'c1', title: 'Training plan', message_count: 6, unread_count: 0 })]);

    const { findByTestId, queryByTestId } = render(<ConversationsScreen />);
    await findByTestId('conversation-row-c1');
    expect(queryByTestId('conversation-unread-c1')).toBeNull();

    await act(async () => {
      fireEvent.press(await findByTestId('swipeable-conversation-c1-action-mark-unread'));
    });

    expect(mockMarkConversationUnread).toHaveBeenCalledWith('c1');
    expect(await findByTestId('conversation-unread-c1')).toHaveTextContent('6');
    expect(mockPush).not.toHaveBeenCalled();
  });

  it('swipe → Delete asks first, then deletes and drops the row', async () => {
    serveMutableList([makeConv({ id: 'c1', title: 'Training plan' }), makeConv({ id: 'c2', title: 'Keep me' })]);

    const { findByTestId, queryByTestId } = render(<ConversationsScreen />);
    await findByTestId('conversation-row-c1');

    fireEvent.press(await findByTestId('swipeable-conversation-c1-action-delete'));
    expect(mockDeleteConversation).not.toHaveBeenCalled();

    const confirm = (Alert.alert as jest.Mock).mock.calls.at(-1) as [
      string,
      string,
      Array<{ text: string; onPress?: () => void }>,
    ];
    expect(confirm[0]).toBe('Delete Conversation');
    await act(async () => {
      confirm[2].find((button) => button.text === 'Delete')?.onPress?.();
    });

    expect(mockDeleteConversation).toHaveBeenCalledWith('c1');
    await waitFor(() => expect(queryByTestId('conversation-row-c1')).toBeNull());
    expect(queryByTestId('conversation-row-c2')).toBeTruthy();
  });

  it('long-press offers Rename, Mark unread and Delete', async () => {
    serveMutableList([makeConv({ id: 'c1', title: 'Training plan' })]);

    const { findByTestId, getByTestId } = render(<ConversationsScreen />);
    const row = await findByTestId('conversation-row-c1');

    await act(async () => {
      fireEvent(row, 'longPress');
    });
    expect(getByTestId('conversation-action-rename')).toBeTruthy();
    expect(getByTestId('conversation-action-delete')).toBeTruthy();
    await act(async () => {
      fireEvent.press(getByTestId('conversation-action-mark-unread'));
    });
    expect(mockMarkConversationUnread).toHaveBeenCalledWith('c1');

    await act(async () => {
      fireEvent(await findByTestId('conversation-row-c1'), 'longPress');
    });
    fireEvent.press(getByTestId('conversation-action-rename'));
    fireEvent.changeText(getByTestId('rename-conversation-dialog-input'), 'Renamed');
    await act(async () => {
      fireEvent.press(getByTestId('rename-conversation-dialog-submit'));
    });
    expect(mockUpdateConversation).toHaveBeenCalledWith('c1', { title: 'Renamed' });
    expect(await findByTestId('conversation-title-c1')).toHaveTextContent('Renamed');
  });

  it('pull-to-refresh re-reads the list', async () => {
    mockGetConversations.mockResolvedValue(page([makeConv({ id: 'c1', title: 'Training plan' })]));

    const { findByTestId, getByTestId } = render(<ConversationsScreen />);
    await findByTestId('conversation-row-c1');
    expect(mockGetConversations).toHaveBeenCalledTimes(1);

    await act(async () => {
      fireEvent(getByTestId('conversations-list'), 'refresh');
    });

    await waitFor(() => expect(mockGetConversations).toHaveBeenCalledTimes(2));
  });

  it('filters rows by title, coach handle and preview from the search field', async () => {
    mockGetConversations.mockResolvedValue(
      page([
        makeConv({ id: 'c1', title: 'Training plan', coach_handle: 'coach-tempo' }),
        makeConv({ id: 'c2', title: 'Nutrition', last_message: { preview: 'More carbs on Sunday', role: 'assistant', created_at: '2026-04-13T10:00:00Z' } }),
        makeConv({ id: 'c3', title: 'Sleep' }),
      ]),
    );

    const { findByTestId, getByTestId, queryByTestId, findByText } = render(<ConversationsScreen />);
    await findByTestId('conversation-row-c1');

    fireEvent.changeText(getByTestId('conversation-search-input'), '@tempo');
    await waitFor(() => expect(queryByTestId('conversation-row-c3')).toBeNull());
    expect(getByTestId('conversation-row-c1')).toBeTruthy();
    expect(queryByTestId('conversation-row-c2')).toBeNull();

    fireEvent.changeText(getByTestId('conversation-search-input'), 'carbs');
    await waitFor(() => expect(queryByTestId('conversation-row-c1')).toBeNull());
    expect(getByTestId('conversation-row-c2')).toBeTruthy();

    fireEvent.changeText(getByTestId('conversation-search-input'), 'nothing here');
    // Typographic quotes: the line is a corpus string now, and every locale
    // uses its own pair — « » in French, „ " in German.
    expect(await findByText('No chat matches \u201Cnothing here\u201D')).toBeTruthy();
  });

  // The list is virtualised, so the fifty-first row is off screen by
  // construction; what this pins is that reaching the end asks the server for
  // the next offset exactly once, and stops asking once the total is loaded.
  it('asks for the next page while the server says more rows exist', async () => {
    const first = Array.from({ length: 50 }, (_, i) => makeConv({ id: `c${i}`, title: `Chat ${i}` }));
    mockGetConversations
      .mockResolvedValueOnce({ conversations: first, total: 51, limit: 50, offset: 0 })
      .mockResolvedValueOnce({ conversations: [makeConv({ id: 'c50', title: 'Chat 50' })], total: 51, limit: 50, offset: 50 });

    const { findByTestId, getByTestId } = render(<ConversationsScreen />);
    await findByTestId('conversation-row-c0');

    await act(async () => {
      fireEvent(getByTestId('conversations-list'), 'endReached');
    });
    await waitFor(() => expect(mockGetConversations).toHaveBeenCalledWith(50, 50));

    // Fifty-one of fifty-one rows are loaded: the end of the list is the end.
    await act(async () => {
      fireEvent(getByTestId('conversations-list'), 'endReached');
    });
    expect(mockGetConversations).toHaveBeenCalledTimes(2);
  });

  it('shows the load error with a Retry that re-reads', async () => {
    mockGetConversations.mockRejectedValueOnce(new Error('offline')).mockResolvedValueOnce(page([makeConv({ id: 'c1', title: 'Back' })]));

    const { findByTestId, findByText, getByTestId } = render(<ConversationsScreen />);
    expect(await findByText('offline')).toBeTruthy();

    await act(async () => {
      fireEvent.press(getByTestId('conversations-retry'));
    });

    expect(await findByTestId('conversation-row-c1')).toBeTruthy();
  });
});
