// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A failed turn's stored notice offers copy only on long press — no retry, rating or share
// ABOUTME: carnet#1363 — the notice already says to send the question again, so the menu must not offer retry a second time

import React from 'react';
import { ActionSheetIOS, Platform } from 'react-native';
import { fireEvent, render } from '@testing-library/react-native';

import { MessageList } from '../src/screens/chat/MessageList';
import type { Message } from '../src/types';

type SheetOptions = { options: string[]; cancelButtonIndex?: number };

const question: Message = {
  id: 'question-1',
  role: 'user',
  content: 'How was my long run?',
  created_at: '2026-11-03T10:00:00Z',
};

// The row the platform writes when a turn fails after its question was stored:
// the localized `messaging.turn_interrupted` notice, no model call.
const notice: Message = {
  id: 'notice-1',
  role: 'assistant',
  content: "My reply was cut off before it reached you. Send your question again and I'll pick it back up.",
  finish_reason: 'interrupted',
  created_at: '2026-11-03T10:00:05Z',
};

describe('MessageList failed-turn notice', () => {
  const originalOS = Platform.OS;

  beforeEach(() => {
    (Platform as { OS: string }).OS = 'ios';
    jest.spyOn(ActionSheetIOS, 'showActionSheetWithOptions').mockImplementation(() => undefined);
  });

  afterEach(() => {
    (Platform as { OS: string }).OS = originalOS;
    jest.restoreAllMocks();
  });

  it('long-pressing the notice offers copy and cancel only', () => {
    // A Retry here re-sent the question beside the notice's own "send it
    // again" — retry offered twice, and a duplicate user row — and a rating
    // filed feedback against a row no model wrote.
    const onRetryMessage = jest.fn();
    const screen = render(
      <MessageList
        messages={[question, notice]}
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
        onRetryMessage={onRetryMessage}
        onOpenUrl={jest.fn()}
        onReconnectProvider={jest.fn()}
        onActionClick={jest.fn()}
      />,
    );

    fireEvent(screen.getByTestId(`message-turn-${notice.id}`), 'longPress');
    const sheet = ActionSheetIOS.showActionSheetWithOptions as unknown as jest.Mock;
    expect(sheet).toHaveBeenCalledTimes(1);
    const [options] = sheet.mock.calls[0] as [SheetOptions];
    expect(options.options).toEqual(['Copy', 'Cancel']);
    expect(onRetryMessage).not.toHaveBeenCalled();
  });
});
