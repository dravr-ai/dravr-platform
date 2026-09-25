// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the Home Today section — today's session enlarged with its phase week, tomorrow under it
// ABOUTME: Red if a rest day and a day the plan never reached read the same, or a tap drafts the wrong question

import { describe, it, expect, vi } from 'vitest';
import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { HomeToday } from '../HomeToday';
import { planWindow, DRAFT_DATE, formatCivilDate } from '../homeFormat';
import { homePlan, TODAY } from './homeFixtures';

function calendarFor(today: string) {
  const calendar = planWindow(today);
  if (calendar === null) throw new Error(`fixture today ${today} is not a calendar day`);
  return calendar;
}

describe('HomeToday', () => {
  it("shows today's session with its facts and the build phase's week", () => {
    render(<HomeToday plan={homePlan()} calendar={calendarFor(TODAY)} onOpenChatDraft={vi.fn()} />);

    const card = screen.getByTestId('home-today-session');
    expect(within(card).getByText('Tempo run')).toBeInTheDocument();
    expect(within(card).getByText('Run · 50 min · threshold')).toBeInTheDocument();
    // 2026-09-14 starts the build phase, so the 24th is in its second week.
    expect(within(card).getByText('Build · week 2')).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: 'Today' })).toBeInTheDocument();
  });

  it('drafts the walk-through question for the session in chat', async () => {
    const onOpenChatDraft = vi.fn();
    render(<HomeToday plan={homePlan()} calendar={calendarFor(TODAY)} onOpenChatDraft={onOpenChatDraft} />);

    await userEvent.click(screen.getByTestId('home-today-session'));

    const named = formatCivilDate(TODAY, 'en', DRAFT_DATE);
    expect(named).toMatch(/Thursday/);
    expect(onOpenChatDraft).toHaveBeenCalledExactlyOnceWith(`Walk me through my session on ${named}: Tempo run`);
  });

  it('puts tomorrow under it — a rest day reads as rest and drafts the rest-day question', async () => {
    const onOpenChatDraft = vi.fn();
    render(<HomeToday plan={homePlan()} calendar={calendarFor(TODAY)} onOpenChatDraft={onOpenChatDraft} />);

    const tomorrow = screen.getByTestId('home-tomorrow');
    expect(within(tomorrow).getByText('Tomorrow')).toBeInTheDocument();
    await userEvent.click(within(tomorrow).getByRole('button', { name: 'Rest' }));

    const named = formatCivilDate('2026-09-25', 'en', DRAFT_DATE);
    expect(onOpenChatDraft).toHaveBeenCalledExactlyOnceWith(`Why is ${named} a rest day in my plan?`);
  });

  it('says a rest day today is rest, with the week focus under it', () => {
    render(<HomeToday plan={homePlan()} calendar={calendarFor('2026-09-23')} onOpenChatDraft={vi.fn()} />);

    const card = screen.getByTestId('home-today-rest');
    expect(within(card).getByText('Rest')).toBeInTheDocument();
    expect(within(card).getByText('threshold volume')).toBeInTheDocument();
    expect(screen.queryByTestId('home-today-uncovered')).toBeNull();
  });

  it('says the plan does not cover a day it never reached, and offers no draft for it', () => {
    const onOpenChatDraft = vi.fn();
    // Saturday the 26th has no entry at all in the fixture's week.
    render(<HomeToday plan={homePlan()} calendar={calendarFor('2026-09-26')} onOpenChatDraft={onOpenChatDraft} />);

    expect(screen.getByTestId('home-today-uncovered')).toHaveTextContent("Your plan doesn't cover this day.");
    expect(screen.queryByTestId('home-today-rest')).toBeNull();
    expect(screen.queryByTestId('home-today-session')).toBeNull();
    // Tomorrow (Sunday) is the long ride, as its own line.
    expect(within(screen.getByTestId('home-tomorrow')).getByRole('button', { name: 'Long ride · 180 min' })).toBeInTheDocument();
  });

  it('marks tomorrow uncovered past the last shown week, as text rather than a button', () => {
    render(<HomeToday plan={homePlan()} calendar={calendarFor('2026-09-28')} onOpenChatDraft={vi.fn()} />);

    const tomorrow = screen.getByTestId('home-tomorrow');
    expect(tomorrow).toHaveTextContent("Your plan doesn't cover this day.");
    expect(within(tomorrow).queryByRole('button')).toBeNull();
  });
});
