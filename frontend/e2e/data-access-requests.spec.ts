// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Follows a data access request on the privacy pane — written, its letter shown, marked sent, and the due date the server counted
// ABOUTME: Pins the bodies each step sends, the fallback note to the import screen, and the section in both themes (carnet#1646)

import { test, expect, type Page, type Route } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard } from './test-helpers';

const ID = '6f1c0e4a-2b7d-4c49-9a43-0d6b1e5f2a10';

function json(route: Route, status: number, body: unknown): Promise<void> {
  return route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });
}

/** A request as the server sends it, at the stage `status` names. */
function request(status: 'drafted' | 'sent', extra: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    id: ID,
    reference: 'DRV-2026-AB3XZ7',
    controller: 'Acme Fitness',
    controller_contact: 'privacy@acme.example',
    athlete_identifier: 'ana@example.com',
    law: 'quebec_p39_1',
    scope: 'access_portability',
    letter_locale: 'en',
    status,
    derived_state: null,
    sent_on: null,
    due_on: null,
    extended_until: null,
    answered_on: null,
    created_at: '2026-11-18T15:00:00Z',
    updated_at: '2026-11-18T15:00:00Z',
    ...extra,
  };
}

const LETTER = {
  locale: 'en',
  subject: 'Request for access to personal information — reference DRV-2026-AB3XZ7',
  body: [
    'Date: 18 November 2026',
    'To the person in charge of the protection of personal information at Acme Fitness,',
    'I am making a request for access to my personal information under the Loi sur la protection des renseignements personnels dans le secteur privé (RLRQ, c. P-39.1), art. 27.',
    'Please confirm whether you hold personal information about me and, if you do, send me a copy of it.',
    'Sincerely,\nAna',
  ].join('\n\n'),
  text: '',
  filename: 'DRV-2026-AB3XZ7.txt',
  mailto: 'mailto:privacy@acme.example?subject=Request&body=Date',
};
LETTER.text = `${LETTER.subject}\n\n${LETTER.body}\n`;

/** Signs in and opens Settings → Privacy, where the section lives. */
async function openPrivacy(page: Page): Promise<void> {
  await loginToDashboard(page);
  await page.getByRole('button', { name: 'Settings', exact: true }).first().click();
  await page.getByTestId('settings-menu-privacy').click();
  await expect(page.getByTestId('data-access-section')).toBeVisible();
}

