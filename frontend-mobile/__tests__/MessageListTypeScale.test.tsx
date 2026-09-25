// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A coach reply's markdown renders at steps of the phone's one type ladder, the class ladder of DESIGN.md §10
// ABOUTME: Prose at `base`, headings at `xl` and `lg`, inline code, indented code and table cells at the interface step `sm`

import React from 'react';
import { StyleSheet } from 'react-native';
import { render } from '@testing-library/react-native';
import { MessageList } from '../src/screens/chat/MessageList';
import type { Message } from '../src/types';

const REPLY = [
  '# Semaine de récupération',
  '',
  '## Mardi',
  '',
  'Footing en zone `Z2` pendant quarante minutes.',
  '',
  '    rpe 3',
  '',
  '| Jour | Séance |',
  '| --- | --- |',
  '| Jeudi | Repos |',
].join('\n');

const assistantMessage: Message = {
  id: 'msg-1',
  role: 'assistant',
  content: REPLY,
  created_at: '2026-09-25T08:00:00Z',
};

function renderList() {
  return render(
    <MessageList
      messages={[assistantMessage]}
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
    />,
  );
}

/** The size and leading a rendered string finally takes, after every inherited style. */
function textMetrics(screen: ReturnType<typeof renderList>, text: string) {
  const style = StyleSheet.flatten(screen.getByText(text).props.style) as {
    fontSize?: number;
    lineHeight?: number;
  };
  return { fontSize: style.fontSize, lineHeight: style.lineHeight };
}

describe('MessageList markdown on the class ladder', () => {
  it('sets the prose at the reading step, 16 with a 1.5 leading', () => {
    const screen = renderList();
    expect(textMetrics(screen, 'Footing en zone ')).toEqual({ fontSize: 16, lineHeight: 24 });
  });

  it('sets the first heading at xl (20) and the second at lg (17)', () => {
    const screen = renderList();
    expect(textMetrics(screen, 'Semaine de récupération').fontSize).toBe(20);
    expect(textMetrics(screen, 'Mardi').fontSize).toBe(17);
  });

  it('sets inline code, an indented code block and table cells at the interface step sm (13)', () => {
    const screen = renderList();
    expect(textMetrics(screen, 'Z2').fontSize).toBe(13);
    expect(textMetrics(screen, 'rpe 3\n').fontSize).toBe(13);
    expect(textMetrics(screen, 'Jour').fontSize).toBe(13);
    expect(textMetrics(screen, 'Repos').fontSize).toBe(13);
  });
});
