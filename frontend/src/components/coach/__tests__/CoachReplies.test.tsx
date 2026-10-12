// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The coach's Replies page — every group reply with its verdicts, the unmatched-figure line, a flag and a correction
// ABOUTME: Pins what the page shows from the server's page and what it sends back when the coach flags or corrects (carnet#847)

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { CoachRepliesPage } from '@pierre/shared-types';
import { ToastProvider } from '../../ui';
import CoachReplies from '../CoachReplies';

const list = vi.fn();
const flag = vi.fn();
const correct = vi.fn();
const weekly = vi.fn();

vi.mock('../../../services/api', () => ({
  coachRepliesApi: {
    list: (...a: unknown[]) => list(...a),
    flag: (...a: unknown[]) => flag(...a),
    correct: (...a: unknown[]) => correct(...a),
    weekly: (...a: unknown[]) => weekly(...a),
  },
}));

const PAGE: CoachRepliesPage = {
  groups: [{ id: 'group-1', name: "Alex's block" }],
  athletes: [
    { user_id: 'user-alex', name: 'Alex', shares_with_coach: true },
    { user_id: 'user-ben', name: 'Ben', shares_with_coach: false },
  ],
  replies: [
    {
      id: 'reply-1',
      group_id: 'group-1',
      group_name: "Alex's block",
      athlete_user_id: 'user-alex',
      athlete_name: 'Alex',
      content: 'Your 2:41 marathon in May shows you are ready.',
      withheld: false,
      created_at: '2026-10-09T08:00:00Z',
      verdicts: [
        {
          id: 'v1',
          conversation_id: 'c1',
          message_id: 'm1',
          agent_id: null,
          claim_text: 'your 2:41 marathon in May',
          category: 'athlete_data',
          status: 'unverifiable',
          evidence_strength: 'none',
          confidence: 0,
          layer_fired: 'athlete_data',
          explanation: null,
          evidence_refs: null,
          created_at: '2026-10-09T08:00:01Z',
        },
      ],
      unmatched_figures: 1,
      flags: [],
      corrections: [],
    },
  ],
  next_before: null,
};

function renderPage() {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={queryClient}>
      <ToastProvider>
        <CoachReplies />
      </ToastProvider>
    </QueryClientProvider>,
  );
}

describe('CoachReplies', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    list.mockResolvedValue(PAGE);
    weekly.mockResolvedValue({ weeks: [{ week_start: '2026-10-05', flags: 2, corrections: 1 }] });
  });

  it('shows each reply with its verdict chips, the unmatched figure and the week\'s count', async () => {
    renderPage();

    const card = await screen.findByTestId('coach-reply-reply-1');
    expect(within(card).getByText("To Alex · Alex's block")).toBeInTheDocument();
    expect(within(card).getByTestId('coach-reply-content')).toHaveTextContent(
      'Your 2:41 marathon in May shows you are ready.',
    );
    expect(within(card).getByTestId('coach-reply-verdict')).toHaveTextContent(
      'unverifiable · your 2:41 marathon in May',
    );
    expect(within(card).getByTestId('coach-reply-unmatched')).toHaveTextContent(
      "Figures the checker couldn't match to Alex's activities: 1",
    );
    expect(await screen.findByText('This week: 2 flagged · 1 corrected')).toBeInTheDocument();
    // An athlete who does not share is offered, and says so.
    expect(screen.getByRole('option', { name: 'Ben (not shared with you)' })).toBeInTheDocument();
    expect(list).toHaveBeenCalledWith({ athlete_id: undefined, before: undefined, limit: 20 });
  });

  it('narrows the page to one athlete', async () => {
    renderPage();
    await screen.findByTestId('coach-reply-reply-1');

    await userEvent.selectOptions(screen.getByRole('combobox', { name: 'Athlete' }), 'user-alex');

    await waitFor(() =>
      expect(list).toHaveBeenLastCalledWith({ athlete_id: 'user-alex', before: undefined, limit: 20 }),
    );
  });

  it('flags a reply with its reason and note', async () => {
    flag.mockResolvedValue({ flag: {} });
    renderPage();
    const card = await screen.findByTestId('coach-reply-reply-1');

    await userEvent.click(within(card).getByRole('button', { name: 'Flag' }));
    await userEvent.selectOptions(within(card).getByRole('combobox', { name: 'Why' }), 'unsafe');
    await userEvent.type(within(card).getByRole('textbox', { name: 'Note (only you see it)' }), 'Too hard');
    await userEvent.click(within(card).getByRole('button', { name: 'Flag reply' }));

    await waitFor(() => expect(flag).toHaveBeenCalledWith('reply-1', 'unsafe', 'Too hard'));
  });

  it('posts a correction', async () => {
    correct.mockResolvedValue({ correction: {}, posted_to_chat: false });
    renderPage();
    const card = await screen.findByTestId('coach-reply-reply-1');

    await userEvent.click(within(card).getByRole('button', { name: 'Correct' }));
    await userEvent.type(
      within(card).getByRole('textbox', { name: 'Correction (posted in the group thread)' }),
      'It was a half marathon.',
    );
    await userEvent.click(within(card).getByRole('button', { name: 'Post correction' }));

    await waitFor(() => expect(correct).toHaveBeenCalledWith('reply-1', 'It was a half marathon.'));
  });

  it('says so when the caller coaches no group', async () => {
    list.mockResolvedValue({ groups: [], athletes: [], replies: [], next_before: null });
    renderPage();

    expect(await screen.findByTestId('coach-replies-not-a-coach')).toHaveTextContent(
      "You don't run a coaching group yet.",
    );
  });
});
