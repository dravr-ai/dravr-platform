// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: E2E for a coach's athlete workspace (carnet#1816) — opened from À traiter, or from Groups' one row and thread header (carnet#1817), Dravr's swap proposed, Refuser, « C'est mon choix » and « Retirer mon choix » sent, the panel's tabs by keyboard, back to the athlete
// ABOUTME: Every route is mocked and dated from the day the spec runs; the routes record what the workspace wrote, so the spec reads the request, not only the page

import { test, expect, type Page } from '@playwright/test';
import { mockCoach } from './coach-home-mocks';
import {
  WORKSPACE_ATHLETE_ID,
  WORKSPACE_PLAN_ID,
  WORKSPACE_ROOM_ID,
  mockWorkspace,
  trifectaRoster,
  workspaceSubjectPlan,
  type WorkspaceCalls,
} from './athlete-workspace-mocks';

/** The dates of the mocked draft: Wednesday and Thursday swapped, a session added on Saturday, next Saturday kept as the coach's call, and next Sunday outside its evidence. */
function draftDates() {
  const plan = workspaceSubjectPlan();
  const [thisWeek, nextWeek] = plan.draft?.weeks ?? [];
  const dateOf = (index: number) => thisWeek.days.map((day) => day.date).filter((d, i, all) => all.indexOf(d) === i)[index];
  return {
    wednesday: dateOf(2),
    thursday: dateOf(3),
    saturday: dateOf(5),
    nextSaturday: nextWeek.days[nextWeek.days.length - 2].date,
    nextSunday: nextWeek.days[nextWeek.days.length - 1].date,
  };
}

/** Open Léa's workspace from her row in À traiter, as a coach does. */
async function openWorkspace(page: Page, calls: WorkspaceCalls) {
  await page.setViewportSize({ width: 1440, height: 960 });
  await mockCoach(page, { roster: trifectaRoster, routes: (routed) => mockWorkspace(routed, calls) });
  await page.getByTestId(`coach-queue-row-${WORKSPACE_ATHLETE_ID}`).click();
  await page.getByTestId('coach-open-workspace').click();
  await expect(page).toHaveURL(new RegExp(`#home/athlete/${WORKSPACE_ATHLETE_ID}/plan$`));
  await expect(page.getByTestId('workspace-calendar')).toBeVisible();
}

