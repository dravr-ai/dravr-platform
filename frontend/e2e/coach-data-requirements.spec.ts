// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Playwright E2E tests for coach data requirements (startup_query + data_requirements)
// ABOUTME: Drives Discover's edit sheet on an installed coach and pins the PUT payload it sends

import { test, expect, type Page } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard, navigateToTab, APP_SHELL_TIMEOUT_MS } from './test-helpers';

const STORE_ID = 'store-running';
const COACH_TITLE = 'Test Running Coach';

const storeListing = {
  id: STORE_ID,
  title: COACH_TITLE,
  description: 'Coach with data requirements configured',
  category: 'training',
  tags: ['running'],
  sample_prompts: [],
  token_count: 100,
  install_count: 12,
  icon_url: null,
  published_at: '2026-01-01T00:00:00Z',
  author_id: 'author-1',
  handle: 'test-running-coach',
};

// The athlete's installed copy, with data_requirements configured
const coachWithDataReqs = {
  id: 'coach-dr-1',
  title: COACH_TITLE,
  description: 'Coach with data requirements configured',
  system_prompt: 'You are a running coach...',
  category: 'Training',
  tags: ['running'],
  token_count: 100,
  is_favorite: false,
  use_count: 5,
  last_used_at: null,
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
  is_system: false,
  visibility: 'private',
  is_assigned: true,
  forked_from: STORE_ID,
  handle: 'test-running-coach',
  startup_query: 'Analyze my weekly mileage and long run progression.',
  data_requirements: {
    activities: {
      count: 25,
      time_frame: '16w',
      mode: 'summary',
      format: 'toon',
      analysis_type: 'race_preparation',
    },
    athlete_profile: true,
  },
};

// Track PUT requests for verification
interface CapturedRequest {
  method: string;
  body: Record<string, unknown>;
}

async function setupCoachMocks(page: Page) {
  await setupDashboardMocks(page, { role: 'user' });

  const capturedRequests: CapturedRequest[] = [];
  const metadata = () => ({ timestamp: new Date().toISOString(), api_version: '1.0' });

  // The catalogue: the one listing the athlete installed.
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
        system_prompt: 'You are a running coach...',
        created_at: '2026-01-01T00:00:00Z',
        publish_status: 'published',
      }),
    });
  });

  // Mock user coaches list (matches /api/coaches and /api/coaches?...)
  await page.route(/\/api\/coaches(\?.*)?$/, async (route) => {
    if (route.request().method() === 'GET') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          coaches: [coachWithDataReqs],
          total: 1,
          metadata: metadata(),
        }),
      });
      return;
    }
    await route.fallback();
  });


  // Mock individual coach endpoints (GET/PUT/DELETE)
  await page.route(/\/api\/coaches\/[^/]+$/, async (route) => {
    const method = route.request().method();
    if (method === 'GET') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify(coachWithDataReqs),
      });
    } else if (method === 'PUT') {
      const body = route.request().postDataJSON();
      capturedRequests.push({ method: 'PUT', body });
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ ...coachWithDataReqs, ...body }),
      });
    } else if (method === 'DELETE') {
      await route.fulfill({ status: 204 });
    } else {
      await route.continue();
    }
  });

  return capturedRequests;
}

/** Discover → the installed listing → its edit sheet. */
async function openEditSheet(page: Page) {
  await navigateToTab(page, 'Discover');
  const listing = page.getByTestId('store-coach-grid').getByText(COACH_TITLE);
  await expect(listing).toBeVisible({ timeout: APP_SHELL_TIMEOUT_MS });
  await listing.click();
  await page.getByRole('button', { name: 'Edit coach' }).click();
  await expect(page.getByRole('heading', { name: 'Edit Coach' })).toBeVisible({ timeout: 5000 });
}

async function saveCoach(page: Page) {
  await page.getByRole('button', { name: 'Save Changes' }).click();
  await expect(page.getByRole('heading', { name: 'Edit Coach' })).toBeHidden({ timeout: 10000 });
}

test.describe('Coach Data Requirements', () => {
  test('edit sheet shows Data Context hydrated from the stored coach', async ({ page }) => {
    await setupCoachMocks(page);
    await loginToDashboard(page);
    await openEditSheet(page);

    // Should see Data Context section
    await expect(page.getByText('Data Context')).toBeVisible();

    // The stored startup query is what the box holds.
    await expect(page.getByPlaceholder(/What should the coach analyze on first message/)).toHaveValue(
      'Analyze my weekly mileage and long run progression.',
    );

    // Pre-fetch is on for this coach, so its structured fields are open.
    await expect(page.getByText('Pre-fetch activity data')).toBeVisible();
    await expect(page.getByText('Activity count')).toBeVisible();
    await expect(page.locator('input[type="number"]').first()).toHaveValue('25');
    await expect(page.getByText('Time frame')).toBeVisible();
  });

  test('pre-fetch toggle hides and reveals activity count, time frame, and mode fields', async ({ page }) => {
    await setupCoachMocks(page);
    await loginToDashboard(page);
    await openEditSheet(page);

    await expect(page.getByText('Activity count')).toBeVisible();

    // Turn pre-fetch off: the structured fields go with it.
    const prefetchLabel = page.getByText('Pre-fetch activity data');
    await prefetchLabel.click();
    await expect(page.getByText('Activity count')).not.toBeVisible();

    // And back on.
    await prefetchLabel.click();
    await expect(page.getByText('Activity count')).toBeVisible();
    await expect(page.getByText('Time frame')).toBeVisible();
    await expect(page.getByText('Summary')).toBeVisible();
    await expect(page.getByText('Detailed')).toBeVisible();
  });

  test('saving with data_requirements sends the structured pre-fetch payload', async ({ page }) => {
    const capturedRequests = await setupCoachMocks(page);
    await loginToDashboard(page);
    await openEditSheet(page);

    // Change the startup query and the activity count
    const startupField = page.getByPlaceholder(/What should the coach analyze on first message/);
    await startupField.fill('Analyze my recent training trends');
    const activityInput = page.locator('input[type="number"]').first();
    await activityInput.fill('30');

    await saveCoach(page);

    // Verify API request
    const updateReq = capturedRequests.find((r) => r.method === 'PUT');
    expect(updateReq).toBeDefined();
    expect(updateReq!.body.title).toBe(COACH_TITLE);
    expect(updateReq!.body.startup_query).toBe('Analyze my recent training trends');
    expect(updateReq!.body.data_requirements).toBeDefined();

    const dr = updateReq!.body.data_requirements as Record<string, unknown>;
    const activities = dr.activities as Record<string, unknown>;
    expect(activities.count).toBe(30);
    expect(activities.time_frame).toBe('16w');
    expect(activities.format).toBe('toon');
    expect(activities.mode).toBe('summary');
    expect(dr.athlete_profile).toBe(true);
  });

  test('saving with pre-fetch off sends no data_requirements', async ({ page }) => {
    const capturedRequests = await setupCoachMocks(page);
    await loginToDashboard(page);
    await openEditSheet(page);

    // Turn pre-fetch off, then save
    await page.getByText('Pre-fetch activity data').click();
    await expect(page.getByText('Activity count')).not.toBeVisible();
    await saveCoach(page);

    const updateReq = capturedRequests.find((r) => r.method === 'PUT');
    expect(updateReq).toBeDefined();
    expect(updateReq!.body.data_requirements).toBeUndefined();
    // The rest of the coach still rides along.
    expect(updateReq!.body.title).toBe(COACH_TITLE);
    expect(updateReq!.body.startup_query).toBe('Analyze my weekly mileage and long run progression.');
  });
});
