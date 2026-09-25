// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the Home week strip — seven days Monday to Sunday, today marked, a day that opens to its steps and fuel
// ABOUTME: Red if a gap in the plan reads as rest, the open day loses its steps, or next week's focus goes missing

import { describe, it, expect, vi } from 'vitest';
import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { HomeWeek } from '../HomeWeek';
import { planWindow, DRAFT_DATE, formatCivilDate } from '../homeFormat';
import { homePlan, TODAY } from './homeFixtures';

function renderWeek(onOpenChatDraft = vi.fn()) {
  const calendar = planWindow(TODAY);
  if (calendar === null) throw new Error('fixture today is not a calendar day');
  render(<HomeWeek plan={homePlan()} calendar={calendar} onOpenChatDraft={onOpenChatDraft} />);
  return onOpenChatDraft;
}

const WEEK = ['2026-09-21', '2026-09-22', '2026-09-23', '2026-09-24', '2026-09-25', '2026-09-26', '2026-09-27'];

describe('HomeWeek', () => {
  it('lays out Monday to Sunday of the current week with today marked', () => {
    renderWeek();

    const strip = screen.getByRole('list');
    const cells = within(strip).getAllByRole('button');
    expect(cells.map((cell) => cell.getAttribute('data-testid'))).toEqual(WEEK.map((date) => `home-week-day-${date}`));
    const current = cells.filter((cell) => cell.getAttribute('aria-current') === 'date');
    expect(current).toHaveLength(1);
    expect(current[0]).toHaveAttribute('data-testid', `home-week-day-${TODAY}`);
    expect(screen.getByRole('heading', { name: 'This week' })).toBeInTheDocument();
    expect(screen.getByText('threshold volume')).toBeInTheDocument();
  });

  it('keeps a rest day and a day the plan never reached apart', () => {
    renderWeek();

    expect(screen.getByTestId('home-week-day-2026-09-25')).toHaveAccessibleName(/— Rest$/);
    expect(screen.getByTestId('home-week-day-2026-09-26')).toHaveAccessibleName(/— Your plan doesn't cover this day\.$/);
    expect(screen.getByTestId('home-week-day-2026-09-24')).toHaveTextContent('50 min');
  });

  it("opens a session day to the plan card's own row — steps and fuel — and drafts the question from it", async () => {
    const onOpenChatDraft = renderWeek();

    const cell = screen.getByTestId(`home-week-day-${TODAY}`);
    expect(cell).toHaveAttribute('aria-expanded', 'false');
    await userEvent.click(cell);
    expect(cell).toHaveAttribute('aria-expanded', 'true');

    const detail = screen.getByTestId('home-week-detail');
    expect(cell).toHaveAttribute('aria-controls', 'home-week-detail');
    expect(within(detail).getByText('Tempo run')).toBeInTheDocument();
    expect(within(detail).getByText('Steps')).toBeInTheDocument();
    expect(within(detail).getByText('Warm-up · 15m · Z1')).toBeInTheDocument();
    expect(within(detail).getByText('Fuel')).toBeInTheDocument();

    const named = formatCivilDate(TODAY, 'en', DRAFT_DATE);
    const draft = `Walk me through my session on ${named}: Tempo run`;
    await userEvent.click(within(detail).getByRole('button', { name: draft }));
    expect(onOpenChatDraft).toHaveBeenCalledExactlyOnceWith(draft);
  });

  it('closes the open day on a second tap', async () => {
    renderWeek();

    const cell = screen.getByTestId('home-week-day-2026-09-27');
    await userEvent.click(cell);
    expect(within(screen.getByTestId('home-week-detail')).getByText('Long ride')).toBeInTheDocument();
    await userEvent.click(cell);
    expect(screen.queryByTestId('home-week-detail')).toBeNull();
  });

  it('opens an uncovered day to the sentence saying so, with no draft to send', async () => {
    const onOpenChatDraft = renderWeek();

    await userEvent.click(screen.getByTestId('home-week-day-2026-09-26'));

    const detail = screen.getByTestId('home-week-detail');
    expect(detail).toHaveTextContent("Your plan doesn't cover this day.");
    expect(within(detail).queryByRole('button')).toBeNull();
    expect(onOpenChatDraft).not.toHaveBeenCalled();
  });

  it("names next week with the plan card's heading and its focus", () => {
    renderWeek();

    const next = screen.getByTestId('home-next-week');
    expect(within(next).getByText('Next week')).toBeInTheDocument();
    expect(within(next).getByText('2026-09-28')).toBeInTheDocument();
    expect(within(next).getByText('absorb the block')).toBeInTheDocument();
  });
});
