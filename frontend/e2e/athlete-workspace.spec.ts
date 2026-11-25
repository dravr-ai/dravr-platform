// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: E2E for a coach's athlete workspace (carnet#1816) — opened from À traiter, Dravr's swap proposed, Refuser and « C'est mon choix » sent, back to the athlete
// ABOUTME: Every route is mocked and dated from the day the spec runs; the routes record what the workspace wrote, so the spec reads the request, not only the page

import { test, expect, type Page } from '@playwright/test';
import { mockCoach } from './coach-home-mocks';
import {
  WORKSPACE_ATHLETE_ID,
  mockWorkspace,
  trifectaRoster,
  workspaceSubjectPlan,
  type WorkspaceCalls,
} from './athlete-workspace-mocks';

/** The dates of the mocked draft: Wednesday and Thursday swapped, a session added on Saturday, and next Sunday outside its evidence. */
function draftDates() {
  const plan = workspaceSubjectPlan();
  const [thisWeek, nextWeek] = plan.draft?.weeks ?? [];
  const dateOf = (index: number) => thisWeek.days.map((day) => day.date).filter((d, i, all) => all.indexOf(d) === i)[index];
  return {
    wednesday: dateOf(2),
    thursday: dateOf(3),
    saturday: dateOf(5),
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

  test('« Revoir » opens the day editor on the session the check names', async ({ page }) => {
    const calls: WorkspaceCalls = { reverted: [], coachCalls: [] };
    await openWorkspace(page, calls);
    const { nextSunday } = draftDates();

    await page.getByTestId(`workspace-check-review-${nextSunday}-0`).click();
    await expect(page.getByRole('dialog')).toBeVisible();
  });

  test('the back button returns to the athlete in À traiter', async ({ page }) => {
    await openWorkspace(page, { reverted: [], coachCalls: [] });
    await page.getByTestId('workspace-back').click();
    await expect(page).toHaveURL(new RegExp(`#home/athlete/${WORKSPACE_ATHLETE_ID}$`));
    await expect(page.getByTestId('coach-athlete-pane')).toBeVisible();
  });
});
