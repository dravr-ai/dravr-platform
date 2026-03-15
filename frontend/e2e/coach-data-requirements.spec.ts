// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Playwright E2E tests for coach data requirements (startup_query + data_requirements)
// ABOUTME: Tests create flow with structured pre-fetch configuration on the Coaches library tab

import { test, expect, type Page } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard, navigateToTab } from './test-helpers';

// Coach with data_requirements configured
const coachWithDataReqs = {
  id: 'coach-dr-1',
  title: 'Test Running Coach',
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
  startup_query: 'Analyze my weekly mileage and long run progression.',
  data_requirements: {
    activities: {
      count: 25,
      sport_types: ['Run'],
      time_frame: '16w',
      mode: 'summary',
      format: 'toon',
      analysis_type: 'race_preparation',
    },
    athlete_profile: true,
  },
};

// Track POST/PUT requests for verification
interface CapturedRequest {
  method: string;
  body: Record<string, unknown>;
}

async function setupCoachMocks(page: Page) {
  await setupDashboardMocks(page, { role: 'user' });

  const capturedRequests: CapturedRequest[] = [];

  // Mock user coaches list (matches /api/coaches and /api/coaches?...)
  await page.route(/\/api\/coaches(\?.*)?$/, async (route) => {
    const method = route.request().method();
    if (method === 'GET') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          coaches: [coachWithDataReqs],
          total: 1,
          metadata: { timestamp: new Date().toISOString(), api_version: '1.0' },
        }),
      });
    } else if (method === 'POST') {
      const body = route.request().postDataJSON();
      capturedRequests.push({ method: 'POST', body });
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          ...body,
          id: 'new-coach-id',
          token_count: 50,
          is_favorite: false,
          use_count: 0,
          last_used_at: null,
          created_at: new Date().toISOString(),
          updated_at: new Date().toISOString(),
          is_system: false,
          visibility: 'private',
          is_assigned: true,
        }),
      });
    }
  });

  // Mock hidden coaches
  await page.route('**/api/coaches/hidden', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ coaches: [] }),
    });
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

test.describe('Coach Data Requirements', () => {
  test('create form shows Data Context section with startup query and pre-fetch toggle', async ({ page }) => {
    await setupCoachMocks(page);
    await loginToDashboard(page);
    await navigateToTab(page, 'Coaches');
    await page.waitForTimeout(500);

    // Click Create Coach button
    const createBtn = page.getByRole('button', { name: /Create Coach/i });
    await expect(createBtn.first()).toBeVisible();
    await createBtn.first().click();
    await page.waitForTimeout(300);

    // Should see Data Context section
    await expect(page.getByText('Data Context')).toBeVisible();

    // Should see startup query label
    await expect(page.getByText('Startup Query')).toBeVisible();

    // Should see pre-fetch toggle text
    await expect(page.getByText('Pre-fetch activity data')).toBeVisible();
  });

  test('pre-fetch toggle reveals activity count, time frame, sport types, and mode fields', async ({ page }) => {
    await setupCoachMocks(page);
    await loginToDashboard(page);
    await navigateToTab(page, 'Coaches');
    await page.waitForTimeout(500);

    const createBtn = page.getByRole('button', { name: /Create Coach/i });
    await createBtn.first().click();
    await page.waitForTimeout(300);

    // Activity count should NOT be visible before toggle
    await expect(page.getByText('Activity count')).not.toBeVisible();

    // Enable pre-fetch by clicking the checkbox next to "Pre-fetch activity data"
    const prefetchLabel = page.getByText('Pre-fetch activity data');
    await prefetchLabel.click();
    await page.waitForTimeout(200);

    // Now structured fields should appear
    await expect(page.getByText('Activity count')).toBeVisible();
    await expect(page.getByText('Time frame')).toBeVisible();
    await expect(page.getByText('Sport types')).toBeVisible();
    await expect(page.getByText('Summary')).toBeVisible();
    await expect(page.getByText('Detailed')).toBeVisible();
  });

  test('creating coach with data_requirements sends correct API payload', async ({ page }) => {
    const capturedRequests = await setupCoachMocks(page);
    await loginToDashboard(page);
    await navigateToTab(page, 'Coaches');
    await page.waitForTimeout(500);

    const createBtn = page.getByRole('button', { name: /Create Coach/i });
    await createBtn.first().click();
    await page.waitForTimeout(300);

    // Fill title (required) - first text input in the form
    const titleInput = page.locator('input').first();
    await titleInput.fill('My Training Coach');

    // Fill system prompt (required) - second textarea (first is description)
    const systemPromptField = page.locator('textarea').nth(1);
    await systemPromptField.fill('You are a training coach specialized in endurance.');

    // Fill startup query - third textarea (after description and system prompt)
    const startupField = page.locator('textarea').nth(2);
    await startupField.fill('Analyze my recent training trends');

    // Enable pre-fetch
    const prefetchLabel = page.getByText('Pre-fetch activity data');
    await prefetchLabel.click();
    await page.waitForTimeout(200);

    // Set activity count to 30
    const activityInput = page.locator('input[type="number"]').first();
    await activityInput.fill('30');

    // Submit the form
    const submitBtn = page.getByRole('button', { name: /Create Coach/i }).last();
    await submitBtn.click();
    await page.waitForTimeout(500);

    // Verify API request
    const createReq = capturedRequests.find(r => r.method === 'POST');
    expect(createReq).toBeDefined();
    expect(createReq!.body.title).toBe('My Training Coach');
    expect(createReq!.body.startup_query).toBe('Analyze my recent training trends');
    expect(createReq!.body.data_requirements).toBeDefined();

    const dr = createReq!.body.data_requirements as Record<string, unknown>;
    expect(dr.activities).toBeDefined();

    const activities = dr.activities as Record<string, unknown>;
    expect(activities.count).toBe(30);
    expect(activities.format).toBe('toon');
    expect(activities.mode).toBe('summary');
  });

  test('creating coach without pre-fetch sends no data_requirements', async ({ page }) => {
    const capturedRequests = await setupCoachMocks(page);
    await loginToDashboard(page);
    await navigateToTab(page, 'Coaches');
    await page.waitForTimeout(500);

    const createBtn = page.getByRole('button', { name: /Create Coach/i });
    await createBtn.first().click();
    await page.waitForTimeout(300);

    // Fill only required fields
    const titleInput = page.locator('input').first();
    await titleInput.fill('Simple Coach');
    const systemPromptField = page.locator('textarea').nth(1);
    await systemPromptField.fill('You are a simple coach.');

    // Submit without enabling pre-fetch
    const submitBtn = page.getByRole('button', { name: /Create Coach/i }).last();
    await submitBtn.click();
    await page.waitForTimeout(500);

    const createReq = capturedRequests.find(r => r.method === 'POST');
    expect(createReq).toBeDefined();
    expect(createReq!.body.data_requirements).toBeUndefined();
  });
});
