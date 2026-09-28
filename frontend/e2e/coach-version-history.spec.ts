// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Playwright E2E tests for an agent's version history in Discover's edit sheet — list, compare with current, revert
// ABOUTME: A mocked server applies the real history rules: an edit snapshots what it replaces, a revert does too

import { test, expect, type Page } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard, navigateToTab, APP_SHELL_TIMEOUT_MS } from './test-helpers';

const STORE_ID = 'store-tempo';
const COACH_ID = 'coach-tempo';
const COACH_HANDLE = 'tempo-coach';

interface Content {
  title: string;
  system_prompt: string;
}

interface HistoryMocks {
  /** Version numbers the editor asked to revert to, oldest first. */
  reverts: number[];
  /** Versions the editor asked to compare with the current content. */
  diffs: number[];
  current: () => Content;
}

const storeListing = {
  id: STORE_ID,
  title: 'Tempo Coach C',
  description: 'Threshold work and race-week sharpening',
  category: 'training',
  tags: ['tempo'],
  sample_prompts: [],
  token_count: 40,
  install_count: 3,
  icon_url: null,
  published_at: '2026-07-01T00:00:00Z',
  author_id: 'author-1',
  handle: COACH_HANDLE,
};

/**
 * One installed agent edited twice: versions 1 and 2 hold the content each
 * edit replaced, and the live agent is the third content. A revert snapshots
 * the live content as the next version before restoring, as the server does.
 */
async function setupHistoryMocks(page: Page): Promise<HistoryMocks> {
  const snapshots: Array<{ version: number; content: Content; at: string; by: string | null }> = [
    { version: 1, content: { title: 'Tempo Coach', system_prompt: 'prompt one' }, at: '2026-09-10T08:30:00Z', by: null },
    { version: 2, content: { title: 'Tempo Coach B', system_prompt: 'prompt two' }, at: '2026-09-20T08:30:00Z', by: 'Ada Lovelace' },
  ];
  let current: Content = { title: 'Tempo Coach C', system_prompt: 'prompt three' };
  const reverts: number[] = [];
  const diffs: number[] = [];

  await setupDashboardMocks(page, { role: 'user' });

  const snapshotOf = (content: Content) => ({
    ...content,
    description: null,
    category: 'training',
    tags: ['tempo'],
    sample_prompts: [],
    token_count: 10,
    visibility: 'private',
  });

  const coachPayload = () => ({
    id: COACH_ID,
    title: current.title,
    description: 'Threshold work and race-week sharpening',
    system_prompt: current.system_prompt,
    category: 'Training',
    tags: ['tempo'],
    token_count: 40,
    is_favorite: false,
    use_count: 4,
    last_used_at: '2026-08-01T10:00:00Z',
    created_at: '2026-07-01T00:00:00Z',
    updated_at: '2026-08-01T10:00:00Z',
    is_system: false,
    visibility: 'private',
    is_assigned: true,
    forked_from: STORE_ID,
    handle: COACH_HANDLE,
  });

  const json = (body: unknown) => ({ status: 200, contentType: 'application/json', body: JSON.stringify(body) });
  const metadata = () => ({ timestamp: new Date().toISOString(), api_version: '1.0' });

  await page.route(/\/api\/store\/agents(\?.*)?$/, (route) =>
    route.fulfill(json({ agents: [storeListing], has_more: false, next_cursor: null, metadata: metadata() })),
  );
  await page.route(`**/api/store/agents/${STORE_ID}`, (route) =>
    route.fulfill(
      json({ ...storeListing, system_prompt: 'prompt three', created_at: '2026-07-01T00:00:00Z', publish_status: 'published' }),
    ),
  );
  await page.route(/\/api\/agents(\?.*)?$/, async (route) => {
    if (route.request().method() !== 'GET') {
      await route.fallback();
      return;
    }
    await route.fulfill(json({ agents: [coachPayload()], total: 1 }));
  });
  await page.route(/\/api\/agents\/[^/?]+(\?.*)?$/, (route) => route.fulfill(json(coachPayload())));

  await page.route(`**/api/agents/${COACH_ID}/versions`, (route) =>
    route.fulfill(
      json({
        versions: [...snapshots].reverse().map((s) => ({
          version: s.version,
          content_snapshot: snapshotOf(s.content),
          change_summary: null,
          created_at: s.at,
          created_by_name: s.by,
        })),
        current_version: snapshots.length,
        total: snapshots.length,
      }),
    ),
  );

  await page.route(new RegExp(`/api/agents/${COACH_ID}/versions/(\\d+)/diff$`), async (route) => {
    const version = Number(/versions\/(\d+)\/diff/.exec(route.request().url())?.[1]);
    diffs.push(version);
    const stored = snapshots.find((s) => s.version === version)!.content;
    const changes = (['title', 'system_prompt'] as const)
      .filter((field) => stored[field] !== current[field])
      .map((field) => ({ field, old_value: stored[field], new_value: current[field] }));
    await route.fulfill(json({ version, changes }));
  });

  await page.route(new RegExp(`/api/agents/${COACH_ID}/versions/(\\d+)/revert$`), async (route) => {
    const version = Number(/versions\/(\d+)\/revert/.exec(route.request().url())?.[1]);
    reverts.push(version);
    snapshots.push({ version: snapshots.length + 1, content: current, at: '2026-09-27T12:00:00Z', by: 'Ada Lovelace' });
    current = { ...snapshots.find((s) => s.version === version)!.content };
    await route.fulfill(
      json({ agent: coachPayload(), reverted_to_version: version, new_version: snapshots.length }),
    );
  });

  return { reverts, diffs, current: () => current };
}

