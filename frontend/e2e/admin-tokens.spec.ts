// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Playwright E2E tests for the admin console's API Tokens screen.
// ABOUTME: Lists, filters, creates, revokes (one and many) and rotates tokens; every case opens the screen and asserts on it.

import { test, expect, type Page } from '@playwright/test';
import { applyTestStubs, navigateToTab, signInThroughHostedPage, TOKENS_TAB_LABEL } from './test-helpers';

// Helper to authenticate and set up common mocks
async function setupAuthenticatedSession(page: Page) {
  await applyTestStubs(page);

  // Mock setup status
  await page.route('**/admin/setup/status', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ needs_setup: false, admin_user_exists: true }),
    });
  });

  // Mock the hosted sign-in's code exchange
  await page.route('**/oauth/token', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        access_token: 'test-jwt-token',
        token_type: 'Bearer',
        expires_in: 86400,
        csrf_token: 'test-csrf-token',
        // The server's `UserInfo`, as a sign-in hands it over: `role` names the
        // admin, and `is_admin` beside it is derived from the role.
        user: {
          id: 'admin-1',
          user_id: 'admin-1',
          email: 'admin@test.com',
          display_name: 'Admin User',
          is_admin: true,
          role: 'admin',
          user_status: 'active',
          tenant_id: 'tenant-1',
          email_verified: true,
          created_at: '2024-01-01T10:00:00Z',
          locale: 'en',
          coaching_persona: 'casual',
          manages_roster: false,
          analytics_consent: false,
          analytics_consent_answered: true,
        },
      }),
    });
  });

  await page.route('**/api/dashboard/analytics**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ daily_usage: [] }),
    });
  });

  // Mock pending users (for badge)
  await page.route('**/api/admin/pending-users', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ success: true, message: 'Retrieved 0 pending users', data: { count: 0, users: [] } }),
    });
  });

  await page.route('**/api/admin/users**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ success: true, message: 'Retrieved users', data: { users: [], total: 0, has_more: false } }),
    });
  });
}

/**
 * Sign in and open the API Tokens screen. A case that reaches its body is on
 * that screen with the listing drawn: the sign-in fixture carried an admin,
 * the rail offered the screen and the listing was read.
 */
async function openTokenScreen(page: Page) {
  await page.goto('/');
  await signInThroughHostedPage(page);
  await navigateToTab(page, TOKENS_TAB_LABEL);
  await expect(page).toHaveURL(/#connections$/);
  await expect(page.getByRole('heading', { name: 'CI/CD Pipeline' })).toBeVisible();
}

const sampleTokens = [
  {
    id: 'token-1',
    service_name: 'CI/CD Pipeline',
    service_description: 'Automated deployment service',
    token_prefix: 'pierre_at_abc123',
    is_active: true,
    is_super_admin: false,
    permissions: ['provision_keys', 'list_keys'],
    created_at: '2024-01-01T10:00:00Z',
    // Midday, so the day it is drawn as does not depend on the runner's zone.
    expires_at: '2024-06-15T12:00:00Z',
    last_used_at: '2024-01-20T15:30:00Z',
    usage_count: 150,
  },
  {
    id: 'token-2',
    service_name: 'Admin Console',
    service_description: 'Full admin access',
    token_prefix: 'pierre_at_def456',
    is_active: true,
    is_super_admin: true,
    permissions: [],
    created_at: '2024-01-05T10:00:00Z',
    expires_at: null,
    last_used_at: '2024-01-21T09:00:00Z',
    usage_count: 500,
  },
  {
    id: 'token-3',
    service_name: 'Monitoring Service',
    service_description: 'Read-only monitoring',
    token_prefix: 'pierre_at_ghi789',
    is_active: false,
    is_super_admin: false,
    permissions: ['list_keys', 'view_audit_logs'],
    created_at: '2023-12-01T10:00:00Z',
    expires_at: '2024-01-01T00:00:00Z',
    last_used_at: '2023-12-31T23:00:00Z',
    usage_count: 75,
  },
];

/** `GET /api/admin/tokens` as the admin handler answers it. */
function tokenListing(tokens: typeof sampleTokens) {
  return {
    success: true,
    message: `Retrieved ${tokens.length} admin tokens`,
    data: { count: tokens.length, tokens },
  };
}

/** Answer the token listing; every other request to the token routes goes on. */
async function mockTokenListing(page: Page) {
  await page.route('**/api/admin/tokens**', async (route) => {
    if (route.request().method() === 'GET') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify(tokenListing(sampleTokens)),
      });
    } else {
      await route.continue();
    }
  });
}

