// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Renders a group thread's room rows on mobile — another member's words, the agent's reply, a withheld placeholder, a coach's private rows
// ABOUTME: Pins attribution by name, the placeholder in place of a silent gap, no menu on a row that is not the caller's, and the private lane's marks

import React from 'react';
import { fireEvent, render } from '@testing-library/react-native';
import { composeRoomThread } from '@pierre/chat-utils';
import type { ChatMessageAction, GroupTranscriptEntry, ReplyBlock } from '@pierre/shared-types';

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

import { MessageList, type MessageListPrivateLane } from '../src/screens/chat/MessageList';
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

function renderThread(
  messages: Message[],
  extra: {
    privateLane?: MessageListPrivateLane;
    messageBlocks?: Record<string, ReplyBlock[]>;
    onActionClick?: (action: ChatMessageAction, audience: 'room' | 'private') => void;
  } = {},
) {
  return render(
    <MessageList
      messages={messages}
      isLoading={false}
      isSending={false}
      messageFeedback={{}}
      messageFeedbackComment={{}}
      messageBlocks={extra.messageBlocks}
      flatListRef={React.createRef()}
      onScrollToBottom={jest.fn()}
      onThumbsUp={jest.fn()}
      onThumbsDown={jest.fn()}
      onSubmitFeedbackReason={jest.fn()}
      onRetryMessage={jest.fn()}
      onOpenUrl={jest.fn()}
      onReconnectProvider={jest.fn()}
      onActionClick={extra.onActionClick}
      privateLane={extra.privateLane}
    />,
  );
}

/**
 * The coach's work thread about J-D, the room's one athlete (carnet#1249):
 * a question quoting J-D's room line and the agent's reply, then — after J-D
 * speaks again in the room — a second question.
 */
const PRIVATE: Message[] = [
  {
    id: 'w1',
    role: 'user',
    content: 'Make Saturday easy instead.',
    created_at: '2026-09-14T11:10:00Z',
    quote: {
      entry_id: 't2',
      author_user_id: 'user-jd',
      author_display_name: 'J-D',
      content: '@coach is Saturday a tempo?',
      created_at: '2026-09-14T11:00:00Z',
    },
  },
  { id: 'w2', role: 'assistant', content: 'Done. Saturday is now easy.', created_at: '2026-09-14T11:10:05Z' },
  { id: 'w3', role: 'user', content: 'And Sunday?', created_at: '2026-09-14T11:30:00Z' },
];

const LATER_ROOM_LINE: GroupTranscriptEntry = {
  id: 't5',
  speaker: 'member',
  withheld: false,
  own: false,
  author_user_id: 'user-jd',
  author_display_name: 'J-D',
  content: 'Thanks!',
  message_id: 'jd-3',
  created_at: '2026-09-14T11:20:00Z',
};

