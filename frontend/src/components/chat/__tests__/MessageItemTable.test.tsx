// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests that a coach table renders and stays scrollable in a message
// ABOUTME: The prose typography plugin squashes tables unless the wrapper wins

import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';
import MessageItem from '../MessageItem';
import type { Message } from '../types';

// A coach reply mixing prose with a table wider than a narrow browser window —
// the shape the contremaitre Tables contract now permits on web and mobile.
const REPLY = [
  'Ta charge grimpe depuis trois semaines.',
  '',
  '| Day | Session | Distance | Pace | Elevation |',
  '| --- | --- | --- | --- | --- |',
  '| Tuesday | Threshold | 12 km | 4:35/km | 120 m |',
  '| Sunday | Long run | 24 km | 5:15/km | 460 m |',
].join('\n');

const tableMessage: Message = {
  id: 'msg-table',
  role: 'assistant',
  content: REPLY,
  created_at: new Date().toISOString(),
};

describe('MessageItem tables', () => {
  it('renders every cell of a wide table', () => {
    render(<MessageItem message={tableMessage} />);

    // The rightmost column is what a squashed or clipped table loses first.
    expect(screen.getByText('Elevation')).toBeInTheDocument();
    expect(screen.getByText('460 m')).toBeInTheDocument();
    expect(screen.getByText('4:35/km')).toBeInTheDocument();
  });

  it('wraps the table in a horizontally scrollable container', () => {
    render(<MessageItem message={tableMessage} />);

    const wrapper = screen.getByTestId('markdown-table-scroll');
    expect(wrapper).toHaveClass('overflow-x-auto');

    // `!w-max` is what lets the table exceed the bubble; without it the
    // typography plugin's `width: 100%` squashes the columns instead.
    const table = wrapper.querySelector('table');
    expect(table).toHaveClass('!w-max');
  });

  it('does not wrap a reply that has no table', () => {
    render(
      <MessageItem
        message={{ ...tableMessage, id: 'msg-prose', content: 'Repose-toi aujourd’hui.' }}
      />
    );

    expect(screen.getByText('Repose-toi aujourd’hui.')).toBeInTheDocument();
    expect(screen.queryByTestId('markdown-table-scroll')).toBeNull();
  });

  it('still opens links in a new tab', () => {
    render(
      <MessageItem
        message={{
          ...tableMessage,
          id: 'msg-link',
          content: 'Voir [Strava](https://www.strava.com/athletes/1).',
        }}
      />
    );

    const link = screen.getByRole('link', { name: 'Strava' });
    expect(link).toHaveAttribute('target', '_blank');
    expect(link).toHaveAttribute('rel', 'noopener noreferrer');
  });
});
