// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Renders MessageList to pin the rule a new topic opens with in the personal thread (carnet#840)
// ABOUTME: "+" and /reset divide the one thread; the divider row reads as a rule with its time, never as a bubble

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

/** A stamp today at a given local wall-clock time, so the clock reads the same in every timezone. */
function today(hour: number, minute: number): string {
  const date = new Date();
  date.setHours(hour, minute, 0, 0);
  return date.toISOString();
}

const MESSAGES: Message[] = [
  { id: 'm1', role: 'user', content: 'How was my week?', created_at: today(9, 0) },
  { id: 'm2', role: 'assistant', content: 'Solid: 62 km.', created_at: today(9, 1) },
  { id: 'd1', role: 'system', content: '', finish_reason: 'divider', created_at: today(9, 30) },
  { id: 'm3', role: 'user', content: 'Plan my tempo week', created_at: today(9, 31) },
];

function renderList(messages: Message[] = MESSAGES) {
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

describe('MessageList topic divider', () => {
  it('draws a new topic as a rule naming it and the time it opened', () => {
    const { getAllByTestId, getByTestId } = renderList();

    expect(getAllByTestId('topic-divider')).toHaveLength(1);
    expect(getByTestId('topic-divider').props.accessibilityLabel).toBe('New chat · 09:30');
  });

  it('never draws the divider row as a message', () => {
    const { getAllByTestId } = renderList();

    // Three rows were written by people or the coach; the divider is not one of them.
    expect(getAllByTestId('message-time')).toHaveLength(3);
  });
});
