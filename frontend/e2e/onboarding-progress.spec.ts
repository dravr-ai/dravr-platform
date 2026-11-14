// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: E2E for the onboarding progress bar — one total per journey, a skipped step kept as skipped, readable labels
// ABOUTME: Walks an athlete past a skipped Connect and a coach past a skipped seat, in English and French, light and dark

import { test, expect, type Page, type Route } from '@playwright/test';
import { setupDashboardMocks, signInThroughHostedPage } from './test-helpers';

const json = (route: Route, body: unknown) =>
  route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(body) });

interface Start {
  /** Per-user onboarding flags already set on this device. */
  flags: string[];
  status: Record<string, unknown>;
  language?: 'en' | 'fr';
  theme?: 'light' | 'dark';
}

const PARQ = [
  'heart_condition',
  'chest_pain',
  'dizziness',
  'chronic_condition',
  'medication',
  'joint_problem',
  'supervised_only',
].map((id, i) => ({ id, text: `PAR-Q question ${i + 1}` }));

/** Signs in with the onboarding state given; the step writes answer 204. */
async function startOnboarding(page: Page, { flags, status, language = 'en', theme = 'light' }: Start) {
  await page.addInitScript(
    ({ keys, lang, scheme }) => {
      window.localStorage.setItem('dravr.theme', scheme);
      window.localStorage.setItem('pierre_app_language', lang);
      for (const key of keys) window.localStorage.setItem(`${key}.user-123`, '1');
    },
    { keys: flags, lang: language, scheme: theme },
  );
  await setupDashboardMocks(page, { role: 'user', email: 'lea@test.com', displayName: 'Léa', language });
  await page.route('**/api/me/onboarding-status', (route) => json(route, status));
  await page.route('**/api/me/onboarding/steps/**', (route) => route.fulfill({ status: 204, body: '' }));
  // Santé reads its seven questions; without them the screen puts itself off.
  await page.route('**/api/me/parq', (route) => json(route, { questions: PARQ }));
  await page.goto('/');
  await signInThroughHostedPage(page);
}

const ATHLETE = {
  flags: ['dravr.profile_type_chosen'],
  status: {
    needs_provider_connection: true,
    steps: [{ step_id: 'profile_type', status: 'complete' }],
    chosen_channel: null,
    coaches_others: false,
  },
};

const COACH = {
  flags: ['dravr.profile_type_chosen', 'dravr.athlete_steps_waived'],
  status: {
    needs_provider_connection: true,
    steps: [
      { step_id: 'profile_type', status: 'complete' },
      { step_id: 'parq', status: 'not_applicable' },
    ],
    chosen_channel: null,
    coaches_others: true,
  },
};

/** The progress landmark, found by the name that says where the user is. */
const bar = (page: Page, name: string | RegExp) => page.getByRole('navigation', { name });

test('an athlete who continues without connecting keeps four steps, Connect drawn as skipped', async ({ page }) => {
  await startOnboarding(page, ATHLETE);

  await expect(bar(page, 'Step 2 of 4: Your data')).toBeVisible({ timeout: 15_000 });
  await page.getByRole('button', { name: 'Continue without connecting' }).click();

  const onHealth = bar(page, 'Step 3 of 4: Health');
  // The skip is written to the server before the health screen draws; on a
  // loaded runner that outlasts the default 5 s, so wait as the first step does.
  await expect(onHealth).toBeVisible({ timeout: 15_000 });
  await expect(onHealth.locator('li')).toHaveCount(4);
  await expect(onHealth.locator('li').nth(1)).toHaveAttribute('data-status', 'skipped');
  await expect(onHealth.locator('li[data-status="skipped"] .border-dashed')).toHaveCount(1);
});

test('a coach who skips the coach seat keeps four steps, the seat kept behind them', async ({ page }) => {
  await startOnboarding(page, COACH);

  await expect(bar(page, 'Step 2 of 4: Coach seat')).toBeVisible({ timeout: 15_000 });
  await page.getByRole('button', { name: 'Skip for now' }).click();

  const onGroup = bar(page, 'Step 3 of 4: Your group');
  await expect(onGroup).toBeVisible();
  await expect(onGroup.locator('li')).toHaveCount(4);
  // A seat put off is a device flag that is not told from one settled, and no
  // step row: it keeps its segment, read as behind the coach.
  await expect(onGroup.locator('li').nth(1)).toHaveText('Coach seat');
  await expect(onGroup.locator('li').nth(1)).toHaveAttribute('data-status', 'done');
  await expect(onGroup.locator('li').nth(3)).toHaveText('Invite');
});

test('the bar names the screens in French, and says the position under 1024px', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await startOnboarding(page, { ...COACH, language: 'fr' });

  const onSeat = bar(page, 'Étape 2 sur 4 : Place de coach');
  await expect(onSeat).toBeVisible({ timeout: 15_000 });
  await expect(onSeat.getByTestId('onboarding-current-step')).toHaveText('Étape 2 sur 4 : Place de coach');
  await expect(onSeat.getByTestId('onboarding-current-step')).toBeVisible();
});

/** WCAG contrast of two `rgb(r, g, b)` colours. */
function contrast(a: string, b: string): number {
  const luminance = (rgb: string) => {
    const [r, g, bl] = (rgb.match(/\d+(\.\d+)?/g) ?? []).slice(0, 3).map(Number).map((c) => {
      const s = c / 255;
      return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
    });
    return 0.2126 * r + 0.7152 * g + 0.0722 * bl;
  };
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

for (const theme of ['light', 'dark'] as const) {
  test(`every step label reads at 4.5:1 or better on the paper ground — ${theme}`, async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await startOnboarding(page, { ...ATHLETE, theme });

    const onTraining = bar(page, 'Step 2 of 4: Your data');
    await expect(onTraining).toBeVisible({ timeout: 15_000 });
    const ground = await page.evaluate(() => getComputedStyle(document.body).backgroundColor);
    const labels = onTraining.locator('li span.text-xs');
    await expect(labels).toHaveCount(4);
    for (const color of await labels.evaluateAll((els) => els.map((el) => getComputedStyle(el).color))) {
      expect(contrast(color, ground), `${color} on ${ground}`).toBeGreaterThanOrEqual(4.5);
    }
    const counter = await onTraining.getByTestId('onboarding-step-counter').evaluate((el) => getComputedStyle(el).color);
    expect(contrast(counter, ground)).toBeGreaterThanOrEqual(4.5);
  });
}