function lane(overrides: Partial<MessageListPrivateLane> = {}): MessageListPrivateLane {
  return {
    conversationId: 'conv-work-jd',
    agentName: 'Dravr',
    canAdjustFrom: (row) => row.room?.speaker === 'member' && row.room.author_user_id === 'user-jd',
    onAdjustFrom: jest.fn(),
    turnLive: false,
    ...overrides,
  };
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
    expect(placeholder).toHaveTextContent("Hidden message: this member keeps their messages private through their sharing settings", {
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

  it('draws the coach\'s correction on the athlete\'s own thread as the coach\'s, under the reply it corrects', () => {
    // carnet#847: the correction concerns the caller (own), yet it is the
    // coach speaking — never the caller's own bubble, never a reply to rate.
    const correction: GroupTranscriptEntry = {
      id: 't10',
      speaker: 'correction',
      withheld: false,
      own: true,
      author_user_id: 'user-me',
      author_display_name: 'Me',
      content: 'That week was 52 km, not 62.',
      message_id: null,
      corrects: { entry_id: 't-own-reply', message_id: 'm2', excerpt: 'Solid week, 62 km.' },
      created_at: '2026-09-14T12:30:00Z',
    };
    const { getByTestId, getByText, queryByTestId } = renderThread(
      composeRoomThread(OWN, [...ROOM, correction]),
    );

    expect(getByTestId('room-entry-correction')).toBeTruthy();
    expect(getByText("Your human coach's correction")).toBeTruthy();
    expect(getByTestId('room-correction-quote')).toHaveTextContent('On: Solid week, 62 km.');
    expect(getByText('That week was 52 km, not 62.')).toBeTruthy();
    expect(queryByTestId('message-turn-room-t10')).toBeNull();
  });

  // The correction bubble did not say which coach wrote it (carnet#1191); it
  // names the coach the entry says wrote it, not the group's current one
  // (carnet#1211).
  it('names the coach who wrote the correction', () => {
    const correction: GroupTranscriptEntry = {
      id: 't11',
      speaker: 'correction',
      withheld: false,
      own: true,
      author_user_id: 'user-me',
      author_display_name: 'Me',
      content: 'That week was 52 km, not 62.',
      message_id: null,
      corrects: { entry_id: 't-own-reply', message_id: 'm2', excerpt: 'Solid week, 62 km.' },
      corrector: { user_id: 'coach-camille', display_name: 'Camille' },
      created_at: '2026-09-14T12:30:00Z',
    };
    const { getByText, queryByText } = renderThread(composeRoomThread(OWN, [...ROOM, correction]));

    expect(getByText('Correction from Camille, your human coach')).toBeTruthy();
    expect(queryByText("Your human coach's correction")).toBeNull();
  });
});

// carnet#1249: a coach's private turns, drawn inline in the room they concern.
describe('MessageList private lane in a one-athlete room', () => {
  beforeEach(() => {
    (presentMessageMenu as jest.Mock).mockClear();
  });

  it('draws the divider over each stretch of private rows, and never over a room row', () => {
    // A short room, so every row of it is inside the list's first window.
    const { getAllByTestId } = renderThread(composeRoomThread([], [ROOM[1], LATER_ROOM_LINE], PRIVATE), {
      privateLane: lane(),
    });

    // Two stretches — w1 and w2, then w3 after J-D's later line — so two dividers.
    const dividers = getAllByTestId('private-lane-divider');
    expect(dividers).toHaveLength(2);
    expect(dividers[0]).toHaveTextContent(/Only you and Dravr see what follows/);
    expect(dividers[0].props.accessibilityLabel).toBe('Only you and Dravr see what follows');
  });

  it("dashes the coach's private bubble, shows the quote it carried, and says who can see it", () => {
    const { getAllByTestId, getByText } = renderThread(composeRoomThread(OWN, ROOM, PRIVATE.slice(0, 2)), {
      privateLane: lane(),
    });

    const bubble = getAllByTestId('private-bubble')[0];
    expect(bubble.props.className).toContain('border-dashed');
    expect(bubble.props.className).toContain('border-warning/60');
    expect(getByText('Make Saturday easy instead.')).toBeTruthy();
    expect(getAllByTestId('private-row-quote')[0]).toHaveTextContent(/From J-D's message/);
    expect(getAllByTestId('private-bubble-lock')[0].props.accessibilityLabel).toBe(
      'Only you and Dravr can see this message',
    );
  });

  it('tags the agent\'s private reply "Only you", spoken as only the coach and the agent seeing it', () => {
    const { getByTestId } = renderThread(composeRoomThread(OWN, ROOM, PRIVATE.slice(0, 2)), {
      privateLane: lane(),
    });

    const tag = getByTestId('private-reply-tag');
    expect(tag).toHaveTextContent('Only you');
    expect(tag.props.accessibilityLabel).toBe('Only you and Dravr can see this message');
    expect(getByTestId('private-row-reply')).toHaveTextContent(/Done\. Saturday is now easy\./);
  });

  it('offers copy alone on a private reply: rating and retry act on the room conversation', () => {
    const { getByTestId } = renderThread(composeRoomThread(OWN, ROOM, PRIVATE.slice(0, 2)), {
      privateLane: lane(),
    });

    fireEvent(getByTestId('message-turn-w2'), 'longPress');
    const menu = presentMessageMenu as jest.Mock;
    expect(menu).toHaveBeenCalledTimes(1);
    const [options] = menu.mock.calls[0];
    expect(options).toMatchObject({ fromModel: false, canRetry: false });
    expect(options.onRate).toBeUndefined();
    expect(options.onRetry).toBeUndefined();
    expect(options.onAdjustPlan).toBeUndefined();
  });

  it('answers a control on a private reply in private', () => {
    const onActionClick = jest.fn();
    const keep: ChatMessageAction = { label: 'Keep Sunday', action_type: 'postback', value: 'ex:0:1' };
    const { getByText } = renderThread(composeRoomThread(OWN, ROOM, PRIVATE.slice(0, 2)), {
      privateLane: lane(),
      onActionClick,
      messageBlocks: { w2: [{ type: 'prose', text: 'Done.' }, { type: 'actions', actions: [keep] }] },
    });

    fireEvent.press(getByText('Keep Sunday'));
    expect(onActionClick).toHaveBeenCalledWith(keep, 'private');
  });

  it('offers "Adjust plan from this" on the athlete\'s room line, and quotes that line', () => {
    const onAdjustFrom = jest.fn();
    const { getByTestId, queryByTestId } = renderThread(composeRoomThread(OWN, ROOM), {
      privateLane: lane({ onAdjustFrom }),
    });

    // The agent's reply to the athlete offers no such thing.
    expect(queryByTestId('message-turn-room-t3')).toBeNull();
    fireEvent(getByTestId('message-turn-room-t2'), 'longPress');

    const menu = presentMessageMenu as jest.Mock;
    expect(menu).toHaveBeenCalledTimes(1);
    const [options] = menu.mock.calls[0];
    expect(options.fromModel).toBe(false);
    options.onAdjustPlan();
    expect(onAdjustFrom).toHaveBeenCalledTimes(1);
    expect(onAdjustFrom.mock.calls[0][0]).toMatchObject({ id: 'room-t2', content: '@coach is Saturday a tempo?' });
  });

  it('reaches "Adjust plan from this" from a screen reader\'s actions too', () => {
    const onAdjustFrom = jest.fn();
    const { getByTestId } = renderThread(composeRoomThread(OWN, ROOM), { privateLane: lane({ onAdjustFrom }) });

    const row = getByTestId('message-turn-room-t2');
    expect(row.props.accessibilityActions).toEqual([{ name: 'adjustPlan', label: 'Adjust plan from this' }]);
    fireEvent(row, 'accessibilityAction', { nativeEvent: { actionName: 'adjustPlan' } });
    expect(onAdjustFrom).toHaveBeenCalledTimes(1);
  });

  it('gives a member\'s line no long press without a private lane', () => {
    const { queryByTestId } = renderThread(composeRoomThread(OWN, ROOM));
    expect(queryByTestId('message-turn-room-t2')).toBeNull();
  });

  it('streams a private turn\'s words under "Only you"', () => {
    const { getByTestId } = render(
      <MessageList
        messages={composeRoomThread(OWN, ROOM, PRIVATE.slice(0, 1))}
        isLoading={false}
        isSending
        streamingContent="Moving Saturday…"
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
        privateLane={lane({ turnLive: true })}
      />,
    );

    expect(getByTestId('streaming-reply')).toHaveTextContent(/Only you/);
  });
});

