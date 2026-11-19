// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: E2E for the invite card a /groups/join/<code> link opens on, Home's held-invite strip and the coach's invite funnel (carnet#1649, #1650)
// ABOUTME: Every route is mocked; the card, the dead-link card and the funnel row are screenshotted in both themes under test-results/invite-card

import { test, expect, type Page } from '@playwright/test';
import type { GroupInvitePreview } from '@pierre/shared-types';
import { mockSetupStatus, mockSignupConfig, mockSignedOut } from './auth-mocks';
import { setupDashboardMocks, loginToDashboard, openHome, waitForLoginScreen } from './test-helpers';
import { mockCoach } from './coach-home-mocks';

const THEMES = ['light', 'dark'] as const;
type Theme = (typeof THEMES)[number];

const CODE = 'MRT2026X';

const LIVE: GroupInvitePreview = {
  status: 'live',
  kind: 'member',
  inviter_name: 'Jeremy B.',
  inviter_initials: 'JB',
  has_coach: true,
  coach_name: 'Jeremy B.',
  group_name: 'Morning Riders',
  expires_at: '2099-12-13T14:40:00Z',
};

async function pinTheme(page: Page, theme: Theme) {
  await page.addInitScript((value) => {
    try {
      window.localStorage.setItem('dravr.theme', value);
    } catch {
      /* storage refused: the default theme stands */
    }
  }, theme);
}

/** The signed-out page a link opens on, with the public preview answering `preview` (or a status). */
async function openLink(page: Page, answer: GroupInvitePreview | { status: number }) {
  await mockSetupStatus(page);
  await mockSignupConfig(page);
  await mockSignedOut(page);
  const reads: string[] = [];
  await page.route('**/api/invites/*', async (route) => {
    reads.push(route.request().url());
    if ('kind' in answer) {
      await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(answer) });
    } else {
      await route.fulfill({ status: answer.status, contentType: 'application/json', body: '{}' });
    }
  });
  await page.goto(`/groups/join/${CODE}`);
  return reads;
}

const carried = (page: Page) => page.evaluate(() => window.sessionStorage.getItem('dravr.pending_invite_code'));

test.describe('The invite card', () => {
  for (const theme of THEMES) {
    test(`a live link opens on the card before any form (${theme})`, async ({ page }) => {
      await pinTheme(page, theme);
      const reads = await openLink(page, LIVE);

      const card = page.getByTestId('invite-card');
      await expect(card).toBeVisible();
      await expect(page.getByTestId('invite-card-title')).toHaveText('Jeremy B. invites you');
      await expect(page.getByTestId('invite-card-group')).toHaveText('Morning Riders');
      await expect(page.getByTestId('invite-card-avatar')).toHaveText('JB');
      await expect(page.getByTestId('invite-card-valid-until')).toContainText('Link valid until');
      // One filled button; the ink one is the other door.
      await expect(page.getByRole('button', { name: 'Create my account and join' })).toBeVisible();
      await expect(page.getByRole('button', { name: 'I already have an account' })).toBeVisible();
      // The link left the address bar, the code is carried, and the card was read once.
      await expect(page).toHaveURL((url) => url.pathname === '/');
      expect(await carried(page)).toBe(CODE);
      expect(reads).toHaveLength(1);
      await page.screenshot({ path: `test-results/invite-card/${theme}-live.png`, fullPage: true });
    });

    test(`an expired link says so and carries on (${theme})`, async ({ page }) => {
      await pinTheme(page, theme);
      await openLink(page, { ...LIVE, status: 'expired' });

      await expect(page.getByTestId('invite-card-dead')).toHaveText(
        'This link has expired. Ask Jeremy B. to send you a new one.',
      );
      await expect(page.getByTestId('invite-card-create')).toHaveCount(0);
      // A dead link is never held.
      await expect.poll(() => carried(page)).toBeNull();
      await page.screenshot({ path: `test-results/invite-card/${theme}-expired.png`, fullPage: true });

      await page.getByRole('button', { name: 'Continue without joining' }).click();
      await expect(page.getByTestId('invite-card')).toHaveCount(0);
      await waitForLoginScreen(page);
    });
  }

  test('the filled button opens the create-account form, the code still riding with it', async ({ page }) => {
    await openLink(page, LIVE);
    await page.getByTestId('invite-card-create').click();

    await expect(page.locator('h1')).toContainText('Create Your Account');
    expect(await carried(page)).toBe(CODE);
  });

  test('the ink button opens the sign-in form', async ({ page }) => {
    await openLink(page, LIVE);
    await page.getByTestId('invite-card-have-account').click();

    await expect(page.getByTestId('invite-card')).toHaveCount(0);
    await waitForLoginScreen(page);
  });

  test('an unknown code says it does not exist, and is dropped', async ({ page }) => {
    const reads = await openLink(page, { status: 404 });

    await expect(page.getByTestId('invite-card-dead')).toContainText("This invite link doesn't exist.");
    await expect.poll(() => carried(page)).toBeNull();
    expect(reads).toHaveLength(1);
  });

  test('a rate-limited read says so and keeps the code for the retry', async ({ page }) => {
    const reads = await openLink(page, { status: 429 });

    await expect(page.getByTestId('invite-card-error')).toContainText('Too many tries.');
    expect(await carried(page)).toBe(CODE);
    expect(reads).toHaveLength(1);
  });
});

