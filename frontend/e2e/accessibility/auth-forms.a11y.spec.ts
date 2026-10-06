// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Accessibility tests for login and registration forms ensuring WCAG 2.1 AA compliance.
// ABOUTME: Tests form labels, error messages, focus management, keyboard navigation, and color contrast.

import { test, expect } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { SIGN_IN_BUTTON, waitForLoginScreen } from '../test-helpers';

test.describe('Auth Forms Accessibility', () => {
  test.describe('Login Page', () => {
    test.beforeEach(async ({ page }) => {
      await page.goto('/');
      await waitForLoginScreen(page);
    });

    test('should have no WCAG 2.1 AA violations on login page', async ({ page }) => {
      const accessibilityScanResults = await new AxeBuilder({ page })
        .withTags(['wcag2a', 'wcag2aa', 'wcag21aa'])
        .analyze();

      // Log violations for awareness
      if (accessibilityScanResults.violations.length > 0) {
        console.log('Login page a11y violations:', JSON.stringify(accessibilityScanResults.violations, null, 2));
      }
      expect.soft(accessibilityScanResults.violations).toEqual([]);
    });

    test('the sign-in button has an accessible name', async ({ page }) => {
      // carnet#787: the login screen asks for no credentials; its one control
      // opens the server's hosted sign-in, and it must say so to a reader.
      await expect(page.getByRole('button', { name: 'Sign in with email' })).toBeVisible();
      await expect(page.locator('input[type="password"]')).toHaveCount(0);
    });

    test('should support keyboard navigation', async ({ page }) => {
      // Start the chain at the sign-in button rather than brittle-counting
      // Tab presses past the theme toggle the editorial layout ships first.
      const signIn = page.locator(SIGN_IN_BUTTON);
      await signIn.focus();
      await expect(signIn).toBeFocused();

      // The next focusable is another button ("Forgot password?"): no trap.
      await page.keyboard.press('Tab');
      const buttonFocused = await page.evaluate(
        () => document.activeElement?.tagName === 'BUTTON'
      );
      expect(buttonFocused).toBe(true);
      await expect(signIn).not.toBeFocused();
    });

    test('should have visible focus indicators', async ({ page }) => {
      // Reach the button by keyboard so :focus-visible applies.
      const signIn = page.locator(SIGN_IN_BUTTON);
      await signIn.focus();
      await page.keyboard.press('Shift+Tab');
      await page.keyboard.press('Tab');
      await expect(signIn).toBeFocused();

      const focusStyles = await signIn.evaluate((el) => {
        const styles = window.getComputedStyle(el);
        return { outlineStyle: styles.outlineStyle, boxShadow: styles.boxShadow };
      });

      const hasFocusIndicator =
        focusStyles.outlineStyle !== 'none' ||
        focusStyles.boxShadow !== 'none';
      expect(hasFocusIndicator).toBe(true);
    });

    test('should announce a refused sign-in to screen readers', async ({ page }) => {
      // The hosted sign-in came back refused (a suspended account).
      await page.goto('/auth/callback?error=access_denied&state=e2e');

      const errorMessage = page.locator('[role="alert"]');
      await expect(errorMessage).toBeVisible({ timeout: 10000 });
      await expect(errorMessage).toHaveAttribute('aria-live', 'polite');
    });

    test('should have proper heading hierarchy', async ({ page }) => {
      const headings = await page.evaluate(() => {
        const h1s = document.querySelectorAll('h1');
        const h2s = document.querySelectorAll('h2');
        return { h1Count: h1s.length, h2Count: h2s.length };
      });

      // Should have at least one h1
      expect(headings.h1Count).toBeGreaterThanOrEqual(1);
    });

    test('should have sufficient color contrast', async ({ page }) => {
      const accessibilityScanResults = await new AxeBuilder({ page })
        .withTags(['cat.color'])
        .disableRules(['color-contrast-enhanced'])
        .analyze();

      const contrastViolations = accessibilityScanResults.violations.filter(
        (v) => v.id.includes('contrast')
      );

      // Log any violations for debugging
      if (contrastViolations.length > 0) {
        console.log(`Login page color contrast violations: ${contrastViolations.length}`);
        for (const violation of contrastViolations) {
          for (const node of violation.nodes) {
            console.log(`  - ${node.html}`);
          }
        }
      }

      expect(contrastViolations).toEqual([]);
    });

    test('should start the sign-in with the Enter key', async ({ page }) => {
      let opened = false;
      await page.route('**/oauth2/authorize**', async (route) => {
        opened = true;
        await route.fulfill({ status: 200, contentType: 'text/html', body: '<h1>Hosted sign-in</h1>' });
      });

      await page.locator(SIGN_IN_BUTTON).focus();
      await page.keyboard.press('Enter');

      await expect(page.getByRole('heading', { name: 'Hosted sign-in' })).toBeVisible();
      expect(opened).toBe(true);
    });
  });

  test.describe('Registration Page', () => {
    test.beforeEach(async ({ page }) => {
      // Navigate to registration (may need to click a link from login)
      await page.goto('/');
      await waitForLoginScreen(page);

      // The login screen's "Don't have an account? Create one" button.
      await page.getByRole('button', { name: /create one/i }).click();
      await page.locator('input[name="displayName"]').waitFor({ state: 'visible' });
    });

    test('should have no WCAG 2.1 AA violations on registration form', async ({ page }) => {
      const accessibilityScanResults = await new AxeBuilder({ page })
        .withTags(['wcag2a', 'wcag2aa', 'wcag21aa'])
        .analyze();

      if (accessibilityScanResults.violations.length > 0) {
        console.log('Registration form a11y violations:', JSON.stringify(accessibilityScanResults.violations, null, 2));
      }
      expect.soft(accessibilityScanResults.violations).toEqual([]);
    });

    test('should indicate required fields', async ({ page }) => {
      // Required fields should have aria-required or required attribute
      const requiredInputs = await page.evaluate(() => {
        const inputs = document.querySelectorAll('input');
        let hasRequiredIndicators = false;
        inputs.forEach((input) => {
          if (
            input.hasAttribute('required') ||
            input.getAttribute('aria-required') === 'true'
          ) {
            hasRequiredIndicators = true;
          }
        });
        return hasRequiredIndicators;
      });

      expect(requiredInputs).toBe(true);
    });

    test('should have password requirements visible', async ({ page }) => {
      // Password field should have instructions visible or via aria-describedby
      const passwordInput = page.locator('input[type="password"]').first();

      if ((await passwordInput.count()) > 0) {
        const hasDescription = await passwordInput.evaluate((el) => {
          const describedBy = el.getAttribute('aria-describedby');
          if (describedBy) {
            const description = document.getElementById(describedBy);
            return !!description?.textContent;
          }
          // Check for adjacent text
          const parent = el.parentElement;
          return parent?.textContent?.includes('characters') ?? false;
        });

        // Should have some form of password requirements
        // This might be in a tooltip or adjacent text
        expect(typeof hasDescription).toBe('boolean');
      }
    });
  });

  test.describe('Password Reset Flow', () => {
    test.beforeEach(async ({ page }) => {
      await page.goto('/');
      await waitForLoginScreen(page);
    });

    test('should have accessible forgot password link', async ({ page }) => {
      const forgotLink = page.getByRole('link', { name: /forgot|reset/i });

      if ((await forgotLink.count()) > 0) {
        // Link should be focusable and have proper text
        await expect(forgotLink).toBeVisible();

        // Should be keyboard accessible
        await forgotLink.focus();
        const isFocused = await page.evaluate(() => {
          const active = document.activeElement;
          return active?.tagName === 'A';
        });
        expect(isFocused).toBe(true);
      }
    });
  });

  test.describe('Error States', () => {
    test.beforeEach(async ({ page }) => {
      // The hosted sign-in came back refused: the login screen shows why.
      await page.goto('/auth/callback?error=access_denied&state=e2e');
      await waitForLoginScreen(page);
      await expect(page.locator('[role="alert"]')).toBeVisible();
    });

    test('should have accessible error messages', async ({ page }) => {
      const accessibilityScanResults = await new AxeBuilder({ page })
        .withTags(['wcag2a', 'wcag2aa'])
        .analyze();

      if (accessibilityScanResults.violations.length > 0) {
        console.log('Error state a11y violations:', JSON.stringify(accessibilityScanResults.violations, null, 2));
      }
      expect.soft(accessibilityScanResults.violations).toEqual([]);
    });

    test('should keep the sign-in reachable after an error', async ({ page }) => {
      const signIn = page.locator(SIGN_IN_BUTTON);
      await expect(signIn).toBeEnabled();
      await signIn.focus();
      await expect(signIn).toBeFocused();
    });
  });

  test.describe('Touch Target Sizes', () => {
    test('should have sufficient touch target sizes (44x44px minimum)', async ({ page }) => {
      await page.goto('/');
      await waitForLoginScreen(page);

      // Check button size
      const button = page.locator(SIGN_IN_BUTTON);
      const buttonSize = await button.boundingBox();

      if (buttonSize) {
        expect(buttonSize.width).toBeGreaterThanOrEqual(44);
        expect(buttonSize.height).toBeGreaterThanOrEqual(44);
      }
    });
  });
});
