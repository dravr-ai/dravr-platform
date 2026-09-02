// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Renders MessageList to pin the verdict chip's count, its press handler and its spoken label
// ABOUTME: The rows are the count once they exist; the turn's chips only preview them until then

import React from 'react';
import { fireEvent, render } from '@testing-library/react-native';

// The icon under test is the glyph's name, so render each Ionicon as a view
// carrying it — the vector font itself draws nothing a test can read.
jest.mock('@expo/vector-icons', () => {
  const View = require('react-native').View;
  return {
    Ionicons: (props: Record<string, unknown>) =>
      require('react').createElement(View, { testID: `icon-${props.name}` }),
  };
});

import type { ClaimVerdict, ReplyBlock } from '@pierre/shared-types';
import { MessageList } from '../src/screens/chat/MessageList';
import type { Message } from '../src/types';

const MESSAGE_ID = 'msg-1';

const assistantMessage: Message = {
  id: MESSAGE_ID,
  role: 'assistant',
  content: 'Ton VO2max est de 82.',
  created_at: '2026-08-22T10:00:04Z',
};

// The live turn's chips, spelled in the coach's French.
const CHIP_BLOCKS: ReplyBlock[] = [
  { type: 'prose', text: 'Ton VO2max est de 82.' },
  {
    type: 'verdicts',
    chips: [
      { claim: 'Ton VO2max est de 82.', contradicted: true },
      { claim: 'Six heures de sommeil suffisent.', contradicted: false },
    ],
  },
];

// The verdict row the read returned for the same reply — the verifier wrote
// the claim in its own words, so no chip matches it by text.
const ROW: ClaimVerdict = {
  id: 'verdict-1',
  conversation_id: 'conv-1',
  message_id: MESSAGE_ID,
  coach_id: 'coach-tempo',
  claim_text: 'Your VO2max is 82.',
  category: 'physiological',
  status: 'contradicted',
  evidence_strength: 'none',
  confidence: 0.91,
  layer_fired: 'deterministic',
  explanation: null,
  evidence_refs: null,
  created_at: '2026-08-22T10:00:05Z',
};

function renderList(props: {
  messageBlocks?: Record<string, ReplyBlock[]>;
  verdicts?: ClaimVerdict[];
  onShowVerdict?: (rows: ClaimVerdict[], messageId: string) => void;
}) {
  return render(
    <MessageList
      bottomInset={0}
      messages={[assistantMessage]}
      isLoading={false}
      isSending={false}
      messageFeedback={{}}
      messageFeedbackComment={{}}
      messageBlocks={props.messageBlocks}
      verdicts={props.verdicts}
      flatListRef={React.createRef()}
      onScrollToBottom={jest.fn()}
      onThumbsUp={jest.fn()}
      onThumbsDown={jest.fn()}
      onSubmitFeedbackReason={jest.fn()}
      onRetryMessage={jest.fn()}
      onOpenUrl={jest.fn()}
      onReconnectProvider={jest.fn()}
      onShowVerdict={props.onShowVerdict}
    />,
  );
}

describe('MessageList verdict chip', () => {
  it('counts the rows alone once they exist, even when the chips spell the claim differently', () => {
    const { getByText, queryByText } = renderList({
      messageBlocks: { [MESSAGE_ID]: CHIP_BLOCKS },
      verdicts: [ROW],
    });

    expect(getByText('1 verdict · contradicted')).toBeTruthy();
    expect(queryByText(/3 verdicts/)).toBeNull();
  });

  it('draws the plain shield when every claim was supported, the alert shield when one was contradicted', () => {
    const supported = renderList({
      messageBlocks: { [MESSAGE_ID]: CHIP_BLOCKS },
      verdicts: [{ ...ROW, status: 'supported', evidence_strength: 'strong' }],
    });
    expect(supported.getByText('1 verdict · supported')).toBeTruthy();
    expect(supported.getByTestId('icon-shield-outline')).toBeTruthy();
    expect(supported.queryByTestId('icon-shield-half-outline')).toBeNull();
    supported.unmount();

    const contradicted = renderList({
      messageBlocks: { [MESSAGE_ID]: CHIP_BLOCKS },
      verdicts: [ROW],
    });
    expect(contradicted.getByTestId('icon-shield-half-outline')).toBeTruthy();
    expect(contradicted.queryByTestId('icon-shield-outline')).toBeNull();
  });

  it('previews the count from the chips while no row has landed', () => {
    const { getByText } = renderList({ messageBlocks: { [MESSAGE_ID]: CHIP_BLOCKS } });

    expect(getByText('2 verdicts · contradicted')).toBeTruthy();
  });

  it('opens the verdicts on press with the rows it has and the message they belong to', () => {
    const onShowVerdict = jest.fn();
    const { getByTestId } = renderList({
      messageBlocks: { [MESSAGE_ID]: CHIP_BLOCKS },
      verdicts: [ROW],
      onShowVerdict,
    });

    fireEvent.press(getByTestId('verdict-chip'));

    expect(onShowVerdict).toHaveBeenCalledTimes(1);
    expect(onShowVerdict).toHaveBeenCalledWith([ROW], MESSAGE_ID);
  });

  it('speaks the worst status in the athlete\'s language', () => {
    const { getByLabelText } = renderList({
      messageBlocks: { [MESSAGE_ID]: CHIP_BLOCKS },
      verdicts: [ROW],
    });

    expect(getByLabelText('Claim verdicts: contradicted')).toBeTruthy();
  });
});