test.describe('Data access requests', () => {
  test('written, its letter shown, marked sent, and its due date shown', async ({ page }) => {
    await setupDashboardMocks(page, { role: 'user' });
    let stored: Record<string, unknown> | null = null;
    const created: unknown[] = [];
    const sent: unknown[] = [];
    // Registered after the dashboard mocks, so these win.
    await page.route('**/api/user/data-access-requests', (route) => {
      if (route.request().method() === 'POST') {
        created.push(route.request().postDataJSON());
        stored = request('drafted');
        return json(route, 201, stored);
      }
      return json(route, 200, { requests: stored ? [stored] : [] });
    });
    await page.route(`**/api/user/data-access-requests/${ID}/letter**`, (route) => json(route, 200, LETTER));
    await page.route(`**/api/user/data-access-requests/${ID}/sent`, (route) => {
      const body = route.request().postDataJSON() as { sent_on: string };
      sent.push(body);
      stored = request('sent', { sent_on: body.sent_on, due_on: '2026-12-18' });
      return json(route, 200, stored);
    });

    await openPrivacy(page);
    await expect(page.getByTestId('data-access-empty')).toBeVisible();
    await expect(page.getByTestId('data-access-fallback')).toContainText('Strava, Garmin, COROS, WHOOP and Apple');

    await page.getByTestId('data-access-new').click();
    await page.getByTestId('data-access-company').fill('Acme Fitness');
    await page.getByTestId('data-access-contact').fill('privacy@acme.example');
    await page.getByTestId('data-access-identifier').fill('ana@example.com');
    await page.getByTestId('data-access-submit').click();

    expect(created).toEqual([
      {
        controller: 'Acme Fitness',
        controller_contact: 'privacy@acme.example',
        athlete_identifier: 'ana@example.com',
        law: 'quebec_p39_1',
        scope: 'access_portability',
        letter_locale: 'en',
      },
    ]);
    const letter = page.getByTestId('data-access-letter');
    await expect(letter).toContainText('reference DRV-2026-AB3XZ7');
    await expect(letter).toContainText('P-39.1');
    await expect(page.getByTestId('data-access-mailto')).toHaveAttribute('href', LETTER.mailto);
    await expect(page.getByTestId('data-access-detail-status')).toHaveText('Draft');

    await page.getByTestId('data-access-mark-sent').click();
    // The day sent starts on today, editable.
    const day = page.getByTestId('data-access-step-date');
    const today = await day.inputValue();
    expect(today).toMatch(/^\d{4}-\d{2}-\d{2}$/);
    await page.getByTestId('data-access-step-confirm').click();

    await expect.poll(() => sent).toEqual([{ sent_on: today }]);
    await expect(page.getByTestId('data-access-dates')).toContainText('Due December 18, 2026');
    await expect(page.getByTestId('data-access-detail-status')).toHaveText('Sent');
    await expect(page.getByTestId(`data-access-row-${ID}`)).toContainText('Due Dec 18, 2026');
    // A Quebec request has no extension under P-39.1.
    await expect(page.getByTestId('data-access-record-extension')).toHaveCount(0);
  });

  test('a form with nothing in it names each field and sends nothing', async ({ page }) => {
    await setupDashboardMocks(page, { role: 'user' });
    let posted = false;
    await page.route('**/api/user/data-access-requests', (route) => {
      if (route.request().method() === 'POST') posted = true;
      return json(route, 200, { requests: [] });
    });

    await openPrivacy(page);
    await page.getByTestId('data-access-new').click();
    await page.getByTestId('data-access-contact').fill('privacy@acme');
    await page.getByTestId('data-access-submit').click();

    await expect(page.getByText("Enter the company's name.")).toBeVisible();
    await expect(page.getByText('Enter an email address, like privacy@company.com.')).toBeVisible();
    await expect(page.getByText('Enter the identifier you use with that company.')).toBeVisible();
    expect(posted).toBe(false);
  });

  for (const theme of ['light', 'dark'] as const) {
    test(`the section reads in the ${theme} theme`, async ({ page }) => {
      await page.addInitScript((value) => window.localStorage.setItem('dravr.theme', value), theme);
      await setupDashboardMocks(page, { role: 'user' });
      const overdue = request('sent', {
        law: 'gdpr',
        derived_state: 'overdue',
        sent_on: '2026-10-01',
        due_on: '2026-11-01',
      });
      const soon = request('sent', {
        id: 'b2',
        controller: 'Strava',
        derived_state: 'due_soon',
        sent_on: '2026-10-20',
        due_on: '2026-11-19',
      });
      const draft = request('drafted', { id: 'c3', controller: 'Garmin' });
      await page.route('**/api/user/data-access-requests', (route) =>
        json(route, 200, { requests: [overdue, soon, draft] }),
      );
      await page.route(`**/api/user/data-access-requests/${ID}/letter**`, (route) => json(route, 200, LETTER));

      await openPrivacy(page);
      await page.getByTestId(`data-access-row-${ID}`).click();
      await expect(page.getByTestId('data-access-letter')).toBeVisible();
      await expect(page.getByTestId('data-access-recourse')).toContainText('supervisory authority');
      const html = page.locator('html');
      await (theme === 'dark' ? expect(html).toHaveClass(/\bdark\b/) : expect(html).not.toHaveClass(/\bdark\b/));
      // The settings pane scrolls inside itself, so each part is brought into view and shot on its own.
      const list = page.getByTestId('data-access-list');
      await list.scrollIntoViewIfNeeded();
      await list.screenshot({ path: `test-results/data-access-requests-list-${theme}.png` });
      const detail = page.getByTestId('data-access-detail');
      await detail.scrollIntoViewIfNeeded();
      await detail.screenshot({ path: `test-results/data-access-requests-detail-${theme}.png` });
    });
  }
});
