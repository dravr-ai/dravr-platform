// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Playwright E2E tests for the Analytics tab.
// ABOUTME: Tests time period selection, stats display, charts, and tool usage list.

import { test, expect, type Page } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard, navigateToTab } from './test-helpers';

// Helper to set up analytics-specific API mocks
async function setupAnalyticsMocks(
  page: Page,
  options: {
    hasData?: boolean;
  } = {}
) {
  const { hasData = true } = options;

  // Set up base dashboard mocks (includes login mock)
  await setupDashboardMocks(page, { role: 'admin' });

  // Override analytics endpoint with custom data
  await page.route('**/api/dashboard/analytics*', async (route) => {
    const url = route.request().url();
    const requestedDays = url.includes('days=7') ? 7 : url.includes('days=90') ? 90 : 30;

    if (!hasData) {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          time_series: [],
          top_tools: [],
          error_rate: 0,
          average_response_time: 0,
        }),
      });
      return;
    }

    // Generate time series data based on requested days
    const timeSeries = Array.from({ length: requestedDays }, (_, i) => {
      const date = new Date();
      date.setDate(date.getDate() - (requestedDays - i - 1));
      return {
        date: date.toISOString().split('T')[0],
        request_count: Math.floor(Math.random() * 200) + 100,
        error_count: Math.floor(Math.random() * 10),
      };
    });

    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        time_series: timeSeries,
        top_tools: [
          { tool_name: 'get_activities', request_count: 4500, success_rate: 0.989, average_response_time: 120 },
          { tool_name: 'get_athlete', request_count: 450, success_rate: 0.967, average_response_time: 85 },
          { tool_name: 'get_zones', request_count: 45, success_rate: 0.98, average_response_time: 150 },
        ],
        error_rate: 2.3,
        average_response_time: 118,
      }),
    });
  });
}

async function loginAndNavigateToAnalytics(page: Page) {
  await loginToDashboard(page);
  await navigateToTab(page, 'Analytics');
  await page.waitForTimeout(500);
}

