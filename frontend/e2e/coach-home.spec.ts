// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: E2E for a coach's Home (carnet#1250, #1252, #1253): the switch, À traiter and its pane, Tout voir and back, La saison, Traité and its undo
// ABOUTME: Every route is mocked — the roster is dated from the day the spec runs — and the phone width gets the list-and-pane shape

import { test, expect } from '@playwright/test';
import { mockCoach } from './coach-home-mocks';

test.describe('Coach Home', () => {
  test('a coach lands on their athletes, queue first, with the scope stated', async ({ page }) => {
    await mockCoach(page);
    await expect(page.getByTestId('coach-home')).toBeVisible();
    await expect(page).toHaveURL(/#home$/);
    await expect(page.getByTestId('coach-home-switch-athletes')).toHaveAttribute('aria-pressed', 'true');
    await expect(page.getByTestId('coach-scope')).toContainText(/8.*10/);
    // The queue, today first, and the first of it open beside.
    await expect(page.getByTestId('coach-queue-today').getByRole('button')).toHaveCount(4);
    await expect(page.getByTestId('coach-queue-week').getByRole('button')).toHaveCount(3);
    await expect(page.getByTestId('coach-athlete-pane').getByRole('heading', { level: 2 })).toHaveText('Maude Poirier');
    // A non-sharer: status only, nothing of training.
    await expect(page.getByTestId('coach-hidden-note')).toBeVisible();
    await expect(page.getByTestId('form-strip')).toHaveCount(0);
  });

  test('the switch shows today\'s athlete Home under "My training" and returns', async ({ page }) => {
    await mockCoach(page);
    await page.getByTestId('coach-home-switch-me').click();
    await expect(page).toHaveURL(/#home\/me$/);
    await expect(page.getByTestId('home-page')).toBeVisible();
    await expect(page.getByTestId('coach-home-switch-me')).toHaveAttribute('aria-pressed', 'true');
    await page.getByTestId('coach-home-switch-athletes').click();
    await expect(page).toHaveURL(/#home$/);
    await expect(page.getByTestId('coach-home')).toBeVisible();
  });

  test('a row opens its athlete: why now, the form band, the plan and the sources', async ({ page }) => {
    await mockCoach(page);
    await page.getByTestId('coach-queue-row-u-lea').click();
    await expect(page).toHaveURL(/#home\/athlete\/u-lea$/);
    const pane = page.getByTestId('coach-athlete-pane');
    await expect(pane.getByRole('heading', { level: 2 })).toHaveText('Léa Gagnon');
    await expect(pane.getByTestId('coach-evidence')).toHaveCount(2);
    await expect(pane.getByTestId('form-strip')).toHaveAttribute('data-band', 'deep_fatigue');
    await expect(pane.getByTestId('coach-today-line')).toBeVisible();
    // The hash is the route: a link to another athlete opens them.
    await page.evaluate(() => {
      window.location.hash = '#home/athlete/u-felix';
    });
    await expect(page.getByTestId('coach-athlete-pane').getByRole('heading', { level: 2 })).toHaveText('Félix Morin');
  });

  test('Tout voir opens the team table sorted by days without activity, and links back', async ({ page }) => {
    await mockCoach(page);
    await page.getByTestId('coach-see-all').click();
    await expect(page).toHaveURL(/#home\/team$/);
    const table = page.getByTestId('coach-roster');
    await expect(table).toBeVisible();
    const first = table.locator('[role="row"][data-testid^="coach-row-"]').first();
    await expect(first).toHaveAttribute('data-testid', 'coach-row-u-julien');
    await expect(table.getByTestId('coach-roster-non-sharers').getByTestId('coach-nonsharer-u-hugo')).toBeVisible();
    // The table scrolls in its own box; the page does not scroll sideways.
    const sideways = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
    expect(sideways).toBeLessThanOrEqual(0);
    await page.getByTestId('coach-group-tab-g-run').click();
    await expect(table.locator('[role="row"][data-testid^="coach-row-u-"]')).toHaveCount(1);
    await page.getByTestId('coach-roster-back').click();
    await expect(page).toHaveURL(/#home$/);
    await expect(page.getByTestId('coach-queue')).toBeVisible();
  });

  test('La saison draws the A races ahead first, then those without one', async ({ page }) => {
    await mockCoach(page);
    await page.getByTestId('coach-view-season').click();
    await expect(page).toHaveURL(/#home\/season$/);
    await expect(page.getByTestId('coach-season-ahead').getByRole('row')).toHaveCount(3);
    await expect(page.getByTestId('coach-season-without').getByRole('row')).toHaveCount(5);
    await expect(page.getByTestId('coach-season-hidden')).toBeVisible();
    await page.getByTestId('coach-season-scale-12w').click();
    await expect(page.getByTestId('coach-season-scale-12w')).toHaveAttribute('aria-pressed', 'true');
    await expect(page.getByTestId('coach-season-today')).toBeVisible();
  });

  test('Traité sends the signal back and can be undone', async ({ page }) => {
    const posted = await mockCoach(page);
    await page.getByTestId('coach-queue-row-u-julien').click();
    await page.getByTestId('coach-treat').click();
    await expect(page.getByTestId('coach-treated-row-u-julien')).toBeVisible();
    await expect(page.getByTestId('coach-queue-row-u-julien')).toHaveCount(0);
    expect(posted.dismiss).toEqual([
      { athlete_user_id: 'u-julien', reason_id: 'nothing_received', fingerprint: 'fp:nothing_received' },
    ]);
    await page.getByTestId('coach-treated-undo-u-julien').click();
    await expect(page.getByTestId('coach-queue-row-u-julien')).toBeVisible();
    expect(posted.restore).toEqual([{ athlete_user_id: 'u-julien', reason_id: 'nothing_received' }]);
  });

  test('"Écrire dans le groupe" opens the group\'s room', async ({ page }) => {
    await mockCoach(page);
    await page.getByTestId('coach-queue-row-u-lea').click();
    await page.getByTestId('coach-write-group').click();
    await expect(page).toHaveURL(/#chat\/room-tri$/);
  });

  test('"Corriger la réponse" opens the reply review on the athlete', async ({ page }) => {
    await mockCoach(page);
    await page.getByTestId('coach-queue-row-u-felix').click();
    await page.getByTestId('coach-primary-action').click();
    await expect(page).toHaveURL(/#coach-replies\/u-felix$/);
  });
});

test.describe('Coach Home at a desktop width', () => {
  test.use({ viewport: { width: 1280, height: 800 } });

  test('all nine columns of the team table fit, with no sideways scroll in its box', async ({ page }) => {
    await mockCoach(page, { fillers: 4 });
    await page.getByTestId('coach-see-all').click();
    const box = page.getByTestId('coach-roster-scroll');
    await expect(box).toBeVisible();
    const overflow = await box.evaluate((el) => el.scrollWidth - el.clientWidth);
    expect(overflow).toBeLessThanOrEqual(0);
    // The last column is on screen, and a stale form row keeps its band word whole.
    await expect(page.getByRole('columnheader').last()).toBeInViewport();
    const word = page.getByTestId('coach-row-u-julien').getByText(/^(Balanced|Équilibré)$/);
    await expect(word).toBeVisible();
    expect(await word.evaluate((el) => el.scrollWidth - el.clientWidth)).toBeLessThanOrEqual(0);
  });

  test('a phase name shows whole or not at all, and never under the race marker', async ({ page }) => {
    await mockCoach(page);
    await page.getByTestId('coach-view-season').click();
    await expect(page.getByTestId('coach-season-ahead')).toBeVisible();
    const clipped = await page.getByTestId('coach-season-bar').evaluateAll((bars) =>
      bars
        .map((bar) => {
          const text = bar.firstElementChild as HTMLElement;
          const visible = getComputedStyle(text).visibility !== 'hidden';
          return visible && text.offsetWidth > bar.clientWidth ? bar.getAttribute('title') : null;
        })
        .filter((name) => name !== null),
    );
    expect(clipped).toEqual([]);
    // Every name stays reachable: as the band's title even when its text is dropped.
    const untitled = await page.getByTestId('coach-season-bar').evaluateAll((bars) => bars.filter((b) => !b.getAttribute('title')).length);
    expect(untitled).toBe(0);
    // The athlete pane's plan bands follow the same rule.
    await page.getByTestId('coach-season-open-u-lea').click();
    const pane = await page.getByTestId('coach-plan-block').locator('[title]').evaluateAll((bands) =>
      bands.filter((band) => {
        const text = band.firstElementChild as HTMLElement;
        return getComputedStyle(text).visibility !== 'hidden' && text.offsetWidth > band.clientWidth;
      }).length,
    );
    expect(pane).toBe(0);
  });

  test('a view tab does not keep its focus ring after a click moves on', async ({ page }) => {
    await mockCoach(page);
    await page.getByTestId('coach-view-team').click();
    await page.getByTestId('coach-view-season').click();
    const shadow = await page.getByTestId('coach-view-team').evaluate((el) => getComputedStyle(el).boxShadow);
    expect(shadow).toBe('none');
  });

  test('an athlete who never shared gets a note that is not the revoke note', async ({ page }) => {
    await mockCoach(page);
    await page.getByTestId('coach-not-sharing-u-hugo').click();
    const never = await page.getByTestId('coach-hidden-note').textContent();
    await page.getByTestId('coach-queue-row-u-maude').click();
    const revoked = await page.getByTestId('coach-hidden-note').textContent();
    expect(never).not.toBe(revoked);
    expect(never).not.toContain('coachHome.');
  });
});

test.describe('Coach Home on a phone', () => {
  test.use({ viewport: { width: 390, height: 844 } });

  test('the list and the athlete take turns, with a way back', async ({ page }) => {
    await mockCoach(page);
    await expect(page.getByTestId('coach-queue')).toBeVisible();
    await expect(page.getByTestId('coach-athlete-pane')).toHaveCount(0);
    await page.getByTestId('coach-queue-row-u-julien').click();
    await expect(page).toHaveURL(/#home\/athlete\/u-julien$/);
    await expect(page.getByTestId('coach-athlete-pane')).toBeVisible();
    await page.getByTestId('coach-pane-back').click();
    await expect(page.getByTestId('coach-queue')).toBeVisible();
    const sideways = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
    expect(sideways).toBeLessThanOrEqual(0);
  });

  test('the team is a two-line list and the season a countdown list', async ({ page }) => {
    await mockCoach(page);
    await page.getByTestId('coach-see-all').click();
    await expect(page.getByTestId('coach-roster-list')).toBeVisible();
    await expect(page.getByRole('table')).toHaveCount(0);
    await page.getByTestId('coach-view-season').click();
    await expect(page.getByTestId('coach-season-ahead').getByRole('button')).toHaveCount(3);
    const sideways = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
    expect(sideways).toBeLessThanOrEqual(0);
  });
});
