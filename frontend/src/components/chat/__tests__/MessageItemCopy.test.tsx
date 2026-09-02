// ABOUTME: Copy and share hand out readable text — the chart named, never the raw ⟦viz:0⟧ marker
// ABOUTME: Pasted into a message to a training partner, the marker is a token that means nothing
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { ReplyBlock } from '@pierre/shared-types';
import MessageItem from '../MessageItem';
import type { Message } from '../types';

const BEFORE = 'Voici ton vélo d’août — surtout du VTT à Prévost 🚴';
const AFTER = 'Neuf sorties, environ 128 km au total.';
const REPLY = `${BEFORE}\n\n⟦viz:0⟧\n\n${AFTER}`;

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

const BLOCKS: ReplyBlock[] = [
  { type: 'prose', text: REPLY },
  { type: 'scene', scene_blocks: SCENE_BLOCKS },
];

const message: Message = {
  id: 'msg-1',
  role: 'assistant',
  content: REPLY,
  created_at: '2026-09-02T10:00:00Z',
};

const READABLE = `${BEFORE}\n\n[Chart: Volume hebdomadaire]\n\n${AFTER}`;

describe('MessageItem copy and share', () => {
  it('copies the reply with the chart named, not with its marker', async () => {
    const user = userEvent.setup();
    const onCopy = vi.fn();

    render(<MessageItem message={message} blocks={BLOCKS} onCopy={onCopy} />);
    await user.click(screen.getByTitle('Copy message'));

    expect(onCopy).toHaveBeenCalledTimes(1);
    const copied = onCopy.mock.calls[0][0] as string;
    expect(copied).not.toContain('⟦');
    expect(copied).not.toContain('⟧');
    expect(copied).toContain('[Chart: Volume hebdomadaire]');
    expect(copied).toBe(READABLE);
  });

  it('shares the same readable text', async () => {
    const user = userEvent.setup();
    const onShare = vi.fn();

    render(<MessageItem message={message} blocks={BLOCKS} onShare={onShare} />);
    await user.click(screen.getByTitle('Share'));

    expect(onShare).toHaveBeenCalledWith(READABLE);
  });

  it('leaves a marker-free reply exactly as the coach wrote it', async () => {
    const user = userEvent.setup();
    const onCopy = vi.fn();
    const plain = { ...message, content: `${BEFORE}\n\n${AFTER}` };

    render(
      <MessageItem
        message={plain}
        blocks={[{ type: 'prose', text: plain.content }]}
        onCopy={onCopy}
      />,
    );
    await user.click(screen.getByTitle('Copy message'));

    expect(onCopy).toHaveBeenCalledWith(`${BEFORE}\n\n${AFTER}`);
  });
});
