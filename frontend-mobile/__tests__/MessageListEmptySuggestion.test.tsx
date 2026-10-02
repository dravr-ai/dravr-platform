// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Renders the empty thread to pin its one suggested question — a route for today's session
// ABOUTME: Red if the suggestion stops reaching the host, or shows for a host with no composer to fill

import React from 'react';
import { fireEvent, render } from '@testing-library/react-native';

jest.mock('@expo/vector-icons', () => {
  const View = require('react-native').View;
  return {
    Ionicons: (props: Record<string, unknown>) =>
      require('react').createElement(View, { testID: `icon-${props.name}` }),
  };
});

import { MessageList } from '../src/screens/chat/MessageList';

function renderEmpty(onSuggestRoute?: () => void) {
  return render(
    <MessageList
      messages={[]}
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
      onSuggestRoute={onSuggestRoute}
    />,
  );
}

describe('MessageList empty thread suggestion', () => {
  it('suggests a route for today\'s session and asks the host to draft it when pressed', () => {
    const onSuggestRoute = jest.fn();
    const { getByTestId, getByText } = renderEmpty(onSuggestRoute);

    expect(getByText("Route for today's session")).toBeTruthy();
    fireEvent.press(getByTestId('chat-empty-route'));

    expect(onSuggestRoute).toHaveBeenCalledTimes(1);
  });

  it('keeps the empty thread to its one line for a host that cannot draft', () => {
    const { getByTestId, queryByTestId } = renderEmpty();

    expect(getByTestId('chat-empty-state')).toBeTruthy();
    expect(queryByTestId('chat-empty-route')).toBeNull();
  });
});
