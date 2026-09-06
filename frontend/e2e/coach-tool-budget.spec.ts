// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Playwright E2E tests for the agent tool-iteration budget, driven through Discover's edit sheet.
// ABOUTME: Pins the three-state write contract — absent inherits, a number pins, an explicit null clears.

import { test, expect, type Page } from '@playwright/test';
import {
  MIN_MAX_TOOL_ITERATIONS,
  MAX_MAX_TOOL_ITERATIONS,
  DEFAULT_MAX_TOOL_ITERATIONS,
} from '@pierre/shared-constants';
import { setupDashboardMocks, loginToDashboard, navigateToTab, APP_SHELL_TIMEOUT_MS } from './test-helpers';

/** The store listing the athlete installed; its copy is what the edit sheet opens. */
const STORE_ID = 'store-tempo';
const COACH_ID = 'coach-tempo';
const COACH_TITLE = 'Tempo Coach';
const COACH_HANDLE = 'tempo-coach';

/**
 * The three states the coach editor can put on the wire, as the server sees
 * them. `absent` is what makes inheritance work: the stored value — or the
 * absence of one — survives an edit that never touched the budget field.
 */
type BudgetState =
  | { kind: 'absent' }
  | { kind: 'null' }
  | { kind: 'number'; value: number };

function readBudget(body: Record<string, unknown>): BudgetState {
  if (!('max_tool_iterations' in body)) return { kind: 'absent' };
  const value = body.max_tool_iterations;
  if (value === null) return { kind: 'null' };
  return { kind: 'number', value: value as number };
}

interface CoachMocks {
  /** Bodies of every PUT the editor sent, oldest first. */
  updates: Array<Record<string, unknown>>;
  /** What the fake server currently stores for the coach, `null` when it inherits. */
  storedBudget: () => number | null;
}

const storeListing = {
  id: STORE_ID,
  title: COACH_TITLE,
  description: 'Threshold work and race-week sharpening',
  category: 'training',
  tags: ['tempo'],
  sample_prompts: ['How should I pace a tempo run?'],
  token_count: 40,
  install_count: 3,
  icon_url: null,
  published_at: '2026-07-01T00:00:00Z',
  author_id: 'author-1',
  handle: COACH_HANDLE,
};

/**
 * Serve one store listing the athlete has installed, plus its personal copy,
 * and apply the server's own three-state update rule to the copy so a saved
 * budget genuinely round-trips through the re-read rather than being echoed
 * back by the mock.
 */
async function setupCoachMocks(
  page: Page,
  options: { initialBudget?: number | null } = {},
): Promise<CoachMocks> {
  const { initialBudget = null } = options;
  let stored: number | null = initialBudget;
  const updates: Array<Record<string, unknown>> = [];

  await setupDashboardMocks(page, { role: 'user' });

  const coachPayload = () => {
    const coach: Record<string, unknown> = {
      id: COACH_ID,
      title: COACH_TITLE,
      description: 'Threshold work and race-week sharpening',
      system_prompt: 'You are a tempo coach.',
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
      // The copy an install minted: the listing is what maps back to it.
      forked_from: STORE_ID,
      handle: COACH_HANDLE,
    };
    // A coach with no pin carries no key at all — exactly how the API answers,
    // and what hydrates the editor's empty box.
    if (stored !== null) {
      coach.max_tool_iterations = stored;
    }
    return coach;
  };

  const metadata = () => ({ timestamp: new Date().toISOString(), api_version: '1.0' });

  // The catalogue: one listing, no next page.
  await page.route(/\/api\/store\/coaches(\?.*)?$/, async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ coaches: [storeListing], has_more: false, next_cursor: null, metadata: metadata() }),
    });
  });
  await page.route(`**/api/store/coaches/${STORE_ID}`, async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        ...storeListing,
        system_prompt: 'You are a tempo coach.',
        created_at: '2026-07-01T00:00:00Z',
        publish_status: 'published',
      }),
    });
  });

  // List (with or without query string) — never the /api/coaches/<id> sub-path.
  await page.route(/\/api\/coaches(\?.*)?$/, async (route) => {
    if (route.request().method() !== 'GET') {
      await route.fallback();
      return;
    }
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ coaches: [coachPayload()], total: 1 }),
    });
  });

  // Single coach: the GET the sheet loads with, the PUT it sends, plus the
  // hidden-coaches sibling.
  await page.route(/\/api\/coaches\/[^/?]+(\?.*)?$/, async (route) => {
    const request = route.request();
    if (request.url().includes('/coaches/hidden')) {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ coaches: [] }),
      });
      return;
    }
    if (request.method() === 'PUT') {
      const body = request.postDataJSON() as Record<string, unknown>;
      updates.push(body);
      const budget = readBudget(body);
      if (budget.kind === 'null') {
        stored = null;
      } else if (budget.kind === 'number') {
        stored = budget.value;
      }
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify(coachPayload()),
      });
      return;
    }
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(coachPayload()),
    });
  });

  return { updates, storedBudget: () => stored };
}

