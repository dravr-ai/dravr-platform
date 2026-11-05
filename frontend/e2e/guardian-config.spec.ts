// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Playwright coverage of the Guardian config tab's save contract against a mocked server
// ABOUTME: A high-impact save lists its effects and asks twice, and a stale save reloads the form

import { test, expect, type Page } from '@playwright/test';
import { setupAndLoginAsSuperAdmin, navigateToTab } from './test-helpers';

const CONFIRM_NAME = 'Confirm and save';
const STAMP = '2026-11-04T12:00:00.123456+00:00';
const POLICY = {
  mode: 'enforce',
  max_destructive_per_turn: 1,
  max_writes_per_turn: 50,
  external_send: 'none',
  tainted_destructive: 'log',
  plan_mode: 'off',
};
const SOURCES = Object.fromEntries(Object.keys(POLICY).map((key) => [key, 'persisted']));
const RESPONSE = {
  config: { schema_version: 1, ...POLICY },
  effective: POLICY,
  defaults: POLICY,
  sources: SOURCES,
  env_pinned: [],
  source: 'persisted',
  updated_at: STAMP,
  loaded_at: STAMP,
  instance_id: 'dravr-00042-abc',
  in_sync: true,
  degraded: false,
  degraded_reason: null,
  plan_mode_armable: false,
};

/** Mock every route the tab reads, then open it. Each PUT body lands in the returned list. */
async function openTab(page: Page, answerPut: (body: Record<string, unknown>) => { status: number; json: unknown }) {
  const puts: Record<string, unknown>[] = [];
  await setupAndLoginAsSuperAdmin(page);
  await page.route('**/api/admin/settings/guardian', async (route) => {
    if (route.request().method() !== 'PUT') {
      await route.fulfill({ json: RESPONSE });
      return;
    }
    const body = route.request().postDataJSON() as Record<string, unknown>;
    puts.push(body);
    const { status, json } = answerPut(body);
    await route.fulfill({ status, json });
  });
  await page.route('**/api/admin/settings/guardian/pending**', (route) =>
    route.fulfill({ json: { entries: [] } }),
  );
  await page.route('**/api/admin/audit-events**', (route) =>
    route.fulfill({ json: { events: [], next_before: null } }),
  );
  await navigateToTab(page, 'Guardian Config');
  await expect(page.getByText('dravr-00042-abc')).toBeVisible();
  return puts;
}

for (const theme of ['light', 'dark'] as const) {
  test.describe(`Guardian config tab (${theme})`, () => {
    test.beforeEach(async ({ page }) => {
      await page.addInitScript((t) => window.localStorage.setItem('dravr.theme', t), theme);
    });

    test('a high-impact save lists its effect and lands only on the second click', async ({ page }) => {
      const puts = await openTab(page, (body) =>
        body.acknowledge_impact
          ? { status: 200, json: RESPONSE }
          : {
              status: 409,
              json: {
                code: 'guardian_config_impact_not_acknowledged',
                message: 'This save changes what athletes can do.',
                impacted_fields: [
                  { field: 'mode', from: 'enforce', to: 'off', effect: 'The Guardian stops blocking anything.' },
                ],
              },
            },
      );
      await page.getByLabel('Mode', { exact: false }).first().selectOption('off');
      await page.getByRole('button', { name: 'Save changes' }).click();
      await expect(page.getByText('The Guardian stops blocking anything.')).toBeVisible();
      await page.screenshot({ path: test.info().outputPath(`impact-${theme}.png`), fullPage: true });
      expect(puts).toHaveLength(1);
      expect(puts[0].acknowledge_impact).toBeFalsy();
      expect(puts[0].expected_updated_at).toBe(STAMP);

      await page.getByRole('button', { name: CONFIRM_NAME }).click();
      await expect.poll(() => puts.length).toBe(2);
      expect(puts[1].acknowledge_impact).toBe(true);
    });

    test('a stale save names the refusal and sends the token verbatim', async ({ page }) => {
      const puts = await openTab(page, () => ({
        status: 409,
        json: { code: 'guardian_config_stale', message: 'Someone else saved this policy first.' },
      }));
      await page.getByLabel('Max writes per turn').fill('20');
      await page.getByRole('button', { name: 'Save changes' }).click();
      await expect(page.getByText('Someone else saved this policy first.')).toBeVisible();
      expect(puts[0].expected_updated_at).toBe(STAMP);
      await page.screenshot({ path: test.info().outputPath(`stale-${theme}.png`), fullPage: true });
    });
  });
}
