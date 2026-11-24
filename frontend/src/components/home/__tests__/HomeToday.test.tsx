// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the Home Today section — today's session enlarged with its phase week, tomorrow under it
// ABOUTME: Red if a rest day and a day the plan never reached read the same, a date's second session goes missing, or a tap drafts the wrong question

import { describe, it, expect, vi } from 'vitest';
import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { HomeToday } from '../HomeToday';
import { homePlan, TODAY, twoSessionPlan } from './homeFixtures';
import {
  DRAFT_DATE,
  formatCivilDate,
  planWindow,
} from '@pierre/ui-logic';

function calendarFor(today: string) {
  const calendar = planWindow(today);
  if (calendar === null) throw new Error(`fixture today ${today} is not a calendar day`);
  return calendar;
}

describe('HomeToday', () => {
  it("shows today's session with its facts and the build phase's week", () => {
    render(<HomeToday plan={homePlan()} calendar={calendarFor(TODAY)} onOpenChatDraft={vi.fn()} />);

    const card = screen.getByTestId('home-today-session-0');
    expect(within(card).getByText('Tempo run')).toBeInTheDocument();
    expect(within(card).getByText('Run · 50 min · threshold')).toBeInTheDocument();
    // 2026-09-14 starts the build phase, so the 24th is in its second week.
    expect(within(card).getByText('Build · week 2')).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: 'Today' })).toBeInTheDocument();
  });

  it('drafts the walk-through question for the session in chat', async () => {
    const onOpenChatDraft = vi.fn();
    render(<HomeToday plan={homePlan()} calendar={calendarFor(TODAY)} onOpenChatDraft={onOpenChatDraft} />);

    await userEvent.click(screen.getByTestId('home-today-session-0'));

    const named = formatCivilDate(TODAY, 'en', DRAFT_DATE);
    expect(named).toMatch(/Thursday/);
    expect(onOpenChatDraft).toHaveBeenCalledExactlyOnceWith(`Walk me through my session on ${named}: Tempo run`);
  });

  it('offers a route for today\'s session and drafts the question naming it', async () => {
    const onOpenChatDraft = vi.fn();
    render(<HomeToday plan={homePlan()} calendar={calendarFor(TODAY)} onOpenChatDraft={onOpenChatDraft} />);

    const link = screen.getByTestId('home-today-route-0');
    expect(link).toHaveTextContent('Find a route for this session');
    await userEvent.click(link);

    const named = formatCivilDate(TODAY, 'en', DRAFT_DATE);
    expect(onOpenChatDraft).toHaveBeenCalledExactlyOnceWith(
      `Suggest a route close to where I am for my session on ${named}: Tempo run`,
    );
  });

  it('offers no route on a rest day, an uncovered day, or a session with no route to find', () => {
    const { unmount } = render(
      <HomeToday plan={homePlan()} calendar={calendarFor('2026-09-23')} onOpenChatDraft={vi.fn()} />,
    );
    expect(screen.getByTestId('home-today-rest-0')).toBeInTheDocument();
    expect(screen.queryByTestId('home-today-route-0')).toBeNull();
    unmount();

    const uncovered = render(
      <HomeToday plan={homePlan()} calendar={calendarFor('2026-09-26')} onOpenChatDraft={vi.fn()} />,
    );
    expect(screen.queryByTestId('home-today-route-0')).toBeNull();
    uncovered.unmount();

    const plan = homePlan();
    for (const week of plan.weeks) {
      for (const day of week.days) {
        if (day.date === TODAY) day.sport = 'swim';
      }
    }
    render(<HomeToday plan={plan} calendar={calendarFor(TODAY)} onOpenChatDraft={vi.fn()} />);
    expect(screen.getByTestId('home-today-session-0')).toBeInTheDocument();
    expect(screen.queryByTestId('home-today-route-0')).toBeNull();
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

    const card = screen.getByTestId('home-today-rest-0');
    expect(within(card).getByText('Rest')).toBeInTheDocument();
    expect(within(card).getByText('threshold volume')).toBeInTheDocument();
    expect(screen.queryByTestId('home-today-uncovered')).toBeNull();
  });

  it('says the plan does not cover a day it never reached, and offers no draft for it', () => {
    const onOpenChatDraft = vi.fn();
    // Saturday the 26th has no entry at all in the fixture's week.
    render(<HomeToday plan={homePlan()} calendar={calendarFor('2026-09-26')} onOpenChatDraft={onOpenChatDraft} />);

    expect(screen.getByTestId('home-today-uncovered')).toHaveTextContent("Your plan doesn't cover this day.");
    expect(screen.queryByTestId('home-today-rest-0')).toBeNull();
    expect(screen.queryByTestId('home-today-session-0')).toBeNull();
    // Tomorrow (Sunday) is the long ride, as its own line.
    expect(within(screen.getByTestId('home-tomorrow')).getByRole('button', { name: 'Long ride · 180 min' })).toBeInTheDocument();
  });

  it('shows one card per entry today, in order, each drafting its own question and offering its own route', async () => {
    const onOpenChatDraft = vi.fn();
    render(<HomeToday plan={twoSessionPlan()} calendar={calendarFor(TODAY)} onOpenChatDraft={onOpenChatDraft} />);

    const swim = screen.getByTestId('home-today-session-0');
    const tempo = screen.getByTestId('home-today-session-1');
    expect(swim.compareDocumentPosition(tempo) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(within(swim).getByText('Technique swim')).toBeInTheDocument();
    // The phase's week rides on the first card alone.
    expect(within(swim).getByText('Build · week 2')).toBeInTheDocument();
    expect(within(tempo).getByText('Tempo run')).toBeInTheDocument();
    expect(within(tempo).queryByText('Build · week 2')).toBeNull();

    // A route only for the session that has one to find, named by it.
    expect(screen.queryByTestId('home-today-route-0')).toBeNull();
    const named = formatCivilDate(TODAY, 'en', DRAFT_DATE);
    await userEvent.click(screen.getByTestId('home-today-route-1'));
    expect(onOpenChatDraft).toHaveBeenLastCalledWith(
      `Suggest a route close to where I am for my session on ${named}: Tempo run`,
    );

    await userEvent.click(swim);
    expect(onOpenChatDraft).toHaveBeenLastCalledWith(`Walk me through my session on ${named}: Technique swim`);
    await userEvent.click(tempo);
    expect(onOpenChatDraft).toHaveBeenLastCalledWith(`Walk me through my session on ${named}: Tempo run`);
  });

  it("puts each of tomorrow's entries on its own line — the rest day, then the optional spin", async () => {
    const onOpenChatDraft = vi.fn();
    render(<HomeToday plan={twoSessionPlan()} calendar={calendarFor(TODAY)} onOpenChatDraft={onOpenChatDraft} />);

    const tomorrow = screen.getByTestId('home-tomorrow');
    expect(within(tomorrow).getByTestId('home-tomorrow-0')).toHaveTextContent('Rest');
    const spin = within(tomorrow).getByTestId('home-tomorrow-1');
    expect(spin).toHaveTextContent('Optional: easy spin · 30 min');
    await userEvent.click(spin);

    const named = formatCivilDate('2026-09-25', 'en', DRAFT_DATE);
    expect(onOpenChatDraft).toHaveBeenCalledExactlyOnceWith(`Walk me through my session on ${named}: Optional: easy spin`);
  });

  it('marks tomorrow uncovered past the last shown week, as text rather than a button', () => {
    render(<HomeToday plan={homePlan()} calendar={calendarFor('2026-09-28')} onOpenChatDraft={vi.fn()} />);

    const tomorrow = screen.getByTestId('home-tomorrow');
    expect(tomorrow).toHaveTextContent("Your plan doesn't cover this day.");
    expect(within(tomorrow).queryByRole('button')).toBeNull();
  });
});