/** Discover → the installed listing's detail, where "Edit agent" lives. */
async function openInstalledListing(page: Page) {
  await navigateToTab(page, 'Discover');
  // Discover is a lazy chunk: the first worker to open it waits on Vite's
  // cold transform, the same wait the app shell gets.
  const listing = page.getByTestId('store-coach-grid').getByText(COACH_TITLE);
  await expect(listing).toBeVisible({ timeout: APP_SHELL_TIMEOUT_MS });
  await listing.click();
  await expect(page.getByRole('button', { name: 'Edit agent' })).toBeVisible({ timeout: 10000 });
}

/** Open the edit sheet from the listing detail already on screen. */
async function openEditSheet(page: Page) {
  await page.getByRole('button', { name: 'Edit agent' }).click();
  await expect(page.getByRole('heading', { name: 'Edit Agent' })).toBeVisible({ timeout: 5000 });
}

function budgetInput(page: Page) {
  return page.locator('#max-tool-iterations');
}

async function saveCoach(page: Page) {
  // Arm the wait before the click: the save invalidates the coaches query, and
  // the reopened editor must hydrate from that refetch rather than from state
  // the mutation left behind. Awaiting the refetch is what makes the
  // round-trip assertions a genuine server round-trip.
  const listRefetched = page.waitForResponse(
    (response) =>
      response.request().method() === 'GET' && /\/api\/coaches(\?.*)?$/.test(response.url()),
    { timeout: 10000 },
  );
  await page.getByRole('button', { name: 'Save Changes' }).click();
  // The sheet closes only on a successful mutation, so its disappearance is
  // the signal that the PUT completed rather than an arbitrary sleep.
  await expect(page.getByRole('heading', { name: 'Edit Agent' })).toBeHidden({ timeout: 10000 });
  await listRefetched;
  await page.waitForTimeout(300);
}

test.describe('Agent tool budget - explicit value', () => {
  test('saving an explicit budget sends the number and it round-trips on reopen', async ({ page }) => {
    const mocks = await setupCoachMocks(page, { initialBudget: null });
    await loginToDashboard(page);
    await openInstalledListing(page);
    await openEditSheet(page);

    // A coach with no pin hydrates an empty box that advertises the workspace
    // limit as its placeholder.
    await expect(budgetInput(page)).toHaveValue('');
    await expect(budgetInput(page)).toHaveAttribute(
      'placeholder',
      String(DEFAULT_MAX_TOOL_ITERATIONS),
    );

    await budgetInput(page).fill('30');
    await saveCoach(page);

    expect(mocks.updates).toHaveLength(1);
    expect(readBudget(mocks.updates[0])).toEqual({ kind: 'number', value: 30 });
    expect(mocks.storedBudget()).toBe(30);

    // Reopen: the value came back from the re-read coach, not from local state.
    await openEditSheet(page);
    await expect(budgetInput(page)).toHaveValue('30');
  });

  test('the budget box holds the 1..=50 bounds the server enforces', async ({ page }) => {
    await setupCoachMocks(page, { initialBudget: 10 });
    await loginToDashboard(page);
    await openInstalledListing(page);
    await openEditSheet(page);

    await expect(budgetInput(page)).toHaveValue('10');

    await budgetInput(page).fill('99');
    await expect(budgetInput(page)).toHaveValue(String(MAX_MAX_TOOL_ITERATIONS));

    await budgetInput(page).fill('0');
    await expect(budgetInput(page)).toHaveValue(String(MIN_MAX_TOOL_ITERATIONS));
  });
});

