// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Renders the empty thread to pin its three use-case links — the starters ranked for the athlete, in rank order
// ABOUTME: Red if a link stops handing the host its starter, or shows for a host with nothing to send it through

import React from 'react';
import { fireEvent, render } from '@testing-library/react-native';
import type { UseCaseStarter } from '@pierre/shared-types';

jest.mock('@expo/vector-icons', () => {
  const View = require('react-native').View;
  return {
    Ionicons: (props: Record<string, unknown>) =>
      require('react').createElement(View, { testID: `icon-${props.name}` }),
  };
});

import { MessageList } from '../src/screens/chat/MessageList';

const STARTERS: UseCaseStarter[] = [
  { id: 'connect', label: 'Connect my watch', postback: 'uc:0:connect' },
  { id: 'about_me', label: 'Get to know me', postback: 'uc:1:about_me' },
  { id: 'can_see', label: 'What can you see?', postback: 'uc:2:can_see' },
];

function renderEmpty(onStarter?: (starter: UseCaseStarter) => void, starters = STARTERS) {
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
      starters={starters}
      onStarter={onStarter}
    />,
  );
}

describe('MessageList empty thread starters', () => {
  it('draws the three ranked starters as links, in rank order, and hands the host the one pressed', () => {
    const onStarter = jest.fn();
    const { getAllByTestId, getByTestId } = renderEmpty(onStarter);

    const links = getAllByTestId(/^chat-empty-starter-/);
    expect(links.map((link) => link.props.testID)).toEqual([
      'chat-empty-starter-connect',
      'chat-empty-starter-about_me',
      'chat-empty-starter-can_see',
    ]);
    fireEvent.press(getByTestId('chat-empty-starter-about_me'));

    expect(onStarter).toHaveBeenCalledTimes(1);
    expect(onStarter).toHaveBeenCalledWith(STARTERS[1]);
  });

  it('shows each starter by its label, never its postback', () => {
    const { getByText, queryByText } = renderEmpty(jest.fn());
    for (const starter of STARTERS) {
      expect(getByText(starter.label)).toBeTruthy();
      expect(queryByText(starter.postback)).toBeNull();
    }
  });

  it('keeps the empty thread to its line and the slash hint for a host that cannot send a starter', () => {
    const { getByTestId, queryAllByTestId } = renderEmpty();

    expect(getByTestId('chat-empty-state')).toBeTruthy();
    expect(getByTestId('chat-slash-hint')).toBeTruthy();
    expect(queryAllByTestId(/^chat-empty-starter-/)).toHaveLength(0);
  });

  it('draws no link while the starters load or when none fits', () => {
    const { queryAllByTestId } = renderEmpty(jest.fn(), []);
    expect(queryAllByTestId(/^chat-empty-starter-/)).toHaveLength(0);
  });
});
