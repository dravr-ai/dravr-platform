// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Renders MessageList to pin the day pill, the bubble clock and the run boundary on mobile
// ABOUTME: The web thread has drawn all three since the messenger cutover; this is the parity that was missing

import React from 'react';
import { render } from '@testing-library/react-native';

jest.mock('@expo/vector-icons', () => {
  const View = require('react-native').View;
  return {
    Ionicons: (props: Record<string, unknown>) =>
      require('react').createElement(View, { testID: `icon-${props.name}` }),
  };
});

import { MessageList } from '../src/screens/chat/MessageList';
import type { Message } from '../src/types';

/**
 * A stamp `daysAgo` days back at a given local wall-clock time.
 *
 * Built from the real clock rather than a frozen date so the test says the
 * same thing in every timezone: what is asserted is the relationship between
 * the rows, not a particular calendar day.
 */
function at(daysAgo: number, hour: number, minute: number): string {
  const date = new Date();
  date.setDate(date.getDate() - daysAgo);
  date.setHours(hour, minute, 0, 0);
  return date.toISOString();
}

const pad = (n: number) => String(n).padStart(2, '0');

/**
 * Four rows over two days: yesterday's exchange, then two of the athlete's
 * own messages this morning a minute apart — one run, not two.
 */
const MESSAGES: Message[] = [
  { id: 'm1', role: 'user', content: 'Comment était ma semaine ?', created_at: at(1, 9, 5) },
  { id: 'm2', role: 'assistant', content: 'Solide — 62 km.', created_at: at(1, 9, 6) },
  { id: 'm3', role: 'user', content: 'Et aujourd’hui ?', created_at: at(0, 7, 12) },
  { id: 'm4', role: 'user', content: 'Repos ou seuil ?', created_at: at(0, 7, 13) },
];

function renderList(messages: Message[] = MESSAGES) {
  return render(
    <MessageList
      bottomInset={0}
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

describe('MessageList day pill, clock and grouping', () => {
  it('puts a pill above each day and names today and yesterday in words', () => {
    const { getAllByTestId, getByText } = renderList();

    expect(getAllByTestId('day-separator')).toHaveLength(2);
    expect(getByText('Yesterday')).toBeTruthy();
    expect(getByText('Today')).toBeTruthy();
  });

  it('shows every row its own 24-hour clock', () => {
    const { getAllByTestId } = renderList();

    const clocks = getAllByTestId('message-time').map((node) => node.props.children);
    expect(clocks).toEqual([
      `${pad(9)}:${pad(5)}`,
      `${pad(9)}:${pad(6)}`,
      `${pad(7)}:${pad(12)}`,
      `${pad(7)}:${pad(13)}`,
    ]);
  });

  it('draws two of one author’s messages a minute apart as one run', () => {
    const { getAllByTestId, queryAllByTestId } = renderList();

    // Three rows open a run — the question, the reply that changes author,
    // and this morning's first line. Only the athlete's second message a
    // minute later continues one.
    expect(getAllByTestId('message-row-start')).toHaveLength(3);
    expect(getAllByTestId('message-row-continued')).toHaveLength(1);

    // Six minutes apart is outside the window, so the same pair opens two runs.
    const apart = renderList([MESSAGES[2], { ...MESSAGES[3], created_at: at(0, 7, 19) }]);
    expect(apart.getAllByTestId('message-row-start')).toHaveLength(2);
    expect(apart.queryAllByTestId('message-row-continued')).toHaveLength(0);
    expect(queryAllByTestId('message-row-continued')).toHaveLength(1);
  });

  it('says nothing about a day it cannot read', () => {
    const { queryAllByTestId } = renderList([
      { id: 'bad', role: 'user', content: 'sans date', created_at: 'not-a-date' },
    ]);

    expect(queryAllByTestId('day-separator')).toHaveLength(0);
    expect(queryAllByTestId('message-time')).toHaveLength(0);
  });
});
