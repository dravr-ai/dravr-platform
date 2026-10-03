// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: An agent's welcome read back from history draws its starters as buttons that send them, and no model menu
// ABOUTME: carnet#735 — the row carries its controls in `actions`, with no block list from a live turn

import React from 'react';
import { fireEvent, render } from '@testing-library/react-native';

import { MessageList } from '../src/screens/chat/MessageList';
import { presentMessageMenu } from '../src/screens/chat/presentMessageMenu';
import type { Message } from '../src/types';

jest.mock('../src/screens/chat/presentMessageMenu', () => ({
  presentMessageMenu: jest.fn(),
}));

const STARTERS = [
  'Que manger avant une course à 6 h du matin ?',
  'Comment faire ma charge glucidique pour un marathon ?',
  'Combien de gels pour un marathon ?',
];

const welcome: Message = {
  id: 'welcome-1',
  role: 'assistant',
  content: 'Bonjour ! Agent Ravitaillement de Dravr, à ton écoute.\n\nSpécialiste du ravitaillement.',
  finish_reason: 'agent_welcome',
  created_at: '2026-10-02T10:00:00Z',
  actions: {
    title: 'Pour commencer, tu peux me demander :',
    actions: STARTERS.map((q) => ({ label: q, action_type: 'postback', value: q })),
  },
};

describe('MessageList agent welcome', () => {
  it('draws the welcome with its starters and sends the one pressed', () => {
    const onActionClick = jest.fn();
    const screen = render(
      <MessageList
        messages={[welcome]}
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
        onActionClick={onActionClick}
      />,
    );

    expect(screen.getByText(/Agent Ravitaillement de Dravr/)).toBeTruthy();
    expect(screen.getByText('Pour commencer, tu peux me demander :')).toBeTruthy();
    // Each starter is a button a screen reader announces by its question.
    for (const q of STARTERS) {
      expect(screen.getByRole('button', { name: q })).toBeTruthy();
    }
    fireEvent.press(screen.getByRole('button', { name: STARTERS[2] }));
    expect(onActionClick).toHaveBeenCalledWith({
      label: STARTERS[2],
      action_type: 'postback',
      value: STARTERS[2],
    });
  });

  it('long-pressing the welcome offers a menu with no rating, share or retry', () => {
    // No model wrote it: retry re-sent the binding command, a rating filed
    // feedback against a platform row.
    const screen = render(
      <MessageList
        messages={[welcome]}
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
        onActionClick={jest.fn()}
      />,
    );

    fireEvent(screen.getByTestId(`message-turn-${welcome.id}`), 'longPress');
    const menu = presentMessageMenu as jest.Mock;
    expect(menu).toHaveBeenCalledTimes(1);
    expect(menu.mock.calls[0][0]).toMatchObject({ fromModel: false });
  });
});
