// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Copy and share hand out readable text — the chart named, never the raw ⟦viz:0⟧ marker
// ABOUTME: Pasted into a message to a training partner, the marker is a token that means nothing

import React from 'react';
import { Share } from 'react-native';
import * as Clipboard from 'expo-clipboard';
import { fireEvent, render } from '@testing-library/react-native';

import type { ReplyBlock } from '@pierre/shared-types';
import { MessageList } from '../src/screens/chat/MessageList';
import { presentMessageMenu } from '../src/screens/chat/presentMessageMenu';
import type { Message } from '../src/types';

jest.mock('expo-clipboard', () => ({
  setStringAsync: jest.fn(() => Promise.resolve(true)),
}));

// The actions live behind the turn's long press, presented by the platform's
// own menu. The menu is its own unit (MessageMenu.test.tsx); here it is a
// mock that hands back the callbacks the turn wired into it.
jest.mock('../src/screens/chat/presentMessageMenu', () => ({
  presentMessageMenu: jest.fn(),
}));

type MenuOptions = { onCopy: () => void; onShare: () => void };

const MESSAGE_ID = 'msg-1';
const BEFORE = 'Voici ton vélo d’août — surtout du VTT à Prévost 🚴';
const AFTER = 'Neuf sorties, environ 128 km au total.';

const SCENE_BLOCKS = JSON.stringify([
  {
    kind: 'chart',
    view_box: { x: 0, y: 0, width: 320, height: 180 },
    nodes: [],
    legend: [],
    title: 'Volume hebdomadaire',
    source_tool: 'get_activities',
  },
]);

const REPLY = `${BEFORE}\n\n⟦viz:0⟧\n\n${AFTER}`;

const BLOCKS: ReplyBlock[] = [
  { type: 'prose', text: REPLY },
  { type: 'scene', scene_blocks: SCENE_BLOCKS },
];

const message: Message = {
  id: MESSAGE_ID,
  role: 'assistant',
  content: REPLY,
  created_at: '2026-09-02T10:00:04Z',
};

function renderList() {
  return render(
    <MessageList
      messages={[message]}
      isLoading={false}
      isSending={false}
      messageFeedback={{}}
      messageFeedbackComment={{}}
      messageBlocks={{ [MESSAGE_ID]: BLOCKS }}
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

/** Long-press the turn and read back the callbacks it handed the menu. */
function openMenu(screen: ReturnType<typeof renderList>): MenuOptions {
  fireEvent(screen.getByTestId(`message-turn-${MESSAGE_ID}`), 'longPress');
  const menu = presentMessageMenu as jest.Mock;
  expect(menu).toHaveBeenCalledTimes(1);
  return menu.mock.calls[0][0] as MenuOptions;
}

describe('MessageList copy and share', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it('copies the reply with the chart named, not with its marker', () => {
    const { onCopy } = openMenu(renderList());

    onCopy();

    expect(Clipboard.setStringAsync).toHaveBeenCalledTimes(1);
    const copied = (Clipboard.setStringAsync as jest.Mock).mock.calls[0][0] as string;

    expect(copied).not.toContain('⟦');
    expect(copied).not.toContain('⟧');
    expect(copied).toContain('[Chart: Volume hebdomadaire]');
    expect(copied).toBe(`${BEFORE}\n\n[Chart: Volume hebdomadaire]\n\n${AFTER}`);
  });

  it('shares the same readable text', () => {
    const share = jest.spyOn(Share, 'share').mockResolvedValue({ action: 'sharedAction' });
    const { onShare } = openMenu(renderList());

    onShare();

    expect(share).toHaveBeenCalledTimes(1);
    const shared = (share.mock.calls[0][0] as { message: string }).message;
    expect(shared).not.toContain('⟦');
    expect(shared).toBe(`${BEFORE}\n\n[Chart: Volume hebdomadaire]\n\n${AFTER}`);

    share.mockRestore();
  });
});