async function openEditSheet(page: Page) {
  await navigateToTab(page, 'Discover');
  const listing = page.getByTestId('store-coach-grid').getByText('Tempo Coach C');
  await expect(listing).toBeVisible({ timeout: APP_SHELL_TIMEOUT_MS });
  await listing.click();
  await page.getByRole('button', { name: 'Edit agent' }).click();
  await expect(page.getByRole('heading', { name: 'Edit Agent' })).toBeVisible({ timeout: 5000 });
}

test.describe('Agent version history', () => {
  test('lists the versions, compares one with the current content, and reverts to it', async ({ page }) => {
    const mocks = await setupHistoryMocks(page);
    await loginToDashboard(page);
    await openEditSheet(page);

    const history = page.getByTestId('agent-version-history');
    await expect(history.getByTestId('agent-version-2')).toBeVisible();
    await expect(history.getByTestId('agent-version-1')).toBeVisible();
    await expect(history.getByTestId('agent-version-2').getByTestId('agent-version-meta')).toContainText('Ada Lovelace');

    // Compare version 1 with what the agent is now.
    await history.getByTestId('agent-version-compare-1').click();
    const diff = history.getByTestId('agent-version-diff-1');
    const title = diff.getByTestId('agent-version-change-title');
    await expect(title.getByTestId('agent-version-old-value')).toHaveText('Tempo Coach');
    await expect(title.getByTestId('agent-version-new-value')).toHaveText('Tempo Coach C');
    await expect(diff.getByTestId('agent-version-change-system_prompt')).toBeVisible();
    expect(mocks.diffs).toEqual([1]);

    // Revert asks first; the confirmation is the dialog's primary action.
    await history.getByTestId('agent-version-revert-1').click();
    const dialog = page.getByRole('dialog');
    await expect(dialog).toBeVisible();
    expect(mocks.reverts).toEqual([]);
    await dialog.getByRole('button').last().click();
    await expect(dialog).toBeHidden({ timeout: 5000 });
    expect(mocks.reverts).toEqual([1]);

    // The form shows the restored content, and the refreshed history holds
    // the replaced content as version 3.
    await expect(page.getByPlaceholder('e.g., Marathon Training Agent')).toHaveValue('Tempo Coach');
    await expect(history.getByTestId('agent-version-3')).toBeVisible();
    expect(mocks.current().title).toBe('Tempo Coach');
  });
});
