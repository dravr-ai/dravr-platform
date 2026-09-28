// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Renders a group thread's room rows on mobile — another member's words, the agent's reply to them, a withheld placeholder
// ABOUTME: Pins attribution by name, the placeholder text in place of a silent gap, and no menu on a row that is not the caller's

import React from 'react';
import { fireEvent, render } from '@testing-library/react-native';
import { composeRoomThread } from '@pierre/chat-utils';
import type { GroupTranscriptEntry } from '@pierre/shared-types';

jest.mock('@expo/vector-icons', () => {
  const View = require('react-native').View;
  return {
    Ionicons: (props: Record<string, unknown>) =>
      require('react').createElement(View, { testID: `icon-${props.name}` }),
  };
});

// The turn's actions sit behind a long press, presented by the platform's own
// menu (its own unit, MessageMenu.test.tsx); here a mock records whether a
// long press presents it at all.
jest.mock('../src/screens/chat/presentMessageMenu', () => ({
  presentMessageMenu: jest.fn(),
}));

import { MessageList } from '../src/screens/chat/MessageList';
import { presentMessageMenu } from '../src/screens/chat/presentMessageMenu';
import type { Message } from '../src/types';

const OWN: Message[] = [
  { id: 'm1', role: 'user', content: 'How was my week?', created_at: '2026-09-14T10:00:00Z' },
  { id: 'm2', role: 'assistant', content: 'Solid week, 62 km.', created_at: '2026-09-14T10:00:05Z' },
];

const ROOM: GroupTranscriptEntry[] = [
  {
    id: 't1',
    speaker: 'member',
    withheld: false,
    own: true,
    author_user_id: 'user-me',
    author_display_name: 'Me',
    content: 'How was my week?',
    message_id: 'm1',
    created_at: '2026-09-14T10:00:00.2Z',
  },
  {
    id: 't2',
    speaker: 'member',
    withheld: false,
    own: false,
    author_user_id: 'user-jd',
    author_display_name: 'J-D',
    content: '@coach is Saturday a tempo?',
    message_id: 'jd-1',
    created_at: '2026-09-14T11:00:00Z',
  },
  {
    id: 't3',
    speaker: 'coach',
    withheld: false,
    own: false,
    author_user_id: 'user-jd',
    author_display_name: 'J-D',
    content: 'Make it an easy 90 minutes, J-D.',
    message_id: 'jd-2',
    created_at: '2026-09-14T11:00:04Z',
  },
  {
    id: 't4',
    speaker: 'member',
    withheld: true,
    own: false,
    author_user_id: null,
    author_display_name: null,
    content: null,
    message_id: null,
    created_at: '2026-09-14T11:05:00Z',
  },
];

function renderThread(messages: Message[]) {
  return render(
    <MessageList
      messages={messages}
      isLoading={false}
      isSending={false}
      messageFeedback={{}}
      messageFeedbackComment={{}}
      flatListRef={React.createRef()}
      onScrollToBottom={jest.fn()}
      onThumbsUp={jest.fn()}
      onThumbsDown={jest.fn()}
      onSubmitFeedbackReason={jest.fn()}
      onRetryMessage={jest.fn()}
      onOpenUrl={jest.fn()}
      onReconnectProvider={jest.fn()}
    />,
  );
}

describe('MessageList group thread room', () => {
  it('shows another member\'s line under their name and the coach\'s reply to them, once each', () => {
    const { getByTestId, getByText, getAllByText } = renderThread(composeRoomThread(OWN, ROOM));

    const member = getByTestId('room-entry-member');
    expect(getByText('J-D')).toBeTruthy();
    expect(getByText('@coach is Saturday a tempo?')).toBeTruthy();
    expect(member).toBeTruthy();

    expect(getByTestId('room-entry-coach')).toBeTruthy();
    expect(getByText('to J-D')).toBeTruthy();
    expect(getByText('Make it an easy 90 minutes, J-D.')).toBeTruthy();

    // The caller's own question comes from their conversation, drawn once.
    expect(getAllByText('How was my week?')).toHaveLength(1);
  });

  it('keeps a withheld entry as a placeholder naming no one', () => {
    const { getByTestId } = renderThread(composeRoomThread(OWN, ROOM));

    const placeholder = getByTestId('room-entry-withheld');
    expect(placeholder).toHaveTextContent("A member's message is hidden: sharing not enabled", {
      exact: false,
    });
  });

  it('gives the coach\'s reply to another member no menu, and keeps the caller\'s own reply\'s', () => {
    const { queryByTestId } = renderThread(composeRoomThread(OWN, ROOM));

    // The caller's own reply is long-pressable for copy, rating and retry.
    expect(queryByTestId('message-turn-m2')).toBeTruthy();
    // The room's reply to J-D is not a row of the caller's conversation.
    expect(queryByTestId('message-turn-room-t3')).toBeNull();
  });

  it('gives the caller\'s own reply from another of their threads no menu to rate or retry it by', () => {
    // The room holds a reply to the caller that another thread of theirs in
    // this group holds, not this conversation: it has no chat row here to
    // rate or retry, so a long press offers nothing.
    const elsewhere: GroupTranscriptEntry = {
      id: 't9',
      speaker: 'coach',
      withheld: false,
      own: true,
      author_user_id: 'user-me',
      author_display_name: 'Me',
      content: 'Answered in your other thread.',
      message_id: 'other-thread-row',
      created_at: '2026-09-14T12:00:00Z',
    };
    const { getByTestId, getByText } = renderThread(composeRoomThread(OWN, [...ROOM, elsewhere]));

    expect(getByText('Answered in your other thread.')).toBeTruthy();
    const menu = presentMessageMenu as jest.Mock;
    menu.mockClear();
    fireEvent(getByTestId('message-turn-room-t9'), 'longPress');
    expect(menu).not.toHaveBeenCalled();
    // The caller's reply in this conversation keeps its menu.
    fireEvent(getByTestId('message-turn-m2'), 'longPress');
    expect(menu).toHaveBeenCalledTimes(1);
  });
});
