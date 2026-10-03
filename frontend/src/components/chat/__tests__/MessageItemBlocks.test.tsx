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
  it("draws an agent's welcome under the agent's name, its starters as buttons that send them", async () => {
    // carnet#735: the row an agent posts when it is bound into a thread is
    // read back from history — no block list — with its starters in the
    // persisted `actions`. It renders like any agent message.
    const user = userEvent.setup();
    const onActionClick = vi.fn();
    const starters = [
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
        actions: starters.map((q) => ({ label: q, action_type: 'postback', value: q })),
      },
    };

    render(
      <MessageItem message={welcome} assistantLabel="Agent Ravitaillement" onActionClick={onActionClick} />,
    );

    expect(screen.getByText('Agent Ravitaillement')).toBeInTheDocument();
    expect(screen.getByText(/Agent Ravitaillement de Dravr/)).toBeInTheDocument();
    expect(screen.getByText('Pour commencer, tu peux me demander :')).toBeInTheDocument();
    for (const q of starters) {
      expect(screen.getByRole('button', { name: q })).toBeInTheDocument();
    }
    await user.click(screen.getByRole('button', { name: starters[1] }));
    expect(onActionClick).toHaveBeenCalledWith({
      label: starters[1],
      action_type: 'postback',
      value: starters[1],
    });
  });

  it("offers only copy on an agent's welcome — no model wrote it to rate, share, regenerate or label", () => {
    // Review of carnet#735: the welcome row drew Share, both ratings,
    // Regenerate and the conversation's model label. Regenerate walked back to
    // the `/agent add` line, re-sent it and dropped the welcome from the cache;
    // a rating filed feedback against a row no model produced.
    const handlers = {
      onCopy: vi.fn(),
      onShare: vi.fn(),
      onThumbsUp: vi.fn(),
      onThumbsDown: vi.fn(),
      onRetry: vi.fn(),
    };
    const metadata = { model: 'gemini-test-model', executionTimeMs: 0 };
    const welcome: Message = {
      id: 'welcome-2',
      role: 'assistant',
      content: "Hi! Dravr's Tempo Agent here.",
      finish_reason: 'agent_welcome',
      created_at: '2026-10-02T10:00:00Z',
      actions: {
        title: 'To get started, you can ask me:',
        actions: [{ label: 'Plan my tempo week', action_type: 'postback', value: 'Plan my tempo week' }],
      },
    };

    const { unmount } = render(
      <MessageItem message={assistantMessage()} metadata={metadata} {...handlers} />,
    );
    // The control: a model's reply draws all five, and its model.
    expect(screen.getAllByRole('button')).toHaveLength(5);
    expect(screen.getByText('gemini-test-model')).toBeInTheDocument();
    unmount();

    render(<MessageItem message={welcome} metadata={metadata} {...handlers} />);
    const group = screen.getByRole('group');
    const controls = group.querySelectorAll('button');
    expect(controls).toHaveLength(1);
    controls[0].click();
    expect(handlers.onCopy).toHaveBeenCalledTimes(1);
    expect(screen.queryByText('gemini-test-model')).not.toBeInTheDocument();
  });

  it('gives each starter chip the coarse-pointer touch target', () => {
    // DESIGN.md §8: 44×44 px under a coarse pointer, which the `touch-target`
    // class supplies; the plain chip was ~32 px tall on a phone browser.
    render(
      <MessageItem
        message={assistantMessage()}
        blocks={[
          { type: 'actions', actions: [{ label: 'Plan my tempo week', action_type: 'postback', value: 'x' }] },
        ]}
      />,
    );
    expect(screen.getByRole('button', { name: 'Plan my tempo week' })).toHaveClass('touch-target');
  });
});