test.describe('Agent tool budget - inherit guarantee', () => {
  test('an edit that never touches the budget omits the key entirely', async ({ page }) => {
    const mocks = await setupCoachMocks(page, { initialBudget: null });
    await loginToDashboard(page);
    await openInstalledListing(page);
    await openEditSheet(page);

    await expect(budgetInput(page)).toHaveValue('');

    // Touch an unrelated field only.
    await page
      .getByPlaceholder('Brief description of what this agent specializes in')
      .fill('Threshold work, race-week sharpening');
    await saveCoach(page);

    expect(mocks.updates).toHaveLength(1);
    const body = mocks.updates[0];
    expect(body.description).toBe('Threshold work, race-week sharpening');
    // The key must be ABSENT, not present-and-undefined: an omitted field is
    // what tells the server to leave the coach inheriting the tenant-wide
    // tool_execution.max_iterations.
    expect(Object.keys(body)).not.toContain('max_tool_iterations');
    expect(readBudget(body)).toEqual({ kind: 'absent' });
    expect(mocks.storedBudget()).toBeNull();
  });

  test('an edit that never touches an EXISTING pin re-sends that pin, never the default', async ({ page }) => {
    const mocks = await setupCoachMocks(page, { initialBudget: 25 });
    await loginToDashboard(page);
    await openInstalledListing(page);
    await openEditSheet(page);

    // The box hydrates from the stored pin, so the value the editor carries is
    // the coach's own — not the workspace default the placeholder advertises.
    await expect(budgetInput(page)).toHaveValue('25');

    await page
      .getByPlaceholder('Brief description of what this agent specializes in')
      .fill('Sharpening only');
    await saveCoach(page);

    expect(mocks.updates).toHaveLength(1);
    expect(readBudget(mocks.updates[0])).toEqual({ kind: 'number', value: 25 });
    expect(mocks.updates[0].max_tool_iterations).not.toBe(DEFAULT_MAX_TOOL_ITERATIONS);
    expect(mocks.storedBudget()).toBe(25);

    await openEditSheet(page);
    await expect(budgetInput(page)).toHaveValue('25');
  });
});

test.describe('Agent tool budget - clearing', () => {
  test('clearing a pinned budget sends an explicit null and returns the agent to inherit', async ({ page }) => {
    const mocks = await setupCoachMocks(page, { initialBudget: 12 });
    await loginToDashboard(page);
    await openInstalledListing(page);
    await openEditSheet(page);

    await expect(budgetInput(page)).toHaveValue('12');
    await budgetInput(page).fill('');
    await saveCoach(page);

    expect(mocks.updates).toHaveLength(1);
    const body = mocks.updates[0];
    // Key PRESENT, value null — the only shape that clears a stored pin.
    expect(Object.keys(body)).toContain('max_tool_iterations');
    expect(body.max_tool_iterations).toBeNull();
    expect(readBudget(body)).toEqual({ kind: 'null' });
    expect(mocks.storedBudget()).toBeNull();

    await openEditSheet(page);
    await expect(budgetInput(page)).toHaveValue('');
    await expect(budgetInput(page)).toHaveAttribute(
      'placeholder',
      String(DEFAULT_MAX_TOOL_ITERATIONS),
    );
  });
});

test.describe('Agent authoring surface - the edit sheet is the only agent editor', () => {
  /** Step chrome that only the deleted seven-step CoachWizard ever rendered. */
  const WIZARD_ONLY_CHROME = [
    'Basic Info',
    'Prerequisites',
    'Review Your Coach',
    'Required Providers',
    'Minimum Activities',
  ];

  test('the edit sheet is a single screen with no wizard step chrome and no version history', async ({ page }) => {
    await setupCoachMocks(page, { initialBudget: null });
    await loginToDashboard(page);
    await openInstalledListing(page);
    await openEditSheet(page);

    // The surviving editor: one screen, submitted in one action, with the
    // agent's deletion under it.
    await expect(page.getByRole('button', { name: 'Save Changes' })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Delete this agent' })).toBeVisible();
    await expect(budgetInput(page)).toBeVisible();

    for (const chrome of WIZARD_ONLY_CHROME) {
      await expect(page.getByText(chrome, { exact: true })).toHaveCount(0);
    }
    await expect(page.getByPlaceholder('Enter agent title')).toHaveCount(0);
    await expect(page.getByRole('button', { name: 'Next' })).toHaveCount(0);
    await expect(page.getByText(/Version History/i)).toHaveCount(0);
  });

  test('Discover offers no agent creation: agents are created with /agent create', async ({ page }) => {
    await setupCoachMocks(page, { initialBudget: null });
    await loginToDashboard(page);

    // A hand-typed wizard route resolves to a real tab instead of mounting one.
    await page.goto('/#coach-wizard');
    await page.waitForSelector('main', { timeout: 20000 });
    for (const chrome of WIZARD_ONLY_CHROME) {
      await expect(page.getByText(chrome, { exact: true })).toHaveCount(0);
    }

    await navigateToTab(page, 'Discover');
    await expect(page.getByTestId('store-coach-grid')).toBeVisible({ timeout: APP_SHELL_TIMEOUT_MS });
    await expect(page.getByRole('region', { name: /Your agents/ })).toHaveCount(0);
    await expect(page.getByRole('button', { name: 'Create Agent' })).toHaveCount(0);
    await expect(page.getByRole('button', { name: 'Import Agent' })).toHaveCount(0);
    await expect(page.getByRole('heading', { name: 'Create Custom Agent' })).toHaveCount(0);
  });
});