test.describe('athlete workspace', () => {
  test('marks the dates Dravr changed, and publish stands ready', async ({ page }) => {
    const calls: WorkspaceCalls = { reverted: [], coachCalls: [] };
    await openWorkspace(page, calls);
    const { wednesday, thursday, saturday } = draftDates();

    const proposed = page.locator('[data-testid^="workspace-date-"][data-proposed="true"]');
    await expect(proposed).toHaveCount(3);
    for (const date of [wednesday, thursday, saturday]) {
      await expect(page.getByTestId(`workspace-date-${date}`)).toHaveAttribute('data-proposed', 'true');
    }
    await expect(page.getByTestId('plan-draft-publish')).toBeEnabled();
  });

  test('« Refuser » puts the live plan back on a date, and only that date stops being proposed', async ({ page }) => {
    const calls: WorkspaceCalls = { reverted: [], coachCalls: [] };
    await openWorkspace(page, calls);
    const { wednesday } = draftDates();

    await page.getByTestId(`workspace-refuse-${wednesday}`).click();
    await expect(page.getByTestId(`workspace-date-${wednesday}`)).not.toHaveAttribute('data-proposed', 'true');
    expect(calls.reverted).toEqual([wednesday]);
    await expect(page.getByTestId(`workspace-date-${wednesday}`)).toContainText('Piste à Bromont');
    await expect(page.locator('[data-testid^="workspace-date-"][data-proposed="true"]')).toHaveCount(2);
    // Focus stays on the date the coach answered.
    await expect(page.getByTestId(`workspace-date-${wednesday}`).getByRole('heading', { level: 3 })).toBeFocused();
  });

  test('« Accepter » writes nothing and leaves the date to publish', async ({ page }) => {
    const calls: WorkspaceCalls = { reverted: [], coachCalls: [] };
    await openWorkspace(page, calls);
    const { thursday } = draftDates();

    await page.getByTestId(`workspace-accept-${thursday}`).click();
    await expect(page.getByTestId(`workspace-date-${thursday}`).getByTestId('workspace-date-accepted')).toBeVisible();
    await expect(page.getByTestId(`workspace-accept-${thursday}`)).toHaveCount(0);
    await expect(page.getByTestId(`workspace-refuse-${thursday}`)).toBeVisible();
    expect(calls.reverted).toEqual([]);
    expect(calls.coachCalls).toEqual([]);
  });

  test('« C\'est mon choix » keeps a session the check names as the coach\'s call', async ({ page }) => {
    const calls: WorkspaceCalls = { reverted: [], coachCalls: [] };
    await openWorkspace(page, calls);
    const { nextSunday } = draftDates();

    const check = page.getByTestId('workspace-prepublish');
    await expect(check).toBeVisible();
    await check.getByTestId(`workspace-check-keep-${nextSunday}-0`).click();
    await expect.poll(() => calls.coachCalls.length).toBe(1);
    expect(calls.coachCalls[0]).toMatchObject({ on: true, expect: { sport: 'ride', workout: 'Longue sortie avec sprints' } });
  });

  test('« Retirer mon choix » withdraws the coach\'s call, and focus stays on the session it sat under', async ({ page }) => {
    const calls: WorkspaceCalls = { reverted: [], coachCalls: [] };
    await openWorkspace(page, calls);
    const { nextSaturday } = draftDates();

    const date = page.getByTestId(`workspace-date-${nextSaturday}`);
    await expect(date.getByTestId('workspace-session-coach-call')).toBeVisible();
    await date.getByTestId(`workspace-call-undo-${nextSaturday}-0`).click();
    await expect.poll(() => calls.coachCalls.length).toBe(1);
    expect(calls.coachCalls[0]).toMatchObject({ on: false, expect: { sport: 'hike', workout: 'Randonnée autour de Sutton' } });
    await expect(date.getByTestId(`workspace-call-undo-${nextSaturday}-0`)).toHaveCount(0);
    await expect(date.getByTestId('workspace-session-coach-call')).toHaveCount(0);
    await expect(page.getByTestId(`workspace-session-${nextSaturday}-0`)).toBeFocused();
  });

  test('« Revoir » opens the day editor on the session the check names', async ({ page }) => {
    const calls: WorkspaceCalls = { reverted: [], coachCalls: [] };
    await openWorkspace(page, calls);
    const { nextSunday } = draftDates();

    await page.getByTestId(`workspace-check-review-${nextSunday}-0`).click();
    await expect(page.getByRole('dialog')).toBeVisible();
  });

  test('the panel\'s tabs are one tab stop, and the arrows, Home and End move between them', async ({ page }) => {
    await openWorkspace(page, { reverted: [], coachCalls: [] });
    const tab = (id: string) => page.locator(`#athlete-workspace-tab-${id}`);

    await expect(tab('history')).toHaveAttribute('tabindex', '0');
    await expect(tab('dravr')).toHaveAttribute('tabindex', '-1');
    await tab('history').focus();
    await page.keyboard.press('ArrowRight');
    await expect(tab('dravr')).toBeFocused();
    await expect(tab('dravr')).toHaveAttribute('aria-selected', 'true');
    await expect(page.getByTestId('workspace-panel-dravr')).toBeVisible();

    await page.keyboard.press('End');
    await expect(tab('group')).toBeFocused();
    await expect(page.getByTestId('workspace-panel-group')).toBeVisible();
    await page.keyboard.press('Home');
    await expect(tab('history')).toBeFocused();
    await expect(tab('history')).toHaveAttribute('tabindex', '0');
    await expect(tab('group')).toHaveAttribute('tabindex', '-1');
  });

  test('the back button returns to the athlete in À traiter', async ({ page }) => {
    await openWorkspace(page, { reverted: [], coachCalls: [] });
    await page.getByTestId('workspace-back').click();
    await expect(page).toHaveURL(new RegExp(`#home/athlete/${WORKSPACE_ATHLETE_ID}$`));
    await expect(page.getByTestId('coach-athlete-pane')).toBeVisible();
  });
});

test.describe("a trifecta athlete in Groups (carnet#1817)", () => {
  test('her room and plan thread are one row, named after her, that opens her workspace', async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 960 });
    await mockCoach(page, { roster: trifectaRoster, routes: (routed) => mockWorkspace(routed, { reverted: [], coachCalls: [] }) });
    await page.goto('/#chat');

    const list = page.getByTestId('conversation-list');
    const athlete = list.locator('[data-testid="conversation-row"][data-opens-workspace="true"]');
    await expect(athlete).toHaveCount(1);
    await expect(list.getByTestId('conversation-row')).toHaveCount(1);
    await expect(athlete.getByTestId('conversation-title')).toHaveText('Léa Gagnon');
    // The plan thread spoke last, and both threads' unread messages count.
    await expect(athlete.getByTestId('conversation-preview')).toHaveText("C'est proposé sur le calendrier.");
    await expect(athlete.getByTestId('conversation-unread-count')).toHaveText('1');
    await expect(athlete.getByTestId('conversation-actions-trigger')).toHaveCount(0);

    await athlete.getByRole('button').click();
    await expect(page).toHaveURL(new RegExp(`#home/athlete/${WORKSPACE_ATHLETE_ID}/plan$`));
    await expect(page.getByTestId('workspace-calendar')).toBeVisible();
  });

  test('« Ouvrir le plan » in her room and in the plan thread opens her workspace', async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 960 });
    await mockCoach(page, { roster: trifectaRoster, routes: (routed) => mockWorkspace(routed, { reverted: [], coachCalls: [] }) });

    for (const conversationId of [WORKSPACE_ROOM_ID, WORKSPACE_PLAN_ID]) {
      await page.goto(`/#chat/${conversationId}`);
      // The athlete's one row is the open thread's, whichever of the two it is.
      await expect(
        page.locator('[data-testid="conversation-row"][data-opens-workspace="true"] button').first(),
      ).toHaveAttribute('aria-current', 'true');
      await page.getByTestId('thread-open-workspace').click();
      await expect(page).toHaveURL(new RegExp(`#home/athlete/${WORKSPACE_ATHLETE_ID}/plan$`));
      await expect(page.getByTestId('workspace-calendar')).toBeVisible();
    }
  });
});
