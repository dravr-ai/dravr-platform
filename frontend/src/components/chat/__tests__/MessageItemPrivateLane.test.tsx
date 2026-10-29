// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A coach's private rows in a one-athlete room, and "Adjust plan from this" on the athlete's room message (carnet#1249)
// ABOUTME: Pins the dashed private bubble with the quote it carries, the "Only you" tag, the screen-reader label and the action's reach

import { describe, it, expect, vi } from 'vitest';
import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import MessageItem from '../MessageItem';
import type { Message } from '../types';

const AGENT = 'Dravr';
const PRIVATE_ARIA = 'Only you and Dravr can see this message';

/** The coach's private question, quoting the athlete's room message. */
const privateQuestion: Message = {
  id: 'msg-work-1',
  role: 'user',
  content: "Move Thursday's threshold run to Saturday and keep Thursday easy.",
  created_at: '2026-10-29T20:40:00Z',
  audience: 'private',
  quote: {
    entry_id: 'entry-camille-1',
    author_user_id: 'user-camille',
    author_display_name: 'Camille Roy',
    content: "Legs felt heavy in this morning's swim. Is Thursday's threshold run still a good idea?",
    created_at: '2026-10-29T07:12:00Z',
  },
};

/** The agent's private reply. */
const privateReply: Message = {
  id: 'msg-work-2',
  role: 'assistant',
  content: 'Done. Two days change; the other five stay as they are.',
  created_at: '2026-10-29T20:41:00Z',
  audience: 'private',
};

/** The athlete's own words in the room. */
const athleteLine: Message = {
  id: 'room-entry-camille-1',
  role: 'user',
  content: "Legs felt heavy in this morning's swim. Is Thursday's threshold run still a good idea?",
  created_at: '2026-10-29T07:12:00Z',
  room: { speaker: 'member', author_user_id: 'user-camille', author_name: 'Camille Roy', own: false, withheld: false },
};

describe("a coach's private rows in the room", () => {
  it("draws the coach's private question dashed, says who can see it, and carries the quoted message", () => {
    render(<MessageItem message={privateQuestion} assistantLabel={AGENT} timestamp="20:40" />);

    const row = screen.getByRole('group', { name: PRIVATE_ARIA });
    expect(row).toHaveAttribute('data-audience', 'private');
    expect(row.querySelector('.chat-bubble-private')).not.toBeNull();
    expect(row.querySelector('.chat-bubble-user')).toBeNull();
    const quote = within(row).getByTestId('message-quote');
    expect(quote).toHaveTextContent("From Camille Roy's message");
    expect(quote).toHaveTextContent("Legs felt heavy in this morning's swim.");
    expect(row).toHaveTextContent("Move Thursday's threshold run to Saturday");
  });

  it("tags the agent's private reply \"Only you\" behind a dashed rule, and names it for a screen reader", () => {
    render(<MessageItem message={privateReply} assistantLabel={AGENT} timestamp="20:41" onCopy={vi.fn()} />);

    const row = screen.getByRole('group', { name: PRIVATE_ARIA });
    expect(within(row).getByTestId('private-reply-tag')).toHaveTextContent('Only you');
    expect(within(row).getByTestId('message-column')).toHaveClass('chat-column-private');
    expect(row).toHaveTextContent('Done. Two days change');
  });

  it('draws a row the room reads with none of it', () => {
    const roomReply: Message = { ...privateReply, id: 'msg-room-2', audience: undefined };
    render(<MessageItem message={roomReply} assistantLabel={AGENT} timestamp="20:41" />);

    expect(screen.queryByRole('group', { name: PRIVATE_ARIA })).toBeNull();
    expect(screen.queryByTestId('private-reply-tag')).toBeNull();
    expect(screen.getByTestId('message-row')).toHaveAttribute('data-audience', 'room');
    expect(screen.getByTestId('message-column')).not.toHaveClass('chat-column-private');
  });
});

describe('Adjust plan from this', () => {
  it("offers the action on the athlete's room message, from the keyboard too", async () => {
    const onAdjustPlan = vi.fn();
    const user = userEvent.setup();
    render(<MessageItem message={athleteLine} assistantLabel={AGENT} onAdjustPlan={onAdjustPlan} />);

    // Hidden until hover or focus, never out of the tab order.
    const action = screen.getByRole('button', { name: 'Adjust plan from this' });
    await user.tab();
    expect(action).toHaveFocus();
    await user.keyboard('{Enter}');
    expect(onAdjustPlan).toHaveBeenCalledTimes(1);

    await user.click(action);
    expect(onAdjustPlan).toHaveBeenCalledTimes(2);
  });

  it('offers nothing on a room message it cannot quote', () => {
    render(<MessageItem message={athleteLine} assistantLabel={AGENT} />);

    expect(screen.getByTestId('room-entry-member')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Adjust plan from this' })).toBeNull();
  });
});