test.describe('Analytics Tab', () => {
  test('renders Analytics tab with all main sections', async ({ page }) => {
    await setupAnalyticsMocks(page);
    await loginAndNavigateToAnalytics(page);

    // Check header
    await expect(page.locator('h1')).toContainText('Analytics');

    // Check for main sections
    await expect(page.getByText('Usage Analytics')).toBeVisible();
    await expect(page.getByText('Total Requests')).toBeVisible();
    await expect(page.getByText('Error Rate')).toBeVisible();
    await expect(page.getByText('Avg Response Time')).toBeVisible();
  });

  test('displays time period pill buttons with all options', async ({ page }) => {
    await setupAnalyticsMocks(page);
    await loginAndNavigateToAnalytics(page);

    // The time range selector uses pill buttons, not a dropdown
    const pillContainer = page.locator('.card-admin .flex.rounded-lg.bg-surface-container-high');
    await expect(pillContainer).toBeVisible();

    // Check all pill button labels
    await expect(pillContainer.getByText('7 Days')).toBeVisible();
    await expect(pillContainer.getByText('14 Days')).toBeVisible();
    await expect(pillContainer.getByText('30 Days')).toBeVisible();
    await expect(pillContainer.getByText('90 Days')).toBeVisible();
  });

  test('changes time period when pill button is clicked', async ({ page }) => {
    await setupAnalyticsMocks(page);
    await loginAndNavigateToAnalytics(page);

    // Click 7 Days pill button
    const pillContainer = page.locator('.card-admin .flex.rounded-lg.bg-surface-container-high');
    await pillContainer.getByText('7 Days').click();
    await page.waitForTimeout(500);

    // Click 90 Days pill button
    await pillContainer.getByText('90 Days').click();
    await page.waitForTimeout(500);
  });

  test('displays stat cards with correct values', async ({ page }) => {
    await setupAnalyticsMocks(page);
    await loginAndNavigateToAnalytics(page);

    // Wait for stats to load
    await expect(page.getByText('Total Requests')).toBeVisible();

    // Check error rate displays with percentage
    await expect(page.getByText('2.3%')).toBeVisible();

    // Check average response time displays with ms
    await expect(page.getByText('118ms')).toBeVisible();
  });

  test('displays Request Volume Over Time chart section', async ({ page }) => {
    await setupAnalyticsMocks(page);
    await loginAndNavigateToAnalytics(page);

    // Check chart section title
    await expect(page.getByText('Request Volume Over Time')).toBeVisible();

    // Chart should render (canvas element)
    await expect(page.locator('canvas').first()).toBeVisible();
  });

  test('displays Tool Usage Distribution chart', async ({ page }) => {
    await setupAnalyticsMocks(page);
    await loginAndNavigateToAnalytics(page);

    // Check chart section title
    await expect(page.getByText('Tool Usage Distribution')).toBeVisible();
  });

  test('displays Response Time by Tool chart', async ({ page }) => {
    await setupAnalyticsMocks(page);
    await loginAndNavigateToAnalytics(page);

    // Check chart section title
    await expect(page.getByText('Response Time by Tool')).toBeVisible();
  });

  test('displays Most Used Tools list with tool details', async ({ page }) => {
    await setupAnalyticsMocks(page);
    await loginAndNavigateToAnalytics(page);

    // Check section title - use flexible matching
    await expect(page.getByText(/Most Used|Top Tools|Tool Usage/i).first()).toBeVisible({ timeout: 10000 });

    // Check at least one tool name is displayed
    await expect(
      page.getByText('get_activities').or(page.getByText('get_athlete')).first()
    ).toBeVisible({ timeout: 10000 });
  });

  test('displays tool average response times in list', async ({ page }) => {
    await setupAnalyticsMocks(page);
    await loginAndNavigateToAnalytics(page);

    // Check average response times are displayed
    await expect(page.getByText('120ms avg')).toBeVisible();
    await expect(page.getByText('85ms avg')).toBeVisible();
  });

  test('shows empty state when no data available', async ({ page }) => {
    await setupAnalyticsMocks(page, { hasData: false });
    await loginAndNavigateToAnalytics(page);

    // Check for empty state messages (matches UsageAnalytics component)
    await expect(page.getByText('No conversations yet')).toBeVisible();
    await expect(page.getByText('Users will see analytics here once they start chatting.')).toBeVisible();
  });

  test('shows loading spinner while data loads', async ({ page }) => {
    await setupDashboardMocks(page, { role: 'admin' });

    // Set up slow analytics response
    await page.route('**/api/dashboard/analytics*', async (route) => {
      await new Promise((resolve) => setTimeout(resolve, 1000));
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ time_series: [], top_tools: [], error_rate: 0, average_response_time: 0 }),
      });
    });

    await loginToDashboard(page);
    await navigateToTab(page, 'Analytics');

    // Should show loading spinner (use .first() since multiple spinners may render)
    await expect(page.locator('.pierre-spinner').first()).toBeVisible({ timeout: 5000 });
  });

  test('handles API error gracefully', async ({ page }) => {
    await setupDashboardMocks(page, { role: 'admin' });

    // Set up error response for analytics
    await page.route('**/api/dashboard/analytics*', async (route) => {
      await route.fulfill({
        status: 500,
        contentType: 'application/json',
        body: JSON.stringify({ error: 'Internal server error' }),
      });
    });

    await loginToDashboard(page);
    await navigateToTab(page, 'Analytics');

    // Page should still be navigable - check the header says Analytics
    await expect(page.locator('h1').first()).toContainText('Analytics');
  });
});

test.describe('Analytics Tab - Chart Interactions', () => {
  test('charts are responsive and render properly', async ({ page }) => {
    await setupAnalyticsMocks(page);
    await loginAndNavigateToAnalytics(page);

    // Wait for charts to render
    await page.waitForTimeout(1000);

    // Check multiple canvas elements are present (Line, Doughnut, Bar charts)
    const canvasElements = await page.locator('canvas').count();
    expect(canvasElements).toBeGreaterThanOrEqual(1);
  });

  test('tool list items are hoverable', async ({ page }) => {
    await setupAnalyticsMocks(page);
    await loginAndNavigateToAnalytics(page);

    // Find a tool list item and verify it's interactive
    const toolItem = page.locator('text=get_activities').locator('..').locator('..');
    await expect(toolItem).toBeVisible();
    // Hover over item to verify it's interactive
    await toolItem.hover();
  });
});
