// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A coach's Chat list on the phone holds one row per trifecta athlete — their room and the plan thread on them — opening their workspace (carnet#1817, #1818)
// ABOUTME: Pins the fold for a roster coach only, the row's person shape and clipboard, no row menu, and every other reader's one row per conversation

import React from 'react';
import { fireEvent, render, screen, within } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';

const mockGetConversations = jest.fn();
const mockMarkConversationRead = jest.fn();
const mockPush = jest.fn();
let mockManagesRoster = true;

jest.mock('../src/services/api', () => ({
  chatApi: {
    getConversations: (...args: unknown[]) => mockGetConversations(...args),
    markConversationRead: (...args: unknown[]) => mockMarkConversationRead(...args),
    updateConversation: jest.fn(),
    deleteConversation: jest.fn(),
    markConversationUnread: jest.fn(),
    listParticipants: jest.fn().mockResolvedValue([]),
  },
  notificationsApi: { getUnreadCount: jest.fn().mockResolvedValue({ unread_count: 0 }) },
  userApi: { getCoachAccessRequest: jest.fn().mockResolvedValue({ request: null }) },
  groupsApi: { getGroup: jest.fn() },
}));
jest.mock('../src/contexts/AuthContext', () => ({
  useAuth: () => ({ user: { id: 'user-coach', manages_roster: mockManagesRoster }, isAuthenticated: true }),
}));
jest.mock('expo-router', () =>
  require('../jest.expo-router').createExpoRouterMock({
    useRouter: () => ({ push: mockPush, back: jest.fn(), navigate: jest.fn(), replace: jest.fn() }),
  }),
);

import { ConversationsScreen } from '../src/screens/conversations/ConversationsScreen';
import { coachWorkspaceHref, threadHref } from '../src/navigation/routes';

const LEA = { user_id: 'u-lea', display_name: 'Léa Gagnon' };

/** The coach's room with Léa alone: the server names her on it to her coach. */
const ROOM = {
  id: 'conv-room-lea',
  title: 'TRAKA',
  group_id: 'g-lea',
  group_name: 'TRAKA',
  message_count: 3,
  unread_count: 2,
  last_message: { preview: 'See you Saturday', role: 'user' as const, created_at: '2026-11-25T09:00:00Z' },
  solo_athlete: LEA,
  created_at: '2026-10-01T00:00:00Z',
  updated_at: '2026-11-25T09:00:00Z',
};
/** The coach's plan thread on Léa, which spoke last. */
const PLAN_THREAD = {
  id: 'conv-plan-lea',
  title: 'Léa Gagnon',
  subject_user_id: LEA.user_id,
  subject_group_id: 'g-lea',
  subject_display_name: LEA.display_name,
  message_count: 2,
  unread_count: 1,
  last_message: { preview: 'Draft saved for two dates', role: 'assistant' as const, created_at: '2026-11-25T10:05:00Z' },
  solo_athlete: LEA,
  created_at: '2026-11-14T00:00:00Z',
  updated_at: '2026-11-25T10:05:00Z',
};
/** A room of several athletes, which no fold touches. */
const SQUAD = {
  id: 'conv-squad',
  title: 'Sunday Riders',
  group_id: 'g-squad',
  group_name: 'Sunday Riders',
  message_count: 9,
  unread_count: 0,
  created_at: '2026-09-01T00:00:00Z',
  updated_at: '2026-11-24T08:00:00Z',
};

function renderList() {
  mockGetConversations.mockResolvedValue({ conversations: [ROOM, PLAN_THREAD, SQUAD], total: 3, limit: 50, offset: 0 });
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 }, mutations: { retry: false, gcTime: 0 } },
  });
  return render(
    <QueryClientProvider client={client}>
      <ConversationsScreen />
    </QueryClientProvider>,
  );
}

const rowIds = () => screen.getAllByTestId(/^conversation-row-/).map((node) => String(node.props.testID));

beforeEach(() => {
  jest.clearAllMocks();
  mockManagesRoster = true;
});

