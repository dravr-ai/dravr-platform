// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Renders MessageList itself to prove a coach table reaches the screen
// ABOUTME: Complements MarkdownTableScroll, which covers the render rule alone

import React from 'react';
import { render } from '@testing-library/react-native';
import { MessageList } from '../src/screens/chat/MessageList';
import type { Message } from '../src/types';

// A coach reply mixing prose with a table wider than a phone — the shape the
// contremaitre Tables contract now permits on web and mobile.
const REPLY = [
  'Ta charge grimpe depuis trois semaines.',
  '',
  '| Day | Session | Distance | Pace | Elevation |',
  '| --- | --- | --- | --- | --- |',
  '| Tuesday | Threshold | 12 km | 4:35/km | 120 m |',
  '| Sunday | Long run | 24 km | 5:15/km | 460 m |',
  '',
  "C'est pourquoi on coupe jeudi.",
].join('\n');

// react-native-markdown-display constructs markdown-it with `typographer: true`,
// so a straight apostrophe in the source is rendered as a typographic one.
// Assertions below match what the user actually sees, not what the coach wrote.
const RENDERED_TRAILING_PROSE = 'C’est pourquoi on coupe jeudi.';

const assistantMessage: Message = {
  id: 'msg-1',
  role: 'assistant',
  content: REPLY,
  created_at: '2026-08-13T12:00:00Z',
};

function renderList(messages: Message[]) {
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
    />
  );
}

describe('MessageList table rendering', () => {
  it('renders the prose and every table cell of a coach reply', () => {
    const { getByText } = renderList([assistantMessage]);

    // Prose on both sides of the table survives extraction.
    expect(getByText('Ta charge grimpe depuis trois semaines.')).toBeTruthy();
    expect(getByText(RENDERED_TRAILING_PROSE)).toBeTruthy();

    // The rightmost column is the one a clipped table would lose.
    expect(getByText('Elevation')).toBeTruthy();
    expect(getByText('460 m')).toBeTruthy();
  });

  it('wraps the table in the horizontal scroller', () => {
    const { getByTestId } = renderList([assistantMessage]);

    const scroller = getByTestId('markdown-table-scroll');
    expect(scroller.props.horizontal).toBe(true);
  });

  it('leaves a table-free reply unwrapped', () => {
    const { queryByTestId, getByText } = renderList([
      { ...assistantMessage, id: 'msg-2', content: 'Repose-toi aujourd’hui.' },
    ]);

    expect(getByText('Repose-toi aujourd’hui.')).toBeTruthy();
    expect(queryByTestId('markdown-table-scroll')).toBeNull();
  });
});