/** The ids of the tokens the page asked `action` for, in request order. */
function recordTokenActions(page: Page, action: 'revoke' | 'rotate'): string[] {
  const ids: string[] = [];
  page.on('request', (request) => {
    const match = new RegExp(`/api/admin/tokens/([^/]+)/${action}$`).exec(new URL(request.url()).pathname);
    if (match && request.method() === 'POST') ids.push(match[1]);
  });
  return ids;
}

test.describe('API Key List', () => {
  test.beforeEach(async ({ page }) => {
    await setupAuthenticatedSession(page);
    await mockTokenListing(page);
    await openTokenScreen(page);
  });

  test('displays API keys list', async ({ page }) => {
    await expect(page.getByRole('heading', { name: 'Your API Tokens' })).toBeVisible();
    await expect(page.getByText('3 total tokens')).toBeVisible();
    // The listing opens on the active tokens.
    await expect(page.getByRole('heading', { name: 'CI/CD Pipeline' })).toBeVisible();
    await expect(page.getByRole('heading', { name: 'Admin Console' })).toBeVisible();
    await expect(page.getByRole('heading', { name: 'Monitoring Service' })).toHaveCount(0);
  });

  test('shows active and inactive token counts', async ({ page }) => {
    await expect(page.getByRole('button', { name: 'Active(2)', exact: true })).toBeVisible();
    await expect(page.getByRole('button', { name: 'All(3)', exact: true })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Inactive(1)', exact: true })).toBeVisible();
  });

  test('can filter tokens by status', async ({ page }) => {
    await page.getByRole('button', { name: 'Inactive(1)', exact: true }).click();
    await expect(page.getByRole('heading', { name: 'Monitoring Service' })).toBeVisible();
    await expect(page.getByRole('heading', { name: 'CI/CD Pipeline' })).toHaveCount(0);

    await page.getByRole('button', { name: 'All(3)', exact: true }).click();
    await expect(page.getByRole('heading', { name: 'Monitoring Service' })).toBeVisible();
    await expect(page.getByRole('heading', { name: 'CI/CD Pipeline' })).toBeVisible();
  });

  test('displays token prefix with lock icon', async ({ page }) => {
    const prefix = page.locator('code', { hasText: 'pierre_at_abc123...' });
    await expect(prefix).toBeVisible();
    await expect(prefix.locator('svg')).toBeVisible();
  });

  test('shows Super Admin badge for super admin tokens', async ({ page }) => {
    // One of the two active tokens carries the flag.
    await expect(page.getByText('Super Admin', { exact: true })).toHaveCount(1);
  });

  test('shows expiration date or Never for tokens', async ({ page }) => {
    await expect(page.getByText('Jun 15, 2024', { exact: true })).toBeVisible();
    // The token that never expires; both active tokens have been used.
    await expect(page.getByText('Never', { exact: true })).toHaveCount(1);
  });
});

test.describe('API Key Creation', () => {
  /** The bodies of the create requests the page sent. */
  let created: unknown[];

  test.beforeEach(async ({ page }) => {
    created = [];
    await setupAuthenticatedSession(page);

    await page.route('**/api/admin/tokens**', async (route) => {
      if (route.request().method() === 'GET') {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify(tokenListing(sampleTokens)),
        });
      } else if (route.request().method() === 'POST') {
        created.push(route.request().postDataJSON());
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({
            success: true,
            message: 'Admin token created successfully',
            data: {
              token_id: 'token-new',
              service_name: 'New Service',
              jwt_token: 'pierre_at_newtoken_full_jwt_here',
              token_prefix: 'pierre_at_new123',
              is_super_admin: false,
              expires_at: null,
            },
          }),
        });
      } else {
        await route.continue();
      }
    });

    await openTokenScreen(page);
    await page.getByRole('button', { name: 'Create API Token' }).click();
    await expect(page.getByRole('heading', { name: 'Create API Token' })).toBeVisible();
  });

  /** The form's submit button, which carries the same words as its heading. */
  const submitButton = (page: Page) => page.locator('button[type="submit"]', { hasText: 'Create API Token' });
  const successModal = (page: Page) => page.getByRole('dialog', { name: 'API Token Generated Successfully' });

  test('can open create token form', async ({ page }) => {
    await expect(page.getByLabel(/Service Name/)).toBeVisible();
    await expect(page.getByLabel('Provision API Keys')).toBeVisible();
  });

  test('validates required service name', async ({ page }) => {
    // A permission alone submits nothing: the token needs a name.
    await page.getByLabel('Provision API Keys').check();
    await expect(submitButton(page)).toBeDisabled();

    await page.getByLabel(/Service Name/).fill('Test Integration Service');
    await expect(submitButton(page)).toBeEnabled();
  });

  test('can create token with permissions', async ({ page }) => {
    await page.getByLabel(/Service Name/).fill('Test Integration Service');
    await page.getByLabel('Provision API Keys').check();
    await submitButton(page).click();

    await expect(successModal(page)).toBeVisible();
    expect(created).toHaveLength(1);
    expect(created[0]).toMatchObject({
      service_name: 'Test Integration Service',
      permissions: ['provision_keys'],
      is_super_admin: false,
    });
  });

  test('shows warning for super admin creation', async ({ page }) => {
    await expect(page.getByRole('heading', { name: 'Danger Zone' })).toHaveCount(0);

    await page.getByLabel('Super Admin API Token').check();

    await expect(page.getByRole('heading', { name: 'Danger Zone' })).toBeVisible();
    // A super-admin token holds every permission, so none is offered.
    await expect(page.getByLabel('Provision API Keys')).toHaveCount(0);
  });

  test('displays token in success modal', async ({ page }) => {
    await page.getByLabel(/Service Name/).fill('New API Service');
    await page.getByLabel('Provision API Keys').check();
    await submitButton(page).click();

    await expect(successModal(page).getByLabel('JWT Token')).toHaveValue('pierre_at_newtoken_full_jwt_here');
    await expect(successModal(page).getByText('pierre_at_new123...')).toBeVisible();

    await successModal(page).getByRole('button', { name: /Saved the Token Securely/ }).click();
    await expect(successModal(page)).toHaveCount(0);
    await expect(page.getByRole('heading', { name: 'Your API Tokens' })).toBeVisible();
  });

  test('can copy token from success modal', async ({ page, context }) => {
    await context.grantPermissions(['clipboard-read', 'clipboard-write']);
    await page.getByLabel(/Service Name/).fill('Copy Test Service');
    await page.getByLabel('List API Keys').check();
    await submitButton(page).click();

    await successModal(page).getByRole('button', { name: 'Copy', exact: true }).click();

    await expect(successModal(page).getByRole('button', { name: 'Copied', exact: true })).toBeVisible();
    expect(await page.evaluate(() => navigator.clipboard.readText())).toBe('pierre_at_newtoken_full_jwt_here');
  });
});

