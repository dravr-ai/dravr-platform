// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A URL the coach writes is tappable on the phone, and a reconnect goes to the auth session
// ABOUTME: Markdown autolinks nothing, so the address printed as running text had nothing to tap

import React from 'react';
import { fireEvent, render } from '@testing-library/react-native';

import type { ReplyBlock } from '@pierre/shared-types';
import { MessageList } from '../src/screens/chat/MessageList';
import type { Message } from '../src/types';

const MESSAGE_ID = 'msg-1';
const LINK = 'https://app.dravr.ai/providers/garmin/reconnect';
// getFriendlyUrlName drops the scheme and truncates the path at 20 characters;
// markdown-it's typographer then folds its trailing "..." into one ellipsis.
const FRIENDLY_LINK_TEXT = 'app.dravr.ai/providers/garmin/re\u2026';

function assistantMessage(content: string): Message {
  return {
    id: MESSAGE_ID,
    role: 'assistant',
    content,
    created_at: '2026-09-02T10:00:04Z',
  };
}

function renderList(props: {
  message: Message;
  blocks?: ReplyBlock[];
  onOpenUrl?: (url: string) => void;
  onReconnectProvider?: (provider: string) => void;
}) {
  return render(
    <MessageList
      bottomInset={0}
      messages={[props.message]}
      isLoading={false}
      isSending={false}
      messageFeedback={{}}
      messageFeedbackComment={{}}
      messageBlocks={props.blocks ? { [MESSAGE_ID]: props.blocks } : undefined}
      flatListRef={React.createRef()}
      onScrollToBottom={jest.fn()}
      onThumbsUp={jest.fn()}
      onThumbsDown={jest.fn()}
      onSubmitFeedbackReason={jest.fn()}
      onRetryMessage={jest.fn()}
      onOpenUrl={props.onOpenUrl ?? jest.fn()}
      onReconnectProvider={props.onReconnectProvider ?? jest.fn()}
    />,
  );
}

describe('MessageList links in coach prose', () => {
  it('makes a bare URL a tappable link that opens the address it names', () => {
    const onOpenUrl = jest.fn();
    const { getByText } = renderList({
      message: assistantMessage(`Reconnecte-toi ici : ${LINK} puis reviens me voir.`),
      onOpenUrl,
    });

    // The raw address is replaced by its friendly name — and it is a node with
    // a press handler, which running text has none of.
    const link = getByText(FRIENDLY_LINK_TEXT);
    fireEvent.press(link);

    expect(onOpenUrl).toHaveBeenCalledWith(LINK);
  });

  it('opens a link inside the activity list too', () => {
    const onOpenUrl = jest.fn();
    const { getByText } = renderList({
      message: assistantMessage('Tes sorties.'),
      blocks: [
        { type: 'prose', text: 'Voici tes sorties.' },
        {
          type: 'activity_list',
          text: `1. Sortie VTT — [Prévost](${LINK})`,
        },
      ],
      onOpenUrl,
    });

    // The list is collapsed until the athlete opens it.
    fireEvent.press(getByText('Your Activities (1)'));
    // The list writes its own markdown links, so the label is the coach's, not
    // a friendly name — what was missing was any handler behind it.
    fireEvent.press(getByText('Prévost'));

    expect(onOpenUrl).toHaveBeenCalledWith(LINK);
  });
});

describe('MessageList reconnect action', () => {
  const RECONNECT_BLOCKS: ReplyBlock[] = [
    { type: 'prose', text: 'Ta connexion Garmin est expirée.' },
    {
      type: 'reconnect',
      provider: 'garmin',
      display_name: 'Garmin',
      url: 'https://app.dravr.ai/providers/garmin/connect?token=one-time',
      text: 'Reconnecte Garmin pour continuer.',
    },
  ];

  it('routes the reconnect to the provider flow, never to the generic opener', () => {
    const onOpenUrl = jest.fn();
    const onReconnectProvider = jest.fn();
    const { getByText } = renderList({
      message: assistantMessage('Ta connexion Garmin est expirée.'),
      blocks: RECONNECT_BLOCKS,
      onOpenUrl,
      onReconnectProvider,
    });

    fireEvent.press(getByText('Reconnect Garmin'));

    // The provider, not the block's URL: that URL was minted for a browser
    // callback, and handing it to the system browser sends the athlete to
    // Safari with nothing to return them to the app.
    expect(onReconnectProvider).toHaveBeenCalledWith('garmin');
    expect(onOpenUrl).not.toHaveBeenCalled();
  });

  it('still sends an ordinary link to the generic opener', () => {
    const onOpenUrl = jest.fn();
    const onReconnectProvider = jest.fn();
    const { getByText } = renderList({
      message: assistantMessage(`Regarde ${LINK} quand tu peux.`),
      onOpenUrl,
      onReconnectProvider,
    });

    fireEvent.press(getByText(FRIENDLY_LINK_TEXT));

    expect(onOpenUrl).toHaveBeenCalledWith(LINK);
    expect(onReconnectProvider).not.toHaveBeenCalled();
  });
});
