// ABOUTME: PHASE 6 tests — MessageItem paints the server's reply blocks through one switch
// ABOUTME: Red the moment a renderer goes back to scraping a URL, a panel or a control out of the prose
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { ReplyBlock } from '@pierre/shared-types';
import MessageItem from '../MessageItem';
import type { Message } from '../types';

const RECONNECT_URL = 'https://app.dravr.ai/providers/sciotte/login?token=one-time-abc';

function assistantMessage(content = 'Your load is climbing.'): Message {
  return {
    id: 'msg-1',
    role: 'assistant',
    content,
    created_at: '2026-08-24T10:00:00Z',
  };
}

describe('MessageItem reply-block switch', () => {
  it('renders the reconnect call to action from the block, not from a URL in the prose', () => {
    // The regression this turns red: the deleted
    // `/https?:\/\/\S*\/providers\/sciotte\/login\?token=\S+/` scrape coming
    // back. The prose here carries NO url at all — on a surface that renders a
    // reconnect control the server does not fold the sentence in — so a
    // regex-driven renderer produces no button and this fails.
    const blocks: ReplyBlock[] = [
      { type: 'prose', text: 'I need you to reconnect before I can read that.' },
      {
        type: 'reconnect',
        provider: 'whoop',
        display_name: 'WHOOP',
        url: RECONNECT_URL,
        text: 'Reconnect WHOOP to continue.',
      },
    ];

    render(<MessageItem message={assistantMessage()} blocks={blocks} />);

    const cta = screen.getByRole('link', { name: /Reconnect WHOOP/ });
    expect(cta).toHaveAttribute('href', RECONNECT_URL);
    expect(cta).toHaveAttribute('rel', 'noopener noreferrer');
    expect(screen.getByText('I need you to reconnect before I can read that.')).toBeInTheDocument();
    // The raw token URL is never printed as text beside the control.
    expect(screen.queryByText(RECONNECT_URL)).not.toBeInTheDocument();
  });

  it('renders the controls the actions block carried, with its own group title', async () => {
    const user = userEvent.setup();
    const onActionClick = vi.fn();
    const blocks: ReplyBlock[] = [
      { type: 'prose', text: 'Which session do you want?' },
      {
        type: 'actions',
        title: 'Pick a session',
        actions: [
          { label: 'Seuil 3x10', action_type: 'postback', value: '/plan session seuil' },
          { label: 'Open Strava', action_type: 'url', value: 'https://www.strava.com/athlete' },
        ],
      },
    ];

    render(<MessageItem message={assistantMessage()} blocks={blocks} onActionClick={onActionClick} />);

    expect(screen.getByText('Pick a session')).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Open Strava' }));
    expect(onActionClick).toHaveBeenCalledWith({
      label: 'Open Strava',
      action_type: 'url',
      value: 'https://www.strava.com/athlete',
    });
  });

  it('draws the activity panel from its block and counts the real activities', () => {
    const blocks: ReplyBlock[] = [
      { type: 'activity_list', text: '1. Long run - 24 km\n2. Threshold 3x10 - 14 km' },
      { type: 'prose', text: 'Your load is climbing.' },
    ];

    render(<MessageItem message={assistantMessage()} blocks={blocks} />);

    expect(screen.getByText('Your Activities (2)')).toBeInTheDocument();
    expect(screen.getByText('Your load is climbing.')).toBeInTheDocument();
  });

  it('renders a verdict chip from the block chips alone, with the status as its qualifier', () => {
    const blocks: ReplyBlock[] = [
      { type: 'prose', text: 'Your VO2max is 82.' },
      {
        type: 'verdicts',
        chips: [
          { claim: 'Your VO2max is 82.', contradicted: true },
          { claim: 'Sleep six hours is plenty.', contradicted: false },
        ],
      },
    ];

    render(<MessageItem message={assistantMessage()} blocks={blocks} />);

    expect(screen.getByText('2 verdicts · contradicted')).toBeInTheDocument();
  });

  it('draws no notice inside the message — the conversation banner owns it', () => {
    const blocks: ReplyBlock[] = [
      { type: 'prose', text: 'Here is your week.' },
      {
        type: 'notice',
        notice: {
          kind: 'quota_warning',
          level: 'approaching',
          current: 45,
          limit: 50,
          resets_at: '2026-08-26T00:00:00Z',
        },
      },
    ];

    render(<MessageItem message={assistantMessage()} blocks={blocks} />);

    expect(screen.getByText('Here is your week.')).toBeInTheDocument();
    expect(screen.queryByText(/45\/50/)).not.toBeInTheDocument();
  });

  it('falls back to the transcript row when the turn sent no blocks', () => {
    // A conversation read back from history has no block list on the wire.
    render(<MessageItem message={assistantMessage('Nice negative split.')} />);
    expect(screen.getByText('Nice negative split.')).toBeInTheDocument();
  });
});