test.describe('API Key Revocation', () => {
  let revoked: string[];

  test.beforeEach(async ({ page }) => {
    await setupAuthenticatedSession(page);
    await mockTokenListing(page);

    // Mock revoke endpoint
    await page.route('**/api/admin/tokens/*/revoke', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          success: true,
          message: 'Admin token revoked successfully',
          data: { token_id: 'token-1' },
        }),
      });
    });
    revoked = recordTokenActions(page, 'revoke');

    await openTokenScreen(page);
  });

  test('can revoke a single token', async ({ page }) => {
    await page.getByRole('button', { name: 'Revoke', exact: true }).first().click();
    await page.getByRole('button', { name: 'Revoke API Token', exact: true }).click();

    await expect(page.getByRole('heading', { name: 'Revoke API Token' })).toHaveCount(0);
    expect(revoked).toEqual(['token-1']);
  });

  test('shows revocation confirmation dialog', async ({ page }) => {
    await page.getByRole('button', { name: 'Revoke', exact: true }).first().click();

    await expect(page.getByRole('heading', { name: 'Revoke API Token' })).toBeVisible();
    await expect(page.getByText(/Are you sure you want to revoke "CI\/CD Pipeline"\?/)).toBeVisible();

    // Cancelling asks the server for nothing.
    await page.getByRole('button', { name: 'Cancel', exact: true }).click();
    await expect(page.getByRole('heading', { name: 'Revoke API Token' })).toHaveCount(0);
    expect(revoked).toEqual([]);
  });

  test('can select multiple tokens for bulk revocation', async ({ page }) => {
    await expect(page.getByRole('button', { name: 'Revoke Selected' })).toHaveCount(0);

    await page.getByLabel('Select CI/CD Pipeline').check();
    await page.getByLabel('Select Admin Console').check();

    await expect(page.getByText('2 selected')).toBeVisible();
    await page.getByRole('button', { name: 'Revoke Selected' }).click();
    await expect(page.getByRole('heading', { name: 'Revoke Multiple API Tokens' })).toBeVisible();
    await page.getByRole('button', { name: 'Revoke 2 API Token(s)', exact: true }).click();

    await expect(page.getByRole('heading', { name: 'Revoke Multiple API Tokens' })).toHaveCount(0);
    expect([...revoked].sort()).toEqual(['token-1', 'token-2']);
  });

  test('can use select all checkbox', async ({ page }) => {
    await page.getByLabel('Select All (2)').check();

    await expect(page.getByLabel('Select CI/CD Pipeline')).toBeChecked();
    await expect(page.getByLabel('Select Admin Console')).toBeChecked();
    await expect(page.getByText('2 selected')).toBeVisible();

    await page.getByLabel('Select All (2)').uncheck();
    await expect(page.getByLabel('Select CI/CD Pipeline')).not.toBeChecked();
    await expect(page.getByRole('button', { name: 'Revoke Selected' })).toHaveCount(0);
  });
});

