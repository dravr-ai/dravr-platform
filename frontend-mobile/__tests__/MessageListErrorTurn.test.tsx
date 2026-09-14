// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A failed turn is one line — the message in the error ink and an inline retry, no slab, no chip
// ABOUTME: Pins the retry's testID and the path it calls, and that the clock still sits on the row

import React from 'react';
import { fireEvent, render } from '@testing-library/react-native';

import { MessageList } from '../src/screens/chat/MessageList';
import type { Message } from '../src/types';

const ERROR_ID = 'error-1';
const ERROR_TEXT = '⚠️ Network request failed\n\nPlease try again.';

const MESSAGES: Message[] = [
  { id: 'user-1', role: 'user', content: 'Comment était ma semaine ?', created_at: '2026-09-02T10:00:00Z' },
  { id: ERROR_ID, role: 'assistant', content: ERROR_TEXT, created_at: '2026-09-02T10:00:04Z', isError: true },
];

function renderList(onRetryMessage = jest.fn()) {
  return render(
    <MessageList
      messages={MESSAGES}
      isLoading={false}
      isSending={false}
      messageFeedback={{}}
      messageFeedbackComment={{}}
      flatListRef={React.createRef()}
      onScrollToBottom={jest.fn()}
      onThumbsUp={jest.fn()}
      onThumbsDown={jest.fn()}
      onSubmitFeedbackReason={jest.fn()}
      onRetryMessage={onRetryMessage}
      onOpenUrl={jest.fn()}
      onReconnectProvider={jest.fn()}
    />,
  );
}

/** Every className in a subtree, one string per node that carries one. */
function classNames(node: { props: { className?: string }; children?: unknown[] }): string[] {
  const own = typeof node.props?.className === 'string' ? [node.props.className] : [];
  const children = (node.children ?? []).filter(
    (child): child is { props: { className?: string }; children?: unknown[] } =>
      typeof child === 'object' && child !== null && 'props' in child,
  );
  return own.concat(...children.map(classNames));
}

describe('MessageList error turn', () => {
  it('shows the message in the error ink with an inline retry', () => {
    const { getByText, getByTestId } = renderList();

    const line = getByText(ERROR_TEXT);
    expect(line.props.className).toContain('text-error');
    expect(line.props.className).toContain('text-sm');

    const retry = getByTestId('message-retry');
    expect(retry.props.accessibilityRole).toBe('button');
    expect(retry.props.className).toContain('text-primary');
    expect(retry.props.className).toContain('font-medium');
    expect(retry.props.children).toBe('Retry');
  });

  it('pressing retry re-sends that message', () => {
    const onRetryMessage = jest.fn();
    const { getByTestId } = renderList(onRetryMessage);

    fireEvent.press(getByTestId('message-retry'));

    expect(onRetryMessage).toHaveBeenCalledTimes(1);
    expect(onRetryMessage).toHaveBeenCalledWith(ERROR_ID);
  });

  it('draws no tinted slab and no filled chip around the error', () => {
    const { getAllByTestId } = renderList();

    // The error is the second row — the reply that changed author.
    const rows = getAllByTestId('message-row-start');
    const errorRow = rows[rows.length - 1];
    const classes = classNames(errorRow as never);

    expect(classes.length).toBeGreaterThan(0);
    for (const className of classes) {
      expect(className).not.toContain('bg-error/10');
      expect(className).not.toContain('border-error');
      expect(className).not.toContain('bg-background-tertiary');
    }
  });

  it('still says when the error arrived, and offers no long-press menu on it', () => {
    const { getAllByTestId, queryByTestId } = renderList();

    // The athlete's clock and the error row's clock: two, in order.
    expect(getAllByTestId('message-time')).toHaveLength(2);
    expect(queryByTestId(`message-turn-${ERROR_ID}`)).toBeNull();
  });
});
