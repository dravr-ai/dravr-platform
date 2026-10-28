// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A coach's correction in a group thread — the coach's words under the reply they correct (carnet#847)
// ABOUTME: Pins that the athlete it concerns never reads it as their own bubble or as a reply to rate

import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import MessageItem from '../MessageItem';
import type { Message } from '../types';

const corrects = { entry_id: 'entry-reply', message_id: 'msg-reply', excerpt: 'Your 2:41 marathon in May' };

function correction(own: boolean, correctorName: string | null = null): Message {
  return {
    id: 'room-entry-9',
    role: 'assistant',
    content: 'Your May race was a half marathon.',
    created_at: '2026-10-09T12:00:00Z',
    room: {
      speaker: 'correction',
      author_user_id: 'user-alex',
      author_name: 'Alex',
      own,
      withheld: false,
      corrects,
      corrector_name: correctorName,
    },
  };
}

describe('a coach\'s correction in a group thread', () => {
  it('reads as the coach\'s on the athlete\'s own thread, under the reply it corrects, with nothing to rate', () => {
    const onThumbsUp = vi.fn();
    render(<MessageItem message={correction(true)} onThumbsUp={onThumbsUp} onCopy={vi.fn()} />);

    expect(screen.getByText("Your human coach's correction")).toBeInTheDocument();
    expect(screen.getByTestId('room-correction-quote')).toHaveTextContent('On: Your 2:41 marathon in May');
    expect(screen.getByTestId('room-entry-correction')).toHaveTextContent('Your May race was a half marathon.');
    // Not the athlete's own words, and not a reply of the agent's to rate.
    expect(screen.queryByText('You')).not.toBeInTheDocument();
    expect(screen.queryByRole('group', { name: /actions/i })).not.toBeInTheDocument();
  });

  it('names the athlete it concerns on another member\'s thread', () => {
    render(<MessageItem message={correction(false)} />);

    expect(screen.getByText("Human coach's correction for Alex")).toBeInTheDocument();
    expect(screen.getByTestId('room-entry-correction')).toHaveTextContent('Your May race was a half marathon.');
  });

  // The bubble did not say which coach wrote it (carnet#1191): the entry names
  // the coach who wrote it (carnet#1211), and the bubble names them on both
  // sides of the room.
  it('names the coach who wrote it on the athlete\'s own thread', () => {
    render(<MessageItem message={correction(true, 'Camille')} />);

    expect(screen.getByText('Correction from Camille, your human coach')).toBeInTheDocument();
    expect(screen.queryByText("Your human coach's correction")).not.toBeInTheDocument();
  });

  it('names the coach and the athlete it concerns on another member\'s thread', () => {
    render(<MessageItem message={correction(false, 'Camille')} />);

    expect(screen.getByText('Correction from Camille for Alex')).toBeInTheDocument();
  });
});