describe("a roster coach's Chat list", () => {
  it("holds one row for a trifecta athlete's room and plan thread, named after the athlete", async () => {
    renderList();
    await screen.findByTestId('conversation-row-conv-room-lea');

    // The athlete's place keeps the room's id, which a notification or a link names.
    expect(rowIds()).toEqual(['conversation-row-conv-room-lea', 'conversation-row-conv-squad']);
    expect(screen.getByTestId('conversation-title-conv-room-lea')).toHaveTextContent('Léa Gagnon');
    // The last words of whichever thread spoke last, and both threads' unread.
    expect(screen.getByTestId('conversation-preview-conv-room-lea')).toHaveTextContent('Draft saved for two dates');
    expect(screen.getByTestId('conversation-unread-text-conv-room-lea')).toHaveTextContent('3');
  });

  it("draws the athlete as a person with the plan thread's clipboard", async () => {
    renderList();
    const row = within(await screen.findByTestId('conversation-row-conv-room-lea'));

    expect(row.getByTestId('conversation-opens-workspace-conv-room-lea')).toHaveProp('accessibilityLabel', 'Athlete plan');
    expect(screen.queryByTestId('conversation-kind-conv-room-lea')).toBeNull();
    // A circle is one person, a square many (DESIGN.md "Shapes").
    const avatar = screen.getByTestId('conversation-avatar-conv-room-lea', { includeHiddenElements: true });
    expect(avatar.props.style).toEqual(expect.objectContaining({ width: 48, height: 48, borderRadius: 24 }));
    expect(avatar).toHaveTextContent('LG');
    // A room of several athletes stays a square with the group glyph.
    expect(screen.getByTestId('conversation-kind-conv-squad')).toHaveProp('accessibilityLabel', 'Group chat');
    const squad = screen.getByTestId('conversation-avatar-conv-squad', { includeHiddenElements: true });
    expect(squad.props.style).toEqual(expect.objectContaining({ borderRadius: 12 }));
  });

  it("opens the athlete's workspace, not a thread, and leaves the read markers to the threads it shows", async () => {
    renderList();
    fireEvent.press(await screen.findByTestId('conversation-row-conv-room-lea'));

    expect(mockPush).toHaveBeenCalledWith(coachWorkspaceHref(LEA.user_id));
    expect(mockPush).not.toHaveBeenCalledWith(threadHref('conv-room-lea'));
    expect(mockMarkConversationRead).not.toHaveBeenCalled();
  });

  it('offers no row menu on the athlete, which stands for two conversations', async () => {
    renderList();
    await screen.findByTestId('conversation-row-conv-room-lea');
    // The touchable itself: the test renderer would hand a long-press on a row
    // with no handler of its own to the row component's own props.
    const { TouchableOpacity } = require('react-native');
    const touchable = (testID: string) =>
      screen.UNSAFE_getAllByType(TouchableOpacity).find((node) => node.props.testID === testID);

    expect(touchable('conversation-row-conv-room-lea')?.props.onLongPress).toBeUndefined();
    expect(touchable('conversation-row-conv-squad')?.props.onLongPress).toEqual(expect.any(Function));
    // Nor does it swipe to Mark unread or Delete; a room of several athletes does.
    expect(within(screen.getByTestId('swipeable-conversation-conv-room-lea')).queryByText('Mark unread')).toBeNull();
    expect(within(screen.getByTestId('swipeable-conversation-conv-squad')).getByText('Mark unread')).toBeTruthy();
  });
});

describe('every other reader', () => {
  it('keeps one row per conversation, each opening its own thread', async () => {
    mockManagesRoster = false;
    renderList();
    await screen.findByTestId('conversation-row-conv-room-lea');

    expect(rowIds()).toEqual(['conversation-row-conv-plan-lea', 'conversation-row-conv-room-lea', 'conversation-row-conv-squad']);
    expect(screen.getByTestId('conversation-title-conv-room-lea')).toHaveTextContent('TRAKA');
    expect(screen.queryByTestId(/^conversation-opens-workspace-/)).toBeNull();

    fireEvent.press(screen.getByTestId('conversation-row-conv-room-lea'));
    expect(mockPush).toHaveBeenCalledWith(threadHref('conv-room-lea'));
  });
});