test.describe('API Key Rotation', () => {
  let rotated: string[];

  test.beforeEach(async ({ page }) => {
    await setupAuthenticatedSession(page);
    await mockTokenListing(page);

    // Mock rotate endpoint
    await page.route('**/api/admin/tokens/*/rotate', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          success: true,
          message: 'Admin token rotated successfully',
          data: {
            old_token_id: 'token-1',
            token_id: 'token-1-rotated',
            service_name: 'CI/CD Pipeline',
            jwt_token: 'pierre_at_rotated_new_jwt_token',
            token_prefix: 'pierre_at_rot123',
            is_super_admin: false,
            expires_at: null,
          },
        }),
      });
    });
    rotated = recordTokenActions(page, 'rotate');

    await openTokenScreen(page);
    await page.getByRole('button', { name: 'View Details' }).first().click();
    await expect(page.getByRole('button', { name: 'Rotate Key' })).toBeVisible();
  });

  const rotationReveal = (page: Page) => page.getByRole('dialog', { name: 'API Token Rotated Successfully' });

  test('can rotate a token', async ({ page }) => {
    // The details screen asks through the browser's own confirm.
    const asked: string[] = [];
    page.on('dialog', async (dialog) => {
      asked.push(dialog.message());
      await dialog.accept();
    });

    await page.getByRole('button', { name: 'Rotate Key' }).click();

    await expect(rotationReveal(page)).toBeVisible();
    expect(asked).toHaveLength(1);
    expect(asked[0]).toContain('rotate the API token for "CI/CD Pipeline"');
    expect(rotated).toEqual(['token-1']);
  });

  test('a declined rotation asks the server for nothing', async ({ page }) => {
    page.on('dialog', (dialog) => dialog.dismiss());

    await page.getByRole('button', { name: 'Rotate Key' }).click();

    await expect(page.getByRole('button', { name: 'Rotate Key' })).toBeEnabled();
    await expect(rotationReveal(page)).toHaveCount(0);
    expect(rotated).toEqual([]);
  });

  test('displays new token after rotation', async ({ page }) => {
    page.on('dialog', (dialog) => dialog.accept());

    await page.getByRole('button', { name: 'Rotate Key' }).click();

    await expect(rotationReveal(page).getByLabel('New API Key')).toHaveValue('pierre_at_rotated_new_jwt_token');
    await expect(rotationReveal(page).getByText('pierre_at_rot123...')).toBeVisible();
  });
});