test.describe('Home — the held-invite strip', () => {
  test('a full group shows the strip once, with no toast, and the retry opens the room', async ({ page }) => {
    await setupDashboardMocks(page, { role: 'user' });
    const held = {
      kind: 'member',
      group_name: 'Morning Riders',
      inviter_name: 'Jeremy B.',
      inviter_initials: 'JB',
      has_coach: true,
      coach_name: 'Jeremy B.',
      held_at: '2026-11-10T10:00:00Z',
      state: 'group_full',
    };
    let seatOpen = false;
    await page.route('**/api/user/group-invite', (route) =>
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ held: seatOpen ? null : held }),
      }),
    );
    const joins: string[] = [];
    await page.route('**/api/chat/groups/join', async (route) => {
      const body = seatOpen
        ? { outcome: 'joined', kind: 'member', group_id: 'group-1', group_name: 'Morning Riders', conversation_id: 'conv-room' }
        : { outcome: 'refused', reason: 'group_full', message: 'Ce groupe est complet.' };
      joins.push(String(body.outcome));
      await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(body) });
    });
    await loginToDashboard(page);
    await openHome(page);

    const strip = page.getByTestId('held-invite-strip');
    await expect(strip).toBeVisible();
    await expect(strip).toContainText('Jeremy B. invites you to “Morning Riders”.');
    await expect(page.getByTestId('held-invite-strip-full')).toContainText("Jeremy B.'s group is full for now.");
    // The page's own join was refused and said nothing in a toast.
    await expect(page.getByRole('alert').filter({ hasText: 'complet' })).toHaveCount(0);
    expect(joins).toEqual(['refused']);

    seatOpen = true;
    await strip.getByRole('button', { name: 'Try again' }).click();
    await expect(page).toHaveURL(/#chat\/conv-room$/, { timeout: 10_000 });
    expect(joins).toEqual(['refused', 'joined']);
  });
});

test.describe('Coach Home — the invite funnel', () => {
  for (const theme of THEMES) {
    test(`a pending invite row says how far the link got (${theme})`, async ({ page }) => {
      await pinTheme(page, theme);
      await page.route('**/api/groups/g-lea/invites', (route) =>
        route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({
            invites: [
              {
                id: 'inv-1',
                group_id: 'g-lea',
                tenant_id: 't-1',
                code: 'ABCD2345',
                kind: 'member',
                created_by: 'user-123',
                expires_at: '2099-12-13T14:40:00Z',
                max_uses: null,
                use_count: 0,
                is_active: true,
                approves_signup: false,
                created_at: '2026-11-13T14:40:00Z',
                funnel: {
                  open_count: 3,
                  last_opened_at: '2026-11-14T10:00:00Z',
                  held_count: 1,
                  last_held_at: '2026-11-15T10:00:00Z',
                  full_count: 1,
                },
              },
            ],
          }),
        }),
      );
      await mockCoach(page, {
        roster: (roster) => ({
          ...roster,
          athletes: [],
          groups: [{ id: 'g-lea', name: 'Léa · Marathon de Montréal', athlete_count: 0 }],
        }),
      });

      const row = page.getByTestId('coach-pending-invite-g-lea');
      await expect(row).toBeVisible();
      const funnel = row.getByTestId('coach-pending-invite-funnel');
      await expect(funnel.getByTestId('coach-pending-invite-step-opened')).toContainText('Opened · 3 times');
      await expect(funnel.getByTestId('coach-pending-invite-step-account')).toContainText('Account created');
      await expect(funnel.getByTestId('coach-pending-invite-step-joined')).toHaveAttribute('data-tone', 'warn');
      await expect(funnel.getByTestId('coach-pending-invite-step-joined')).toContainText('Group full');
      // Counts only: the athlete behind the account is not named.
      await expect(row).not.toContainText('@');
      await row.screenshot({ path: `test-results/invite-card/${theme}-funnel-row.png` });
    });
  }
});
