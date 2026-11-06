// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The long-press Retry on a stored reply is offered only when the server says its turn changed nothing
// ABOUTME: It re-sends the question on a fresh budget, so a blocked or writing turn, or a row that does not say, offers none

import React from 'react';
import { fireEvent, render } from '@testing-library/react-native';

import { MessageList } from '../src/screens/chat/MessageList';
import { presentMessageMenu } from '../src/screens/chat/presentMessageMenu';
import type { Message } from '../src/types';

// The menu is its own unit (MessageMenu.test.tsx); here it is a mock that hands
// back what the turn wired into it.
jest.mock('../src/screens/chat/presentMessageMenu', () => ({
  presentMessageMenu: jest.fn(),
}));

const MESSAGE_ID = 'msg-1';

function menuFor(extra: Partial<Message>): { canRetry: boolean } {
  const message: Message = {
    id: MESSAGE_ID,
    role: 'assistant',
    content: 'Here is the reply.',
    created_at: '2026-11-06T14:00:05Z',
    ...extra,
  };
  const screen = render(
    <MessageList
      messages={[message]}
      isLoading={false}
      isSending={false}
      messageFeedback={{}}
      messageFeedbackComment={{}}
      messageBlocks={{}}
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
  fireEvent(screen.getByTestId(`message-turn-${MESSAGE_ID}`), 'longPress');
  const menu = presentMessageMenu as jest.Mock;
  expect(menu).toHaveBeenCalledTimes(1);
  return menu.mock.calls[0][0] as { canRetry: boolean };
}

describe('MessageList retry on a stored reply', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it('is offered when the turn attempted no write', () => {
    expect(menuFor({ turn_attempted_write: false }).canRetry).toBe(true);
  });

  it('is not offered when the turn was blocked or attempted a write', () => {
    expect(menuFor({ turn_attempted_write: true }).canRetry).toBe(false);
  });

  it('is not offered on a row that does not say', () => {
    expect(menuFor({ turn_attempted_write: null }).canRetry).toBe(false);
  });

  it('is not offered on a row from before the field existed', () => {
    expect(menuFor({}).canRetry).toBe(false);
  });
});