test.describe('API Key Details', () => {
  test.beforeEach(async ({ page }) => {
    await setupAuthenticatedSession(page);
    await mockTokenListing(page);
    await openTokenScreen(page);
  });

  test('can view token details', async ({ page }) => {
    await page.getByRole('button', { name: 'View Details' }).first().click();

    await expect(page.getByText('API Token • pierre_at_abc123...')).toBeVisible();
    await expect(page.getByText('API Token Information')).toBeVisible();
    await expect(page.getByText('Automated deployment service')).toBeVisible();
    await expect(page.getByRole('button', { name: 'Revoke Key' })).toBeVisible();

    await page.getByRole('button', { name: 'Back' }).click();
    await expect(page.getByRole('heading', { name: 'Your API Tokens' })).toBeVisible();
  });
});

/**
 * The listing with a token a rotation retired beside the one that replaced
 * it: an inactive row naming its replacement (carnet#1589).
 */
const withRotated = [
  ...sampleTokens,
  {
    id: 'token-4',
    service_name: 'Nightly Export',
    service_description: 'Rotated into token-1',
    token_prefix: 'pierre_at_jkl012',
    is_active: false,
    is_super_admin: false,
    permissions: ['list_keys'],
    created_at: '2023-11-01T10:00:00Z',
    expires_at: null,
    last_used_at: '2023-12-20T08:00:00Z',
    usage_count: 42,
    rotated_to_token_id: 'token-1',
  },
];

test.describe('A rotated token', () => {
  let revoked: string[];

  test.beforeEach(async ({ page }) => {
    await setupAuthenticatedSession(page);
    await page.route('**/api/admin/tokens**', async (route) => {
      if (route.request().method() === 'GET') {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({
            success: true,
            message: `Retrieved ${withRotated.length} admin tokens`,
            data: { count: withRotated.length, tokens: withRotated },
          }),
        });
      } else {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({ success: true, message: 'Admin token revoked successfully', data: {} }),
        });
      }
    });
    revoked = recordTokenActions(page, 'revoke');
    await openTokenScreen(page);
    await page.getByRole('button', { name: 'Inactive(2)', exact: true }).click();
  });

  test('reads as rotated, and a revoked one as inactive', async ({ page }) => {
    const rotated = page.locator('div', { has: page.getByRole('heading', { name: 'Nightly Export' }) }).last();
    await expect(rotated.getByText('Rotated', { exact: true })).toBeVisible();
    const revokedRow = page.locator('div', { has: page.getByRole('heading', { name: 'Monitoring Service' }) }).last();
    await expect(revokedRow.getByText('Inactive', { exact: true })).toBeVisible();
    await expect(revokedRow.getByText('Rotated', { exact: true })).toHaveCount(0);
  });

  test('is left out of a bulk revoke', async ({ page }) => {
    await expect(page.getByLabel('Select Nightly Export')).toBeDisabled();
    await page.getByLabel('Select All (1)').check();
    await expect(page.getByText('1 selected')).toBeVisible();
    await expect(page.getByLabel('Select Nightly Export')).not.toBeChecked();

    await page.getByRole('button', { name: 'Revoke Selected' }).click();
    await page.getByRole('button', { name: 'Revoke 1 API Token(s)', exact: true }).click();
    await expect(page.getByRole('heading', { name: 'Revoke Multiple API Tokens' })).toHaveCount(0);
    expect(revoked).toEqual(['token-3']);
  });
});
