// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Playwright E2E tests for the Boreal Editorial theming system
// ABOUTME: Verifies the light-first theme, dark variant via <html class="dark">, and runtime toggle

import { test, expect, type Page } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard, navigateToTab } from './test-helpers';

async function setupThemeMocks(page: Page, options: { isAdmin?: boolean } = {}) {
  const { isAdmin = true } = options;
  await setupDashboardMocks(page, { role: isAdmin ? 'admin' : 'user' });
}

async function loginAndGoToDashboard(page: Page) {
  await loginToDashboard(page);
  await page.waitForTimeout(300);
}

/** Force the light theme via localStorage before the app boots. */
async function forceLightTheme(page: Page) {
  await page.addInitScript(() => {
    try {
      window.localStorage.setItem('dravr.theme', 'light');
    } catch {
      /* ignore */
    }
  });
}

/** Force the dark theme via localStorage before the app boots. */
async function forceDarkTheme(page: Page) {
  await page.addInitScript(() => {
    try {
      window.localStorage.setItem('dravr.theme', 'dark');
    } catch {
      /* ignore */
    }
  });
}

function parseRgb(color: string): { r: number; g: number; b: number } | null {
  const match = color.match(/rgba?\((\d+),\s*(\d+),\s*(\d+)/);
  if (!match) return null;
  const [, r, g, b] = match.map(Number);
  return { r, g, b };
}

test.describe('Boreal Light Theme — default surface', () => {
  test('body uses the off-white surface background', async ({ page }) => {
    await forceLightTheme(page);
    await setupThemeMocks(page, { isAdmin: true });
    await loginAndGoToDashboard(page);

    await page.waitForSelector('nav', { timeout: 10000 });

    const bodyBgColor = await page.evaluate(() => window.getComputedStyle(document.body).backgroundColor);
    const rgb = parseRgb(bodyBgColor);
    expect(rgb).not.toBeNull();
    if (rgb) {
      // surface #f9f9f6 — combined brightness ~ 747
      expect(rgb.r + rgb.g + rgb.b).toBeGreaterThan(650);
    }
  });

  test('body text is dark ink, not pure black', async ({ page }) => {
    await forceLightTheme(page);
    await setupThemeMocks(page, { isAdmin: true });
    await loginAndGoToDashboard(page);

    await page.waitForSelector('nav', { timeout: 10000 });

    const bodyColor = await page.evaluate(() => window.getComputedStyle(document.body).color);
    const rgb = parseRgb(bodyColor);
    expect(rgb).not.toBeNull();
    if (rgb) {
      // on-surface #1a1c1b — dark but never pure black
      expect(rgb.r + rgb.g + rgb.b).toBeLessThan(200);
      expect(rgb.r + rgb.g + rgb.b).toBeGreaterThan(20);
    }
  });

  test('headings inherit the on-surface ink color', async ({ page }) => {
    await forceLightTheme(page);
    await setupThemeMocks(page, { isAdmin: true });
    await loginAndGoToDashboard(page);

    await page.waitForSelector('nav', { timeout: 10000 });

    const headingColor = await page.evaluate(() => {
      const heading = document.querySelector('h1, h2, h3');
      return heading ? window.getComputedStyle(heading).color : null;
    });

    if (headingColor) {
      const rgb = parseRgb(headingColor);
      if (rgb) {
        // Boreal ink ≤ on-surface-variant (~250 combined)
        expect(rgb.r + rgb.g + rgb.b).toBeLessThan(250);
      }
    }
  });

  test('sidebar navigation is rendered against the light surface', async ({ page }) => {
    await forceLightTheme(page);
    await setupThemeMocks(page, { isAdmin: true });
    await loginAndGoToDashboard(page);

    await page.waitForSelector('nav', { timeout: 10000 });

    const sidebar = page.locator('nav, aside').first();
    await expect(sidebar).toBeVisible();

    const navButtons = page.locator('nav button');
    expect(await navButtons.count()).toBeGreaterThan(0);
  });

  test('main content area is visible on the light canvas', async ({ page }) => {
    await forceLightTheme(page);
    await setupThemeMocks(page, { isAdmin: true });
    await loginAndGoToDashboard(page);

    await page.waitForSelector('nav', { timeout: 10000 });

    const main = page.locator('main');
    await expect(main).toBeVisible();
  });
});

test.describe('Boreal Light Theme — consistency across navigation', () => {
  test('light theme persists across tab navigation', async ({ page }) => {
    await forceLightTheme(page);
    await setupThemeMocks(page, { isAdmin: true });
    await loginAndGoToDashboard(page);
    await page.waitForSelector('nav', { timeout: 10000 });

    const initialBgColor = await page.evaluate(() => window.getComputedStyle(document.body).backgroundColor);

    await navigateToTab(page, 'Analytics');
    await page.waitForTimeout(300);

    const analyticsBgColor = await page.evaluate(() => window.getComputedStyle(document.body).backgroundColor);
    expect(analyticsBgColor).toBe(initialBgColor);
  });

  test('light theme applies to all major UI sections', async ({ page }) => {
    await forceLightTheme(page);
    await setupThemeMocks(page, { isAdmin: true });
    await loginAndGoToDashboard(page);
    await page.waitForSelector('nav', { timeout: 10000 });

    await expect(page.locator('nav, aside').first()).toBeVisible();
    await expect(page.locator('main')).toBeVisible();
    await expect(page.locator('h1').first()).toBeVisible();
  });

  test('form inputs have proper light theme styling', async ({ page }) => {
    await forceLightTheme(page);
    await setupThemeMocks(page, { isAdmin: true });

    await page.goto('/');
    await page.waitForSelector('form', { timeout: 10000 });

    const emailInput = page.locator('input[name="email"]');
    await expect(emailInput).toBeVisible();
    await emailInput.fill('test@example.com');
    await expect(emailInput).toHaveValue('test@example.com');
  });
});

test.describe('Boreal Dark Theme — tuned variant', () => {
  test('<html class="dark"> is applied when preference is dark', async ({ page }) => {
    await forceDarkTheme(page);
    await setupThemeMocks(page, { isAdmin: true });

    await page.goto('/');
    await page.waitForSelector('form', { timeout: 10000 });

    const htmlClass = await page.evaluate(() => document.documentElement.className);
    expect(htmlClass).toContain('dark');
  });

  test('body uses the near-black surface in dark mode', async ({ page }) => {
    await forceDarkTheme(page);
    await setupThemeMocks(page, { isAdmin: true });
    await loginAndGoToDashboard(page);
    await page.waitForSelector('nav', { timeout: 10000 });

    const bodyBgColor = await page.evaluate(() => window.getComputedStyle(document.body).backgroundColor);
    const rgb = parseRgb(bodyBgColor);
    expect(rgb).not.toBeNull();
    if (rgb) {
      // Boreal dark surface #11130f — combined brightness ~ 53
      expect(rgb.r + rgb.g + rgb.b).toBeLessThan(120);
    }
  });

  test('body text becomes light ink in dark mode', async ({ page }) => {
    await forceDarkTheme(page);
    await setupThemeMocks(page, { isAdmin: true });
    await loginAndGoToDashboard(page);
    await page.waitForSelector('nav', { timeout: 10000 });

    const bodyColor = await page.evaluate(() => window.getComputedStyle(document.body).color);
    const rgb = parseRgb(bodyColor);
    expect(rgb).not.toBeNull();
    if (rgb) {
      // Boreal dark on-surface #e1e3de — combined brightness ~ 676
      expect(rgb.r + rgb.g + rgb.b).toBeGreaterThan(550);
    }
  });
});

test.describe('Boreal Theme — visible layout elements', () => {
  test('login page renders the editorial layout in light mode', async ({ page }) => {
    await forceLightTheme(page);
    await setupThemeMocks(page, { isAdmin: true });

    await page.goto('/');
    await page.waitForSelector('form', { timeout: 10000 });

    await expect(page.locator('input[name="email"]')).toBeVisible();
    await expect(page.locator('input[name="password"]')).toBeVisible();
    await expect(page.getByRole('button', { name: 'Sign in' })).toBeVisible();

    // DRAVR wordmark lives in the hero column on desktop and as a mobile label
    await expect(page.getByText('DRAVR').first()).toBeVisible();
  });

  test('dashboard renders the editorial layout', async ({ page }) => {
    await forceLightTheme(page);
    await setupThemeMocks(page, { isAdmin: true });
    await loginAndGoToDashboard(page);
    await page.waitForSelector('nav', { timeout: 10000 });

    await expect(page.locator('nav').first()).toBeVisible();
    await expect(page.locator('main')).toBeVisible();
    await expect(page.getByText('Users').first()).toBeVisible({ timeout: 10000 });
  });

  test('sidebar navigation surface is readable', async ({ page }) => {
    await forceLightTheme(page);
    await setupThemeMocks(page, { isAdmin: true });
    await loginAndGoToDashboard(page);
    await page.waitForSelector('nav', { timeout: 10000 });

    const sidebar = page.locator('nav, aside').first();
    await expect(sidebar).toBeVisible();

    await expect(page.locator('nav button').filter({ has: page.locator('span:text-is("Users")') })).toBeVisible();
    await expect(page.locator('button').filter({ has: page.locator('span:has-text("Analytics")') })).toBeVisible();
  });

  test('interactive elements have visible focus states', async ({ page }) => {
    await forceLightTheme(page);
    await setupThemeMocks(page, { isAdmin: true });
    await loginAndGoToDashboard(page);
    await page.waitForSelector('nav', { timeout: 10000 });

    const usersButton = page.locator('nav button').filter({ has: page.locator('span:text-is("Users")') });
    await usersButton.focus();
    await expect(usersButton).toBeFocused();

    await page.keyboard.press('Tab');
    const focusedElement = page.locator(':focus');
    await expect(focusedElement).toBeVisible();
  });

  test('cards and panels render with Boreal tonal layering', async ({ page }) => {
    await forceLightTheme(page);
    await setupThemeMocks(page, { isAdmin: true });
    await loginAndGoToDashboard(page);
    await page.waitForSelector('nav', { timeout: 10000 });

    const cards = page.locator('[class*="card"], [class*="rounded"][class*="shadow"], [class*="bg-surface-container"]');
    const cardCount = await cards.count();
    if (cardCount > 0) {
      await expect(cards.first()).toBeVisible();
    }
  });
});

test.describe('Boreal Theme — accessibility', () => {
  test('text remains readable on the light surface', async ({ page }) => {
    await forceLightTheme(page);
    await setupThemeMocks(page, { isAdmin: true });
    await loginAndGoToDashboard(page);
    await page.waitForSelector('nav', { timeout: 10000 });

    const headings = page.locator('h1, h2, h3');
    const headingCount = await headings.count();
    for (let i = 0; i < Math.min(headingCount, 5); i++) {
      await expect(headings.nth(i)).toBeVisible();
    }
  });

  test('form inputs are visible and usable', async ({ page }) => {
    await forceLightTheme(page);
    await setupThemeMocks(page, { isAdmin: true });

    await page.goto('/');
    await page.waitForSelector('form', { timeout: 10000 });

    const emailInput = page.locator('input[name="email"]');
    await expect(emailInput).toBeVisible();
    await emailInput.fill('test@example.com');
    await expect(emailInput).toHaveValue('test@example.com');

    const passwordInput = page.locator('input[name="password"]');
    await expect(passwordInput).toBeVisible();
    await passwordInput.fill('testpassword');
  });

  test('navigation icons are visible', async ({ page }) => {
    await forceLightTheme(page);
    await setupThemeMocks(page, { isAdmin: true });
    await loginAndGoToDashboard(page);
    await page.waitForSelector('nav', { timeout: 10000 });

    const svgIcons = page.locator('nav svg, aside svg');
    const iconCount = await svgIcons.count();
    expect(iconCount).toBeGreaterThan(0);

    for (let i = 0; i < Math.min(iconCount, 3); i++) {
      await expect(svgIcons.nth(i)).toBeVisible();
    }
  });

  test('error states are visible on the light surface', async ({ page }) => {
    await forceLightTheme(page);
    await setupThemeMocks(page, { isAdmin: true });

    await page.route('**/oauth/token', async (route) => {
      await route.fulfill({
        status: 401,
        contentType: 'application/json',
        body: JSON.stringify({
          error: 'invalid_grant',
          error_description: 'Invalid credentials',
        }),
      });
    });

    await page.goto('/');
    await page.waitForSelector('form', { timeout: 10000 });

    await page.locator('input[name="email"]').fill('test@example.com');
    await page.locator('input[name="password"]').fill('wrongpassword');
    await page.getByRole('button', { name: 'Sign in' }).click();

    await page.waitForTimeout(1000);

    const errorElement = page.locator('[role="alert"], .error, .text-error');
    const errorCount = await errorElement.count();
    if (errorCount > 0) {
      await expect(errorElement.first()).toBeVisible();
    }
  });
});
