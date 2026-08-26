// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Playwright E2E tests for the coach tool-iteration budget (WAVE 0) and the deleted wizard (PHASE 0).
// ABOUTME: Pins the three-state write contract — absent inherits, a number pins, an explicit null clears.

import { test, expect, type Page } from '@playwright/test';
import {
  MIN_MAX_TOOL_ITERATIONS,
  MAX_MAX_TOOL_ITERATIONS,
  DEFAULT_MAX_TOOL_ITERATIONS,
} from '@pierre/shared-constants';
import { setupDashboardMocks, loginToDashboard } from './test-helpers';

const COACH_ID = 'coach-tempo';
const COACH_TITLE = 'Tempo Coach';

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

/**
 * Serve one editable user coach and apply the server's own three-state update
 * rule to it, so a saved budget genuinely round-trips through the list the
 * editor re-reads rather than being echoed back by the mock.
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
      is_assigned: false,
    };
    // A coach with no pin carries no key at all — exactly how the API answers,
    // and what hydrates the editor's empty box.
    if (stored !== null) {
      coach.max_tool_iterations = stored;
    }
    return coach;
  };

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

  // Single coach: the PUT the editor sends, plus the hidden-coaches sibling.
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

/** Open the surviving coach editor from the chat welcome view's coach card. */
async function openCoachEditor(page: Page) {
  await expect(page.getByText(COACH_TITLE).first()).toBeVisible({ timeout: 10000 });
  await page.getByText(COACH_TITLE).first().hover();
  await page.getByLabel('Edit coach').click();
  await expect(page.getByRole('heading', { name: 'Edit Coach' })).toBeVisible({ timeout: 5000 });
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
  // The modal closes only on a successful mutation, so its disappearance is
  // the signal that the PUT completed rather than an arbitrary sleep.
  await expect(page.getByRole('heading', { name: 'Edit Coach' })).toBeHidden({ timeout: 10000 });
  await listRefetched;
  await page.waitForTimeout(300);
}

test.describe('Coach tool budget - explicit value', () => {
  test('saving an explicit budget sends the number and it round-trips on reopen', async ({ page }) => {
    const mocks = await setupCoachMocks(page, { initialBudget: null });
    await loginToDashboard(page);
    await openCoachEditor(page);

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

    // Reopen: the value came back from the refetched list, not from local state.
    await openCoachEditor(page);
    await expect(budgetInput(page)).toHaveValue('30');
  });

  test('the budget box holds the 1..=50 bounds the server enforces', async ({ page }) => {
    await setupCoachMocks(page, { initialBudget: 10 });
    await loginToDashboard(page);
    await openCoachEditor(page);

    await expect(budgetInput(page)).toHaveValue('10');

    await budgetInput(page).fill('99');
    await expect(budgetInput(page)).toHaveValue(String(MAX_MAX_TOOL_ITERATIONS));

    await budgetInput(page).fill('0');
    await expect(budgetInput(page)).toHaveValue(String(MIN_MAX_TOOL_ITERATIONS));
  });
});

test.describe('Coach tool budget - inherit guarantee', () => {
  test('an edit that never touches the budget omits the key entirely', async ({ page }) => {
    const mocks = await setupCoachMocks(page, { initialBudget: null });
    await loginToDashboard(page);
    await openCoachEditor(page);

    await expect(budgetInput(page)).toHaveValue('');

    // Touch an unrelated field only.
    await page
      .getByPlaceholder('Brief description of what this coach specializes in')
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
    await openCoachEditor(page);

    // The box hydrates from the stored pin, so the value the editor carries is
    // the coach's own — not the workspace default the placeholder advertises.
    await expect(budgetInput(page)).toHaveValue('25');

    await page
      .getByPlaceholder('Brief description of what this coach specializes in')
      .fill('Sharpening only');
    await saveCoach(page);

    expect(mocks.updates).toHaveLength(1);
    expect(readBudget(mocks.updates[0])).toEqual({ kind: 'number', value: 25 });
    expect(mocks.updates[0].max_tool_iterations).not.toBe(DEFAULT_MAX_TOOL_ITERATIONS);
    expect(mocks.storedBudget()).toBe(25);

    await openCoachEditor(page);
    await expect(budgetInput(page)).toHaveValue('25');
  });
});

test.describe('Coach tool budget - clearing', () => {
  test('clearing a pinned budget sends an explicit null and returns the coach to inherit', async ({ page }) => {
    const mocks = await setupCoachMocks(page, { initialBudget: 12 });
    await loginToDashboard(page);
    await openCoachEditor(page);

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

    await openCoachEditor(page);
    await expect(budgetInput(page)).toHaveValue('');
    await expect(budgetInput(page)).toHaveAttribute(
      'placeholder',
      String(DEFAULT_MAX_TOOL_ITERATIONS),
    );
  });
});

test.describe('Coach authoring surface - the wizard is gone', () => {
  /** Step chrome that only the deleted seven-step CoachWizard ever rendered. */
  const WIZARD_ONLY_CHROME = [
    'Basic Info',
    'Prerequisites',
    'Review Your Coach',
    'Required Providers',
    'Minimum Activities',
  ];

  test('the coach editor is the single-screen modal, with no wizard step chrome', async ({ page }) => {
    await setupCoachMocks(page, { initialBudget: null });
    await loginToDashboard(page);
    await openCoachEditor(page);

    // The surviving editor: one screen, submitted in one action.
    await expect(page.getByRole('button', { name: 'Save Changes' })).toBeVisible();
    await expect(budgetInput(page)).toBeVisible();

    for (const chrome of WIZARD_ONLY_CHROME) {
      await expect(page.getByText(chrome, { exact: true })).toHaveCount(0);
    }
    await expect(page.getByPlaceholder('Enter coach title')).toHaveCount(0);
    await expect(page.getByRole('button', { name: 'Next' })).toHaveCount(0);
  });

  test('no route or affordance reaches a coach wizard or a version history', async ({ page }) => {
    await setupCoachMocks(page, { initialBudget: null });
    await loginToDashboard(page);

    // A hand-typed wizard route resolves to a real tab instead of mounting one.
    await page.goto('/#coach-wizard');
    await page.waitForSelector('main', { timeout: 20000 });
    for (const chrome of WIZARD_ONLY_CHROME) {
      await expect(page.getByText(chrome, { exact: true })).toHaveCount(0);
    }

    // The Coaches tab's own create form is likewise a single screen, and the
    // coach detail offers no version-history drawer.
    await page.getByRole('list').getByRole('button', { name: 'Coaches' }).click();
    await expect(page.getByText('custom AI personas')).toBeVisible({ timeout: 10000 });
    await page.getByRole('button', { name: 'Create Coach' }).click();
    await expect(page.getByRole('heading', { name: 'Create Custom Coach' })).toBeVisible({ timeout: 5000 });
    for (const chrome of WIZARD_ONLY_CHROME) {
      await expect(page.getByText(chrome, { exact: true })).toHaveCount(0);
    }
    await expect(page.getByText(/Version History/i)).toHaveCount(0);
  });
});
