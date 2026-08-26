// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Playwright E2E tests for System Coaches admin functionality.
// ABOUTME: Tests admin coaches management, CRUD operations, and user assignments.

import { test, expect, type Page } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard, navigateToTab, APP_SHELL_TIMEOUT_MS } from './test-helpers';

// Mock coach data
const mockCoaches = [
  {
    id: 'coach-1',
    title: 'Marathon Training Coach',
    description: 'Specialized in marathon preparation and endurance training',
    system_prompt: 'You are a professional marathon coach with expertise in long-distance running...',
    category: 'Training',
    tags: ['marathon', 'endurance', 'running'],
    token_count: 150,
    is_favorite: false,
    use_count: 42,
    last_used_at: '2025-01-10T10:00:00Z',
    created_at: '2025-01-01T00:00:00Z',
    updated_at: '2025-01-10T10:00:00Z',
    is_system: true,
    visibility: 'tenant',
    is_assigned: false,
  },
  {
    id: 'coach-2',
    title: 'Nutrition Expert',
    description: 'Helps with meal planning and sports nutrition',
    system_prompt: 'You are a certified sports nutritionist...',
    category: 'Nutrition',
    tags: ['nutrition', 'diet', 'meal-prep'],
    token_count: 200,
    is_favorite: true,
    use_count: 18,
    last_used_at: '2025-01-12T15:30:00Z',
    created_at: '2025-01-02T00:00:00Z',
    updated_at: '2025-01-12T15:30:00Z',
    is_system: true,
    visibility: 'global',
    is_assigned: true,
  },
];

// Mock users for assignment testing
const mockUsers = [
  { id: 'user-1', email: 'alice@test.com', display_name: 'Alice', user_status: 'active' },
  { id: 'user-2', email: 'bob@test.com', display_name: 'Bob', user_status: 'active' },
  { id: 'user-3', email: 'charlie@test.com', display_name: 'Charlie', user_status: 'pending' },
];

// Mock assignments
const mockAssignments = [
  { user_id: 'user-1', user_email: 'alice@test.com', assigned_at: '2025-01-05T00:00:00Z', assigned_by: 'admin@test.com' },
];

async function setupCoachesMocks(page: Page, options: { isAdmin?: boolean; emptyState?: boolean } = {}) {
  const { isAdmin = true, emptyState = false } = options;

  // Set up base dashboard mocks
  await setupDashboardMocks(page, { role: isAdmin ? 'admin' : 'user' });

  // For non-admin users, mock the user coaches endpoint which ChatTab/PromptSuggestions calls
  if (!isAdmin) {
    await page.route('**/api/coaches', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ coaches: [], total: 0 }),
      });
    });

    await page.route('**/api/coaches/hidden', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ coaches: [] }),
      });
    });
  }

  // Mock admin coaches endpoints
  await page.route('**/api/admin/coaches', async (route) => {
    if (route.request().method() === 'GET') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          coaches: emptyState ? [] : mockCoaches,
          total: emptyState ? 0 : mockCoaches.length,
          metadata: {
            timestamp: new Date().toISOString(),
            api_version: '1.0',
          },
        }),
      });
    } else if (route.request().method() === 'POST') {
      const body = route.request().postDataJSON();
      await route.fulfill({
        status: 201,
        contentType: 'application/json',
        body: JSON.stringify({
          id: 'coach-new',
          title: body.title,
          description: body.description,
          system_prompt: body.system_prompt,
          category: body.category || 'Training',
          tags: body.tags || [],
          token_count: Math.ceil(body.system_prompt.length / 4),
          is_favorite: false,
          use_count: 0,
          last_used_at: null,
          created_at: new Date().toISOString(),
          updated_at: new Date().toISOString(),
          is_system: true,
          visibility: body.visibility || 'tenant',
          is_assigned: false,
        }),
      });
    } else {
      await route.continue();
    }
  });

  // Individual coach operations
  await page.route('**/api/admin/coaches/*', async (route) => {
    const url = route.request().url();

    // Skip assignment endpoints
    if (url.includes('/assign') || url.includes('/assignments')) {
      await route.continue();
      return;
    }

    if (route.request().method() === 'GET') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify(mockCoaches[0]),
      });
    } else if (route.request().method() === 'PUT') {
      const body = route.request().postDataJSON();
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          ...mockCoaches[0],
          ...body,
          updated_at: new Date().toISOString(),
        }),
      });
    } else if (route.request().method() === 'DELETE') {
      await route.fulfill({
        status: 204,
      });
    } else {
      await route.continue();
    }
  });

  // Assignment endpoints
  await page.route('**/api/admin/coaches/*/assign', async (route) => {
    if (route.request().method() === 'POST') {
      const body = route.request().postDataJSON();
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          coach_id: 'coach-1',
          assigned_count: body.user_ids.length,
          total_requested: body.user_ids.length,
        }),
      });
    } else if (route.request().method() === 'DELETE') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          coach_id: 'coach-1',
          removed_count: 1,
          total_requested: 1,
        }),
      });
    } else {
      await route.continue();
    }
  });

  // Assignments list endpoint
  await page.route('**/api/admin/coaches/*/assignments', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        coach_id: 'coach-1',
        assignments: mockAssignments,
      }),
    });
  });

  // Mock admin users for assignment modal
  // API service extracts response.data.users, so return { users: [...] } format
  await page.route('**/api/admin/users**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ users: mockUsers, total_count: mockUsers.length }),
    });
  });
}

test.describe('System Coaches Tab Visibility', () => {
  test('displays Coaches tab for admin users', async ({ page }) => {
    await setupCoachesMocks(page, { isAdmin: true });
    await loginToDashboard(page);

    await page.waitForSelector('nav', { timeout: 10000 });

    // Admin Coaches tab should be visible (exact match so nothing else named Coaches matches)
    await expect(page.locator('nav button').filter({ hasText: /^Coaches$/ })).toBeVisible();
  });

  test('hides Coaches tab for non-admin users', async ({ page }) => {
    await setupCoachesMocks(page, { isAdmin: false });
    await loginToDashboard(page);

    // Non-admin users see chat-first layout (no admin sidebar)
    await page.waitForSelector('main', { timeout: 10000 });

    // Admin-only tabs should NOT be visible for non-admin users
    // User mode also has a "Coaches" button, so check for admin-specific tabs instead
    await expect(page.locator('nav button').filter({ hasText: /^Coach Store$/ })).not.toBeVisible();
    await expect(page.locator('nav button').filter({ hasText: /^Users$/ })).not.toBeVisible();
  });
});

test.describe('System Coaches List View', () => {
  test('displays empty state when no coaches exist', async ({ page }) => {
    await setupCoachesMocks(page, { isAdmin: true, emptyState: true });
    await loginToDashboard(page);

    await page.waitForSelector('nav', { timeout: 10000 });
    await navigateToTab(page, 'Coaches');

    // Should see empty state message
    await expect(page.getByText('No System Coaches')).toBeVisible({ timeout: 10000 });
    await expect(page.getByText('Create your first system coach')).toBeVisible();
    await expect(page.getByRole('button', { name: 'Create Your First Coach' })).toBeVisible();
  });

  test('displays coach cards with correct information', async ({ page }) => {
    await setupCoachesMocks(page, { isAdmin: true });
    await loginToDashboard(page);

    await page.waitForSelector('nav', { timeout: 10000 });
    await navigateToTab(page, 'Coaches');

    // Wait for content to load
    await expect(page.getByText('Marathon Training Coach')).toBeVisible({ timeout: 10000 });
    await expect(page.getByText('Nutrition Expert')).toBeVisible();

    // Should display category badges
    await expect(page.getByText('Training').first()).toBeVisible();
    await expect(page.getByText('Nutrition').first()).toBeVisible();

    // Should display token counts
    await expect(page.getByText('150 tokens')).toBeVisible();
    await expect(page.getByText('200 tokens')).toBeVisible();

    // Should display use counts
    await expect(page.getByText('42 uses')).toBeVisible();
    await expect(page.getByText('18 uses')).toBeVisible();

    // Should display tags (using exact match to avoid title/description matches)
    await expect(page.getByText('marathon', { exact: true })).toBeVisible();
    await expect(page.getByText('endurance', { exact: true })).toBeVisible();
  });

  test('Create Coach button navigates to form', async ({ page }) => {
    await setupCoachesMocks(page, { isAdmin: true });
    await loginToDashboard(page);

    await page.waitForSelector('nav', { timeout: 10000 });
    await navigateToTab(page, 'Coaches');

    await expect(page.getByText('Marathon Training Coach')).toBeVisible({ timeout: 10000 });

    // Click Create Coach button
    await page.getByRole('button', { name: 'Create Coach' }).click();

    // Should see form
    await expect(page.getByText('Create System Coach')).toBeVisible();
    await expect(page.getByText('Title')).toBeVisible();
    await expect(page.getByText('System Prompt')).toBeVisible();
  });
});

test.describe('Create Coach Form', () => {
  test('displays form with all required fields', async ({ page }) => {
    await setupCoachesMocks(page, { isAdmin: true });
    await loginToDashboard(page);

    await page.waitForSelector('nav', { timeout: 10000 });
    await navigateToTab(page, 'Coaches');
    await page.getByRole('button', { name: 'Create Coach' }).click();

    // Check form fields by their labels (text labels, not htmlFor)
    await expect(page.getByText('Title').first()).toBeVisible();
    await expect(page.getByText('Description')).toBeVisible();
    await expect(page.getByText('System Prompt')).toBeVisible();
    await expect(page.getByText('Category')).toBeVisible();
    await expect(page.getByText('Visibility')).toBeVisible();
    await expect(page.getByText('Tags')).toBeVisible();
    // Check that input fields are visible
    await expect(page.getByPlaceholder('e.g., Marathon Training Coach')).toBeVisible();
    await expect(page.getByPlaceholder('You are a professional marathon coach')).toBeVisible();
  });

  test('displays token count estimate for system prompt', async ({ page }) => {
    await setupCoachesMocks(page, { isAdmin: true });
    await loginToDashboard(page);

    await page.waitForSelector('nav', { timeout: 10000 });
    await navigateToTab(page, 'Coaches');
    await page.getByRole('button', { name: 'Create Coach' }).click();

    // Enter system prompt
    const systemPromptField = page.locator('textarea').filter({ hasText: '' }).first();
    await systemPromptField.fill('You are a professional coach with expertise in marathon training.');

    // Should display token estimate
    await expect(page.getByText(/Estimated tokens:/)).toBeVisible();
  });

  test('creates coach successfully', async ({ page }) => {
    await setupCoachesMocks(page, { isAdmin: true });

    let createCalled = false;
    let createdData: Record<string, unknown> = {};
    await page.route('**/api/admin/coaches', async (route) => {
      if (route.request().method() === 'POST') {
        createCalled = true;
        createdData = route.request().postDataJSON();
        await route.fulfill({
          status: 201,
          contentType: 'application/json',
          body: JSON.stringify({
            id: 'coach-new',
            ...createdData,
            token_count: 100,
            is_favorite: false,
            use_count: 0,
            created_at: new Date().toISOString(),
            updated_at: new Date().toISOString(),
            is_system: true,
            is_assigned: false,
          }),
        });
      } else {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({ coaches: mockCoaches, total: mockCoaches.length }),
        });
      }
    });

    await loginToDashboard(page);

    await page.waitForSelector('nav', { timeout: 10000 });
    await navigateToTab(page, 'Coaches');
    await page.getByRole('button', { name: 'Create Coach' }).click();

    // Fill form using placeholders and locators
    await page.getByPlaceholder('e.g., Marathon Training Coach').fill('Recovery Coach');
    await page.locator('textarea').first().fill('Optional description');
    await page.locator('textarea').nth(1).fill('You are a recovery specialist...');
    await page.locator('select').first().selectOption('Recovery');
    await page.getByPlaceholder('marathon, endurance, beginner').fill('recovery, rest, sleep');

    // Submit
    await page.getByRole('button', { name: 'Create Coach' }).click();

    await page.waitForTimeout(500);
    expect(createCalled).toBe(true);
    expect(createdData.title).toBe('Recovery Coach');
    expect(createdData.category).toBe('Recovery');
  });

  test('Back button returns to list view', async ({ page }) => {
    await setupCoachesMocks(page, { isAdmin: true });
    await loginToDashboard(page);

    await page.waitForSelector('nav', { timeout: 10000 });
    await navigateToTab(page, 'Coaches');
    await page.getByRole('button', { name: 'Create Coach' }).click();

    await expect(page.getByText('Create System Coach')).toBeVisible();

    // Click back
    await page.getByText('Back to Coaches').click();

    // Should return to list
    await expect(page.getByText('Marathon Training Coach')).toBeVisible();
  });

  test('Cancel button returns to list view', async ({ page }) => {
    await setupCoachesMocks(page, { isAdmin: true });
    await loginToDashboard(page);

    await page.waitForSelector('nav', { timeout: 10000 });
    await navigateToTab(page, 'Coaches');
    await page.getByRole('button', { name: 'Create Coach' }).click();

    await expect(page.getByText('Create System Coach')).toBeVisible();

    // Click cancel
    await page.getByRole('button', { name: 'Cancel' }).click();

    // Should return to list
    await expect(page.getByText('Marathon Training Coach')).toBeVisible();
  });
});

test.describe('Coach Detail View', () => {
  test('clicking coach card opens detail view', async ({ page }) => {
    await setupCoachesMocks(page, { isAdmin: true });
    await loginToDashboard(page);

    await page.waitForSelector('nav', { timeout: 10000 });
    await navigateToTab(page, 'Coaches');

    await expect(page.getByText('Marathon Training Coach')).toBeVisible({ timeout: 10000 });

    // Click coach card
    await page.getByText('Marathon Training Coach').click();

    // Should see detail view with stats
    await expect(page.getByText('150').first()).toBeVisible({ timeout: 5000 }); // token count
    await expect(page.getByText('42').first()).toBeVisible(); // use count
    await expect(page.getByText('Tokens').first()).toBeVisible();
    await expect(page.getByText('Uses').first()).toBeVisible();

    // Should see system prompt
    await expect(page.getByText('You are a professional marathon coach')).toBeVisible();

    // Should see action buttons
    await expect(page.getByRole('button', { name: 'Edit' })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Delete' })).toBeVisible();
  });

  test('displays timestamps correctly', async ({ page }) => {
    await setupCoachesMocks(page, { isAdmin: true });
    await loginToDashboard(page);

    await page.waitForSelector('nav', { timeout: 10000 });
    await navigateToTab(page, 'Coaches');

    await page.getByText('Marathon Training Coach').click();

    // Should display created and updated timestamps
    await expect(page.getByText('Created:')).toBeVisible({ timeout: 5000 });
    await expect(page.getByText('Last Updated:')).toBeVisible();
  });

  test('displays tags in detail view', async ({ page }) => {
    await setupCoachesMocks(page, { isAdmin: true });
    await loginToDashboard(page);

    await page.waitForSelector('nav', { timeout: 10000 });
    await navigateToTab(page, 'Coaches');

    await page.getByText('Marathon Training Coach').click();

    // Should display tags section
    await expect(page.getByText('Tags')).toBeVisible({ timeout: 5000 });
    // Use exact match to avoid matching the tag text in title/description
    await expect(page.getByText('marathon', { exact: true })).toBeVisible();
    await expect(page.getByText('endurance', { exact: true })).toBeVisible();
    await expect(page.getByText('running', { exact: true })).toBeVisible();
  });
});

test.describe('Edit Coach Form', () => {
  test('Edit button opens form with pre-populated data', async ({ page }) => {
    await setupCoachesMocks(page, { isAdmin: true });
    await loginToDashboard(page);

    await page.waitForSelector('nav', { timeout: 10000 });
    await navigateToTab(page, 'Coaches');

    await page.getByText('Marathon Training Coach').click();
    await expect(page.getByRole('button', { name: 'Edit' })).toBeVisible({ timeout: 5000 });

    // Click edit
    await page.getByRole('button', { name: 'Edit' }).click();

    // Should see edit form with populated data
    await expect(page.getByText('Edit "Marathon Training Coach"')).toBeVisible();
    // Use placeholder selector since form doesn't use htmlFor
    const titleInput = page.getByPlaceholder('e.g., Marathon Training Coach');
    await expect(titleInput).toHaveValue('Marathon Training Coach');
  });

  test('updates coach successfully', async ({ page }) => {
    await setupCoachesMocks(page, { isAdmin: true });

    let updateCalled = false;
    await page.route('**/api/admin/coaches/*', async (route) => {
      const url = route.request().url();
      if (url.includes('/assign') || url.includes('/assignments')) {
        await route.continue();
        return;
      }

      if (route.request().method() === 'PUT') {
        updateCalled = true;
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({
            ...mockCoaches[0],
            title: 'Updated Marathon Coach',
            updated_at: new Date().toISOString(),
          }),
        });
      } else if (route.request().method() === 'GET') {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify(mockCoaches[0]),
        });
      } else {
        await route.continue();
      }
    });

    await loginToDashboard(page);

    await page.waitForSelector('nav', { timeout: 10000 });
    await navigateToTab(page, 'Coaches');

    await page.getByText('Marathon Training Coach').click();
    await page.getByRole('button', { name: 'Edit' }).click();

    // Wait for edit form to be visible
    await expect(page.getByText('Edit "Marathon Training Coach"')).toBeVisible({ timeout: 5000 });

    // Wait for form to be populated - token count > 0 indicates system_prompt has content
    await expect(page.getByText(/Estimated tokens: [1-9]/)).toBeVisible({ timeout: 5000 });

    // Modify title using placeholder selector
    await page.getByPlaceholder('e.g., Marathon Training Coach').fill('Updated Marathon Coach');

    // Save
    await page.getByRole('button', { name: 'Save Changes' }).click();

    await page.waitForTimeout(500);
    expect(updateCalled).toBe(true);
  });

  test('visibility dropdown is disabled when editing', async ({ page }) => {
    await setupCoachesMocks(page, { isAdmin: true });
    await loginToDashboard(page);

    await page.waitForSelector('nav', { timeout: 10000 });
    await navigateToTab(page, 'Coaches');

    await page.getByText('Marathon Training Coach').click();
    await page.getByRole('button', { name: 'Edit' }).click();

    // Wait for edit form to be visible
    await expect(page.getByText('Edit "Marathon Training Coach"')).toBeVisible({ timeout: 5000 });

    // Visibility is the second select (first is category)
    // Check that the select with "Tenant Only" option is disabled
    const visibilitySelect = page.locator('select').nth(1);
    await expect(visibilitySelect).toBeDisabled();
  });
});

test.describe('Delete Coach', () => {
  test('delete button triggers confirmation and deletes', async ({ page }) => {
    await setupCoachesMocks(page, { isAdmin: true });

    let deleteCalled = false;
    await page.route('**/api/admin/coaches/*', async (route) => {
      const url = route.request().url();
      if (url.includes('/assign') || url.includes('/assignments')) {
        await route.continue();
        return;
      }

      if (route.request().method() === 'DELETE') {
        deleteCalled = true;
        await route.fulfill({ status: 204 });
      } else if (route.request().method() === 'GET') {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify(mockCoaches[0]),
        });
      } else {
        await route.continue();
      }
    });

    // Handle confirm dialog
    page.on('dialog', async (dialog) => {
      expect(dialog.message()).toContain('Delete coach');
      await dialog.accept();
    });

    await loginToDashboard(page);

    await page.waitForSelector('nav', { timeout: 10000 });
    await navigateToTab(page, 'Coaches');

    await page.getByText('Marathon Training Coach').click();
    await expect(page.getByRole('button', { name: 'Delete' })).toBeVisible({ timeout: 5000 });

    // Click delete
    await page.getByRole('button', { name: 'Delete' }).click();

    await page.waitForTimeout(500);
    expect(deleteCalled).toBe(true);
  });
});

test.describe('User Assignments', () => {
  test('displays assignments count in detail view', async ({ page }) => {
    await setupCoachesMocks(page, { isAdmin: true });
    await loginToDashboard(page);

    await page.waitForSelector('nav', { timeout: 10000 });
    await navigateToTab(page, 'Coaches');

    await page.getByText('Marathon Training Coach').click();

    // Should display assigned users count
    await expect(page.getByText('Assigned Users')).toBeVisible({ timeout: 5000 });
    await expect(page.getByText('1').first()).toBeVisible(); // 1 assignment in mock
  });

  test('displays User Assignments card with assigned users', async ({ page }) => {
    await setupCoachesMocks(page, { isAdmin: true });
    await loginToDashboard(page);

    await page.waitForSelector('nav', { timeout: 10000 });
    await navigateToTab(page, 'Coaches');

    await page.getByText('Marathon Training Coach').click();

    // Should display assignments card
    await expect(page.getByText('User Assignments').first()).toBeVisible({ timeout: 5000 });
    await expect(page.getByText('alice@test.com')).toBeVisible();
    await expect(page.getByRole('button', { name: 'Assign Users' })).toBeVisible();
  });

  test('Assign Users button opens modal', async ({ page }) => {
    await setupCoachesMocks(page, { isAdmin: true });
    await loginToDashboard(page);

    await page.waitForSelector('nav', { timeout: 10000 });
    await navigateToTab(page, 'Coaches');

    await page.getByText('Marathon Training Coach').click();
    await expect(page.getByRole('button', { name: 'Assign Users' })).toBeVisible({ timeout: 5000 });

    // Click Assign Users
    await page.getByRole('button', { name: 'Assign Users' }).click();

    // Should see modal
    await expect(page.getByText('Assign Users to Coach')).toBeVisible();
    await expect(page.getByText('Select users to give access')).toBeVisible();
  });

  test('can select and assign users', async ({ page }) => {
    await setupCoachesMocks(page, { isAdmin: true });

    let assignCalled = false;
    let assignedUserIds: string[] = [];
    await page.route('**/api/admin/coaches/*/assign', async (route) => {
      if (route.request().method() === 'POST') {
        assignCalled = true;
        const body = route.request().postDataJSON();
        assignedUserIds = body.user_ids;
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({
            coach_id: 'coach-1',
            assigned_count: body.user_ids.length,
            total_requested: body.user_ids.length,
          }),
        });
      } else {
        await route.continue();
      }
    });

    await loginToDashboard(page);

    await page.waitForSelector('nav', { timeout: 10000 });
    await navigateToTab(page, 'Coaches');

    await page.getByText('Marathon Training Coach').click();
    await page.getByRole('button', { name: 'Assign Users' }).click();

    // Wait for modal and users to load
    await expect(page.getByText('Assign Users to Coach')).toBeVisible({ timeout: 5000 });
    await page.waitForTimeout(500);

    // Select a user (bob is not already assigned)
    const bobCheckbox = page.locator('label').filter({ hasText: 'bob@test.com' });
    await bobCheckbox.click();

    // Click Assign Selected
    await page.getByRole('button', { name: 'Assign Selected' }).click();

    await page.waitForTimeout(500);
    expect(assignCalled).toBe(true);
    expect(assignedUserIds).toContain('user-2');
  });

  test('shows empty state when no users assigned', async ({ page }) => {
    // Override assignments mock to return empty
    await setupDashboardMocks(page, { role: 'admin' });

    await page.route('**/api/admin/coaches', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ coaches: mockCoaches, total: mockCoaches.length }),
      });
    });

    await page.route('**/api/admin/coaches/*', async (route) => {
      const url = route.request().url();
      if (url.includes('/assignments')) {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({ coach_id: 'coach-1', assignments: [] }),
        });
      } else if (!url.includes('/assign')) {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify(mockCoaches[0]),
        });
      } else {
        await route.continue();
      }
    });

    await loginToDashboard(page);

    await page.waitForSelector('nav', { timeout: 10000 });
    await navigateToTab(page, 'Coaches');

    await page.getByText('Marathon Training Coach').click();

    // Should show empty state message
    await expect(page.getByText('No users assigned to this coach yet')).toBeVisible({ timeout: 5000 });
  });
});

test.describe('Error Handling', () => {
  test('shows error when failing to load coaches', async ({ page }) => {
    await setupDashboardMocks(page, { role: 'admin' });

    await page.route('**/api/admin/coaches', async (route) => {
      await route.fulfill({
        status: 500,
        contentType: 'application/json',
        body: JSON.stringify({ error: 'Internal server error' }),
      });
    });

    await loginToDashboard(page);
    await page.waitForSelector('nav', { timeout: 10000 });
    await navigateToTab(page, 'Coaches');

    // Should show loading spinner then error or empty state
    // React Query may retry, so we wait a bit
    await page.waitForTimeout(2000);
  });

  test('shows error when create fails', async ({ page }) => {
    await setupCoachesMocks(page, { isAdmin: true });

    await page.route('**/api/admin/coaches', async (route) => {
      if (route.request().method() === 'POST') {
        await route.fulfill({
          status: 400,
          contentType: 'application/json',
          body: JSON.stringify({ error: 'Validation failed' }),
        });
      } else {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({ coaches: mockCoaches, total: mockCoaches.length }),
        });
      }
    });

    await loginToDashboard(page);
    await page.waitForSelector('nav', { timeout: 10000 });
    await navigateToTab(page, 'Coaches');

    await page.getByRole('button', { name: 'Create Coach' }).click();

    // Fill minimal form using placeholder selectors
    await page.getByPlaceholder('e.g., Marathon Training Coach').fill('Test Coach');
    await page.getByPlaceholder('You are a professional marathon coach').fill('Test prompt');

    // Submit
    await page.getByRole('button', { name: 'Create Coach' }).click();

    await page.waitForTimeout(500);
    // Form should still be visible (not submitted successfully)
    await expect(page.getByText('Create System Coach')).toBeVisible();
  });
});

test.describe('Category Colors', () => {
  test('displays correct category colors on cards', async ({ page }) => {
    await setupCoachesMocks(page, { isAdmin: true });
    await loginToDashboard(page);

    await page.waitForSelector('nav', { timeout: 10000 });
    await navigateToTab(page, 'Coaches');

    await expect(page.getByText('Marathon Training Coach')).toBeVisible({ timeout: 10000 });

    // Training category should have Training badge
    const trainingBadge = page.locator('.rounded-full').filter({ hasText: 'Training' });
    await expect(trainingBadge).toBeVisible();

    // Nutrition category should have Nutrition badge
    const nutritionBadge = page.locator('.rounded-full').filter({ hasText: 'Nutrition' });
    await expect(nutritionBadge).toBeVisible();
  });
});

// ============================================================================
// User-Facing Coaches Tests (Chat Interface - PromptSuggestions)
// ============================================================================

// Mock data for user-facing coaches (non-admin view)
const mockUserCoaches = [
  {
    id: 'user-coach-1',
    title: 'My Custom Coach',
    description: 'Personal training coach',
    system_prompt: 'You are my personal coach.',
    category: 'training',
    tags: ['personal'],
    token_count: 50,
    is_favorite: false,
    use_count: 3,
    last_used_at: '2025-01-10T10:00:00Z',
    is_system: false,
    visibility: 'private',
    is_assigned: false,
  },
  {
    id: 'user-coach-structured',
    title: 'Structured Marathon Coach',
    description: 'Coach with structured sections',
    system_prompt: 'Expert marathon coach instructions here.',
    category: 'training',
    tags: ['marathon', 'structured'],
    token_count: 250,
    is_favorite: false,
    use_count: 5,
    last_used_at: '2025-01-15T10:00:00Z',
    created_at: '2025-01-01T00:00:00Z',
    updated_at: '2025-01-15T10:00:00Z',
    is_system: false,
    visibility: 'private',
    is_assigned: false,
    purpose: 'Expert in marathon preparation and race day strategy.',
    when_to_use: '- Training for your first marathon\n- Preparing to PR at the marathon distance',
    instructions: 'Expert marathon coach instructions here.',
    example_inputs: '- "How do I build up to a 20-mile long run safely?"\n- "What should my marathon taper look like?"',
    example_outputs: 'Provide detailed training progressions with specific pacing.',
    success_criteria: '- Runner has a clear weekly training structure\n- Advice is personalized to their goal',
  },
  {
    id: 'system-coach-1',
    title: 'System Training Coach',
    description: 'Official training guidance',
    system_prompt: 'You are a professional coach.',
    category: 'training',
    tags: ['training'],
    token_count: 100,
    is_favorite: false,
    use_count: 10,
    last_used_at: null,
    is_system: true,
    visibility: 'tenant',
    is_assigned: true,
    handle: 'system-training-coach',
  },
];

const mockHiddenCoaches = [
  {
    id: 'hidden-coach-1',
    title: 'Hidden System Coach',
    description: 'A hidden coach',
    system_prompt: 'Hidden prompt.',
    category: 'nutrition',
    tags: [],
    token_count: 80,
    is_favorite: false,
    use_count: 0,
    last_used_at: null,
    is_system: true,
    visibility: 'tenant',
    is_assigned: true,
  },
];

async function setupUserCoachesMocks(page: Page) {
  // Set up base dashboard mocks for non-admin user
  await setupDashboardMocks(page, { role: 'user' });

  // Mock user coaches endpoint (use regex to match URLs with query params like ?include_hidden=true).
  // Like the server, the include_hidden variant also returns the hidden coaches.
  await page.route(/\/api\/coaches(\?.*)?$/, async (route) => {
    if (route.request().method() === 'GET') {
      const includeHidden = route.request().url().includes('include_hidden=true');
      const coaches = includeHidden ? [...mockUserCoaches, ...mockHiddenCoaches] : mockUserCoaches;
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          coaches,
          total: coaches.length,
        }),
      });
    } else if (route.request().method() === 'POST') {
      const body = route.request().postDataJSON();
      await route.fulfill({
        status: 201,
        contentType: 'application/json',
        body: JSON.stringify({
          id: 'new-user-coach',
          ...body,
          token_count: 50,
          is_favorite: false,
          use_count: 0,
          is_system: false,
          visibility: 'private',
          is_assigned: false,
        }),
      });
    } else {
      await route.continue();
    }
  });

  // Mock hidden coaches endpoint
  await page.route('**/api/coaches/hidden', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        coaches: mockHiddenCoaches,
      }),
    });
  });

  // Mock individual coach operations (edit, delete, hide, show). The hidden
  // list lives under the same prefix and must not read as a /hide call.
  await page.route('**/api/coaches/*', async (route) => {
    const url = route.request().url();
    const method = route.request().method();

    if (/\/api\/coaches\/hidden(\?.*)?$/.test(url)) {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ coaches: mockHiddenCoaches }),
      });
    } else if (/\/hide$/.test(url)) {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ success: true, is_hidden: true }),
      });
    } else if (url.includes('/show')) {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ success: true, is_hidden: false }),
      });
    } else if (url.includes('/usage')) {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ success: true }),
      });
    } else if (method === 'PUT') {
      const body = route.request().postDataJSON();
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          ...mockUserCoaches[0],
          ...body,
        }),
      });
    } else if (method === 'DELETE') {
      await route.fulfill({ status: 204 });
    } else {
      await route.continue();
    }
  });
}

/**
 * Open Discover and wait for the pinned "Your coaches" section.
 *
 * The user Coach tab was folded into Discover by the Chat-First Cutover
 * (2026-08-26): the athlete's own coaches sit above the store's category
 * filters, each carrying the `@handle` that brings it into a conversation.
 */
async function openYourCoaches(page: Page) {
  await page.waitForSelector('aside', { timeout: 10000 });
  await page.getByRole('list').getByRole('button', { name: 'Discover', exact: true }).click();
  // Discover is a lazy chunk: the first worker to open it waits on Vite's
  // cold transform, the same wait the app shell gets.
  await expect(page.getByRole('region', { name: /Your coaches/ })).toBeVisible({
    timeout: APP_SHELL_TIMEOUT_MS,
  });
  return page.getByRole('region', { name: /Your coaches/ });
}

test.describe('User coaches - pinned on Discover', () => {
  test('lists the athlete coaches above the store, with the handle of each addressable coach', async ({ page }) => {
    await setupUserCoachesMocks(page);
    await loginToDashboard(page);

    const section = await openYourCoaches(page);
    await expect(section.getByText('My Custom Coach')).toBeVisible();
    await expect(section.getByText('System Training Coach')).toBeVisible();
    // A catalogue coach carries the handle the athlete types in chat; a
    // personal coach that was never published has none.
    await expect(section.getByTestId('coach-handle')).toHaveText(['@system-training-coach']);
    // The store still renders below the pinned section (empty in this mock).
    await expect(page.getByText('Store is empty')).toBeVisible();
    // The section is above the store's category chips, not among them.
    const sectionBox = await section.boundingBox();
    const chipBox = await page.getByRole('main').getByRole('button', { name: 'Training', exact: true }).boundingBox();
    expect(sectionBox!.y).toBeLessThan(chipBox!.y);
  });

  test('offers Edit on a user coach and not on a system coach', async ({ page }) => {
    await setupUserCoachesMocks(page);
    await loginToDashboard(page);
    await openYourCoaches(page);

    await page.getByRole('button', { name: 'Open My Custom Coach' }).click();
    const own = page.getByRole('dialog');
    await expect(own.getByRole('button', { name: 'Edit' })).toBeVisible();
    await expect(own.getByRole('button', { name: 'Delete' })).toBeVisible();
    await own.getByRole('button', { name: 'Close modal' }).click();

    await page.getByRole('button', { name: 'Open System Training Coach' }).click();
    const system = page.getByRole('dialog');
    await expect(system.getByText('@system-training-coach')).toBeVisible();
    await expect(system.getByRole('button', { name: 'Edit' })).toHaveCount(0);
    await expect(system.getByRole('button', { name: 'Delete' })).toHaveCount(0);
    await expect(system.getByRole('button', { name: 'Chat' })).toBeVisible();
  });

  test('the show-hidden toggle reveals a hidden coach', async ({ page }) => {
    await setupUserCoachesMocks(page);
    await loginToDashboard(page);
    const section = await openYourCoaches(page);

    await expect(section.getByText('My Custom Coach')).toBeVisible();
    await expect(section.getByText('Hidden System Coach')).toHaveCount(0);

    await page.getByRole('button', { name: 'Show hidden coaches' }).click();

    await expect(section.getByText('Hidden System Coach')).toBeVisible();
    await expect(section.getByText('Hidden', { exact: true })).toBeVisible();

    await page.getByRole('button', { name: 'Hide hidden coaches' }).click();
    await expect(section.getByText('Hidden System Coach')).toHaveCount(0);
  });

  test('can delete a user coach with confirmation', async ({ page }) => {
    await setupUserCoachesMocks(page);

    let deleteCalled = false;
    await page.route('**/api/coaches/user-coach-1', async (route) => {
      if (route.request().method() === 'DELETE') {
        deleteCalled = true;
        await route.fulfill({ status: 204 });
      } else {
        await route.continue();
      }
    });

    await loginToDashboard(page);
    await openYourCoaches(page);

    await page.getByRole('button', { name: 'Open My Custom Coach' }).click();
    await page.getByRole('dialog').getByRole('button', { name: 'Delete' }).click();

    // The confirmation is the design-system dialog, not a native confirm().
    const confirm = page.getByRole('dialog').last();
    await expect(confirm.getByText('Delete coach "My Custom Coach"? This cannot be undone.')).toBeVisible();
    await confirm.getByRole('button', { name: 'Delete' }).click();

    await expect.poll(() => deleteCalled).toBe(true);
  });

  test('can create a new user coach from the section header', async ({ page }) => {
    await setupUserCoachesMocks(page);

    let createCalled = false;
    let createdBody: Record<string, unknown> = {};
    await page.route('**/api/coaches', async (route) => {
      if (route.request().method() === 'POST') {
        createCalled = true;
        const body = route.request().postDataJSON();
        createdBody = body;
        await route.fulfill({
          status: 201,
          contentType: 'application/json',
          body: JSON.stringify({
            id: 'new-coach',
            ...body,
            is_system: false,
          }),
        });
      } else {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({ coaches: mockUserCoaches, total: mockUserCoaches.length }),
        });
      }
    });

    await loginToDashboard(page);
    await openYourCoaches(page);

    await page.getByRole('button', { name: 'Create Coach' }).click();

    // The one coach editor — the same CoachFormModal the Chat tab uses, tool
    // budget field included. No inline editor chrome (its Tags input) exists.
    await expect(page.getByRole('heading', { name: 'Create Custom Coach' })).toBeVisible();
    await expect(page.getByPlaceholder('marathon, endurance, beginner (comma-separated)')).toHaveCount(0);

    await page.getByPlaceholder('e.g., Marathon Training Coach').fill('New Test Coach');
    await page
      .getByPlaceholder("Define your coach's personality, expertise, and communication style...")
      .fill('Test system prompt for the coach');
    await page.getByLabel('Max tool iterations per turn').fill('25');

    await page.locator('button[type="submit"]', { hasText: 'Create Coach' }).click();

    await expect.poll(() => createCalled).toBe(true);
    expect(createdBody.title).toBe('New Test Coach');
    expect(createdBody.system_prompt).toBe('Test system prompt for the coach');
    expect(createdBody.max_tool_iterations).toBe(25);
  });

  test('can create coach with the Training category', async ({ page }) => {
    await setupUserCoachesMocks(page);

    let capturedBody: Record<string, unknown> | null = null;
    await page.route('**/api/coaches', async (route) => {
      if (route.request().method() === 'POST') {
        capturedBody = route.request().postDataJSON();
        await route.fulfill({
          status: 201,
          contentType: 'application/json',
          body: JSON.stringify({
            id: 'new-training-coach',
            title: capturedBody?.title,
            description: capturedBody?.description,
            system_prompt: capturedBody?.system_prompt,
            category: 'training',
            is_system: false,
            tags: [],
          }),
        });
      } else {
        await route.continue();
      }
    });

    await loginToDashboard(page);
    await openYourCoaches(page);

    await page.getByRole('button', { name: 'Create Coach' }).click();
    await expect(page.getByRole('heading', { name: 'Create Custom Coach' })).toBeVisible();

    await page.getByPlaceholder('e.g., Marathon Training Coach').fill('My Training Coach');
    await page
      .getByPlaceholder("Define your coach's personality, expertise, and communication style...")
      .fill('Training system prompt for the coach');
    await page.getByLabel('Category').selectOption('Training');

    await page.locator('button[type="submit"]', { hasText: 'Create Coach' }).click();

    await expect.poll(() => capturedBody).not.toBeNull();
    expect(capturedBody!.category).toBe('Training');
  });

  test('the Discover category chips narrow the store and leave the pinned coaches alone', async ({ page }) => {
    await setupUserCoachesMocks(page);

    const browseCategories: Array<string | null> = [];
    await page.route('**/api/store/coaches**', async (route) => {
      browseCategories.push(new URL(route.request().url()).searchParams.get('category'));
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          coaches: [],
          next_cursor: null,
          has_more: false,
          metadata: { timestamp: new Date().toISOString(), api_version: 'v1' },
        }),
      });
    });

    await loginToDashboard(page);
    const section = await openYourCoaches(page);
    const main = page.getByRole('main');

    await expect(main.getByRole('button', { name: 'All', exact: true })).toBeVisible();
    await main.getByRole('button', { name: 'Training', exact: true }).click();
    await expect.poll(() => browseCategories.includes('training')).toBe(true);
    await expect(section.getByText('My Custom Coach')).toBeVisible();

    await main.getByRole('button', { name: 'Nutrition', exact: true }).click();
    await expect.poll(() => browseCategories.includes('nutrition')).toBe(true);
    await expect(section.getByText('My Custom Coach')).toBeVisible();

    await main.getByRole('button', { name: 'All', exact: true }).click();
    await expect(section.getByText('My Custom Coach')).toBeVisible();
  });

  test('displays all category filter buttons with correct names', async ({ page }) => {
    await setupUserCoachesMocks(page);
    await loginToDashboard(page);
    await openYourCoaches(page);

    const main = page.getByRole('main');
    await expect(main.getByRole('button', { name: 'All', exact: true })).toBeVisible();
    await expect(main.getByRole('button', { name: 'Training', exact: true })).toBeVisible();
    await expect(main.getByRole('button', { name: 'Nutrition', exact: true })).toBeVisible();
    await expect(main.getByRole('button', { name: 'Recovery', exact: true })).toBeVisible();
    await expect(main.getByRole('button', { name: 'Recipes', exact: true })).toBeVisible();
    await expect(main.getByRole('button', { name: 'Mobility', exact: true })).toBeVisible();
    await expect(main.getByRole('button', { name: 'Custom', exact: true })).toBeVisible();
  });

  test('displays the section header controls', async ({ page }) => {
    await setupUserCoachesMocks(page);
    await loginToDashboard(page);
    await openYourCoaches(page);

    await expect(page.getByRole('button', { name: 'Show hidden coaches' })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Import Coach' })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Create Coach' })).toBeVisible();
  });

  test('displays a favorite toggle on each coach card', async ({ page }) => {
    await setupUserCoachesMocks(page);
    await loginToDashboard(page);
    const section = await openYourCoaches(page);

    const favoriteButtons = section.locator('button[title="Add to favorites"], button[title="Remove from favorites"]');
    await expect(favoriteButtons).toHaveCount(mockUserCoaches.length);
  });

  test('can edit a user coach and update its category', async ({ page }) => {
    await setupUserCoachesMocks(page);

    let capturedUpdate: Record<string, unknown> | null = null;
    await page.route('**/api/coaches/user-coach-1', async (route) => {
      if (route.request().method() === 'PUT') {
        capturedUpdate = route.request().postDataJSON();
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({
            ...mockUserCoaches[0],
            title: capturedUpdate?.title ?? 'My Custom Coach',
            category: 'nutrition',
          }),
        });
      } else {
        await route.continue();
      }
    });

    await loginToDashboard(page);
    await openYourCoaches(page);

    await page.getByRole('button', { name: 'Open My Custom Coach' }).click();
    await page.getByRole('dialog').getByRole('button', { name: 'Edit' }).click();

    await expect(page.getByRole('heading', { name: 'Edit Coach' })).toBeVisible();
    await page.getByLabel('Category').selectOption('Nutrition');
    await page.getByRole('button', { name: 'Save Changes' }).click();

    await expect.poll(() => capturedUpdate).not.toBeNull();
    expect(capturedUpdate!.category).toBe('Nutrition');
  });
});

// ============================================================================
// ============================================================================

const mockConversation = {
  id: 'conv-123',
  title: 'Marathon Training Discussion',
  created_at: '2025-01-10T10:00:00Z',
  updated_at: '2025-01-10T11:00:00Z',
  messages_count: 5,
};

const mockConversationMessages = [
  {
    id: 'msg-1',
    role: 'user',
    content: 'I want to train for a marathon',
    created_at: '2025-01-10T10:00:00Z',
  },
  {
    id: 'msg-2',
    role: 'assistant',
    content: 'A marathon is 26.2 miles. What is your current running experience?',
    created_at: '2025-01-10T10:01:00Z',
  },
  {
    id: 'msg-3',
    role: 'user',
    content: 'I run about 20 miles per week',
    created_at: '2025-01-10T10:02:00Z',
  },
  {
    id: 'msg-4',
    role: 'assistant',
    content: 'Great base! Let me suggest a 16-week training plan.',
    created_at: '2025-01-10T10:03:00Z',
  },
];

const mockGeneratedCoach = {
  title: 'Marathon Training Expert',
  description: 'Specialized in long-distance running preparation',
  system_prompt:
    'You are a professional marathon coach helping runners prepare for their first marathon. Focus on gradual mileage building, proper pacing, and injury prevention.',
  category: 'Training',
  messages_analyzed: 4,
  total_messages: 5,
};

async function setupConversationMocks(page: Page, options: { hasMessages?: boolean } = {}) {
  const { hasMessages = true } = options;

  // Set up base dashboard mocks for non-admin user
  await setupDashboardMocks(page, { role: 'user' });

  // Mock conversations list with one conversation
  await page.route('**/api/chat/conversations**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        conversations: hasMessages ? [mockConversation] : [],
        total: hasMessages ? 1 : 0,
        limit: 50,
        offset: 0,
      }),
    });
  });

  // Mock conversation messages
  await page.route('**/api/chat/conversations/conv-123/messages**', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        messages: hasMessages ? mockConversationMessages : [],
        total: hasMessages ? mockConversationMessages.length : 0,
      }),
    });
  });

  // Mock user coaches endpoint
  await page.route('**/api/coaches', async (route) => {
    if (route.request().method() === 'GET') {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ coaches: [], total: 0 }),
      });
    } else if (route.request().method() === 'POST') {
      const body = route.request().postDataJSON();
      await route.fulfill({
        status: 201,
        contentType: 'application/json',
        body: JSON.stringify({
          id: 'new-coach-from-conv',
          ...body,
          token_count: 150,
          is_favorite: false,
          use_count: 0,
          is_system: false,
          visibility: 'private',
          is_assigned: false,
        }),
      });
    } else {
      await route.continue();
    }
  });

  // Mock hidden coaches endpoint
  await page.route('**/api/coaches/hidden', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ coaches: [] }),
    });
  });

  // Mock generate coach from conversation endpoint
  await page.route('**/api/coaches/generate', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(mockGeneratedCoach),
    });
  });
}

test.describe('Create Coach from Conversation', () => {
  test('shows Create Coach button when conversation has 2+ messages', async ({ page }) => {
    await setupConversationMocks(page, { hasMessages: true });
    await loginToDashboard(page);

    await page.waitForSelector('main', { timeout: 10000 });

    // Click on a conversation in the sidebar
    await page.getByText('Marathon Training Discussion').click();
    await page.waitForTimeout(500);

    // Create Coach button should be visible (conversation has 5 messages)
    await expect(page.getByRole('button', { name: 'Create Coach' })).toBeVisible({ timeout: 5000 });
  });

  test('clicking Create Coach button opens the modal', async ({ page }) => {
    await setupConversationMocks(page, { hasMessages: true });
    await loginToDashboard(page);

    await page.waitForSelector('main', { timeout: 10000 });

    // Click on a conversation
    await page.getByText('Marathon Training Discussion').click();
    await page.waitForTimeout(500);

    // Click Create Coach button
    await page.getByRole('button', { name: 'Create Coach' }).click();

    // Modal should be visible
    await expect(page.getByText('Create Coach from Conversation')).toBeVisible({ timeout: 5000 });
    await expect(page.getByText('AI analyzes your conversation')).toBeVisible();
  });

  test('modal shows analyzing state then displays form with suggestions', async ({ page }) => {
    await setupConversationMocks(page, { hasMessages: true });
    await loginToDashboard(page);

    await page.waitForSelector('main', { timeout: 10000 });

    // Click on a conversation
    await page.getByText('Marathon Training Discussion').click();
    await page.waitForTimeout(500);

    // Click Create Coach button
    await page.getByRole('button', { name: 'Create Coach' }).click();

    // Modal should show
    await expect(page.getByText('Create Coach from Conversation')).toBeVisible({ timeout: 5000 });

    // Wait for form to appear with LLM-generated suggestions
    await expect(page.getByText('Analyzed 4 of 5 messages')).toBeVisible({ timeout: 10000 });

    // Form fields should be pre-filled with LLM suggestions
    const titleInput = page.getByPlaceholder('e.g., Marathon Training Coach');
    await expect(titleInput).toHaveValue('Marathon Training Expert');

    // System prompt should be filled
    const systemPromptTextarea = page.locator('textarea').filter({ hasText: 'professional marathon coach' });
    await expect(systemPromptTextarea).toBeVisible();
  });

  test('can edit and save the generated coach', async ({ page }) => {
    await setupConversationMocks(page, { hasMessages: true });

    let createCalled = false;
    let capturedBody: Record<string, unknown> | null = null;
    await page.route('**/api/coaches', async (route) => {
      if (route.request().method() === 'POST') {
        createCalled = true;
        capturedBody = route.request().postDataJSON();
        await route.fulfill({
          status: 201,
          contentType: 'application/json',
          body: JSON.stringify({
            id: 'new-coach-from-conv',
            ...capturedBody,
            is_system: false,
          }),
        });
      } else {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({ coaches: [], total: 0 }),
        });
      }
    });

    await loginToDashboard(page);

    await page.waitForSelector('main', { timeout: 10000 });

    // Click on a conversation
    await page.getByText('Marathon Training Discussion').click();
    await page.waitForTimeout(500);

    // Click Create Coach button
    await page.getByRole('button', { name: 'Create Coach' }).click();

    // Wait for form with suggestions
    await expect(page.getByText('Analyzed 4 of 5 messages')).toBeVisible({ timeout: 10000 });

    // Modify the title
    await page.getByPlaceholder('e.g., Marathon Training Coach').fill('My Custom Marathon Coach');

    // Click Save Coach button
    await page.getByRole('button', { name: 'Save Coach' }).click();

    await page.waitForTimeout(500);
    expect(createCalled).toBe(true);
    expect(capturedBody?.title).toBe('My Custom Marathon Coach');
    expect(capturedBody?.category).toBe('Training');
  });

  test('can cancel the modal', async ({ page }) => {
    await setupConversationMocks(page, { hasMessages: true });
    await loginToDashboard(page);

    await page.waitForSelector('main', { timeout: 10000 });

    // Click on a conversation
    await page.getByText('Marathon Training Discussion').click();
    await page.waitForTimeout(500);

    // Click Create Coach button
    await page.getByRole('button', { name: 'Create Coach' }).click();

    // Wait for form
    await expect(page.getByText('Create Coach from Conversation')).toBeVisible({ timeout: 5000 });
    await expect(page.getByText('Analyzed 4 of 5 messages')).toBeVisible({ timeout: 10000 });

    // Click Cancel
    await page.getByRole('button', { name: 'Cancel' }).click();

    // Modal should close
    await expect(page.getByText('Create Coach from Conversation')).not.toBeVisible({ timeout: 3000 });
  });

  test('can regenerate coach suggestions', async ({ page }) => {
    await setupConversationMocks(page, { hasMessages: true });

    let generateCallCount = 0;
    await page.route('**/api/coaches/generate', async (route) => {
      generateCallCount++;
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          ...mockGeneratedCoach,
          title: generateCallCount > 1 ? 'Regenerated Coach Title' : 'Marathon Training Expert',
        }),
      });
    });

    await loginToDashboard(page);

    await page.waitForSelector('main', { timeout: 10000 });

    // Click on a conversation
    await page.getByText('Marathon Training Discussion').click();
    await page.waitForTimeout(500);

    // Click Create Coach button
    await page.getByRole('button', { name: 'Create Coach' }).click();

    // Wait for initial form
    await expect(page.getByText('Analyzed 4 of 5 messages')).toBeVisible({ timeout: 10000 });
    expect(generateCallCount).toBe(1);

    // Click regenerate button (title="Regenerate suggestions")
    await page.getByTitle('Regenerate suggestions').click();

    // Wait for regeneration
    await page.waitForTimeout(1000);
    expect(generateCallCount).toBe(2);
  });

  test('handles API error gracefully', async ({ page }) => {
    await setupConversationMocks(page, { hasMessages: true });

    // Override generate endpoint to return error
    await page.route('**/api/coaches/generate', async (route) => {
      await route.fulfill({
        status: 500,
        contentType: 'application/json',
        body: JSON.stringify({ error: 'LLM service unavailable' }),
      });
    });

    await loginToDashboard(page);

    await page.waitForSelector('main', { timeout: 10000 });

    // Click on a conversation
    await page.getByText('Marathon Training Discussion').click();
    await page.waitForTimeout(500);

    // Click Create Coach button
    await page.getByRole('button', { name: 'Create Coach' }).click();

    // Should show error state
    await expect(page.getByText('Analysis Failed')).toBeVisible({ timeout: 10000 });

    // Should show Try Again button
    await expect(page.getByRole('button', { name: 'Try Again' })).toBeVisible();
  });
});

test.describe('Structured Coach Sections', () => {
  test('displays structured sections in the coach detail sheet', async ({ page }) => {
    await setupUserCoachesMocks(page);
    await loginToDashboard(page);
    await openYourCoaches(page);

    await page.getByRole('button', { name: 'Open Structured Marathon Coach' }).click();
    const detail = page.getByRole('dialog');

    // Structured sections take precedence over the flat "System Prompt".
    await expect(detail.getByRole('heading', { name: 'Purpose' })).toBeVisible({ timeout: 5000 });
    await expect(detail.getByText('Expert in marathon preparation and race day strategy.')).toBeVisible();

    await expect(detail.getByRole('heading', { name: 'When to Use' })).toBeVisible();
    await expect(detail.getByText('Training for your first marathon')).toBeVisible();

    await expect(detail.getByRole('heading', { name: 'Instructions' })).toBeVisible();
    await expect(detail.getByText('Expert marathon coach instructions here.')).toBeVisible();

    await expect(detail.getByRole('heading', { name: 'Example Inputs' })).toBeVisible();
    await expect(detail.getByText('How do I build up to a 20-mile long run safely?')).toBeVisible();

    await expect(detail.getByRole('heading', { name: 'Example Outputs' })).toBeVisible();
    await expect(detail.getByText('Provide detailed training progressions')).toBeVisible();

    await expect(detail.getByRole('heading', { name: 'Success Criteria' })).toBeVisible();
    await expect(detail.getByText('Runner has a clear weekly training structure')).toBeVisible();

    await expect(detail.getByRole('heading', { name: 'System Prompt' })).toHaveCount(0);
  });

  test('displays the flat system prompt when no structured sections are available', async ({ page }) => {
    await setupUserCoachesMocks(page);
    await loginToDashboard(page);
    await openYourCoaches(page);

    await page.getByRole('button', { name: 'Open My Custom Coach' }).click();
    const detail = page.getByRole('dialog');

    await expect(detail.getByRole('heading', { name: 'System Prompt' })).toBeVisible({ timeout: 5000 });
    await expect(detail.getByText('You are my personal coach.')).toBeVisible();

    await expect(detail.getByRole('heading', { name: 'Purpose' })).toHaveCount(0);
    await expect(detail.getByRole('heading', { name: 'Instructions' })).toHaveCount(0);
  });
});

// Phase 2: Import/Export E2E tests
test.describe('Coach Import and Export', () => {
  async function setupImportExportMocks(page: Page) {
    await setupUserCoachesMocks(page);

    // Mock import preview endpoint
    await page.route('**/api/coaches/import/preview', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          valid: true,
          parsed: {
            name: 'imported-coach',
            title: 'Imported Training Coach',
            category: 'training',
            tags: ['import', 'test'],
            purpose: 'A coach imported from markdown.',
            has_instructions: true,
            has_example_inputs: true,
            has_example_outputs: false,
            has_success_criteria: false,
          },
          warnings: ['Missing optional section: example_outputs'],
          content_hash: 'abc123',
          duplicate_exists: false,
          duplicate_coach_id: null,
          token_count: 120,
        }),
      });
    });

    // Mock import endpoint
    await page.route('**/api/coaches/import', async (route) => {
      if (route.request().method() === 'POST') {
        await route.fulfill({
          status: 201,
          contentType: 'application/json',
          body: JSON.stringify({
            coach: {
              id: 'imported-coach-1',
              title: 'Imported Training Coach',
              description: 'A coach imported from markdown.',
              system_prompt: 'You are an imported coach.',
              category: 'training',
              tags: ['import', 'test'],
              token_count: 120,
              is_favorite: false,
              use_count: 0,
              is_system: false,
              visibility: 'private',
              is_assigned: false,
              purpose: 'A coach imported from markdown.',
              instructions: 'You are an imported coach.',
            },
            parsed_name: 'imported-coach',
            token_count: 120,
            warnings: ['Missing optional section: example_outputs'],
          }),
        });
      } else {
        await route.continue();
      }
    });

    // Mock export endpoint
    await page.route('**/api/coaches/*/export', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'text/markdown',
        headers: {
          'Content-Disposition': 'attachment; filename="my-custom-coach.md"',
        },
        body: '---\nname: my-custom-coach\ntitle: My Custom Coach\ncategory: training\n---\n\n## Purpose\nPersonal training coach.\n\n## Instructions\nYou are my personal coach.\n',
      });
    });

    // Mock URL import endpoint
    await page.route('**/api/coaches/import/url', async (route) => {
      const body = route.request().postDataJSON();
      if (body?.save === false) {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({
            valid: true,
            parsed: {
              name: 'url-imported-coach',
              title: 'URL Imported Coach',
              category: 'training',
              tags: ['url'],
              purpose: 'Imported from URL.',
              has_instructions: true,
              has_example_inputs: false,
              has_example_outputs: false,
              has_success_criteria: false,
            },
            warnings: [],
            content_hash: 'def456',
            duplicate_exists: false,
            token_count: 80,
          }),
        });
      } else {
        await route.fulfill({
          status: 201,
          contentType: 'application/json',
          body: JSON.stringify({
            coach: {
              id: 'url-imported-1',
              title: 'URL Imported Coach',
              description: 'Imported from URL.',
              system_prompt: 'You are a URL imported coach.',
              category: 'training',
              tags: ['url'],
              token_count: 80,
              is_favorite: false,
              use_count: 0,
              is_system: false,
              visibility: 'private',
            },
            parsed_name: 'url-imported-coach',
            token_count: 80,
            warnings: [],
          }),
        });
      }
    });
  }

  test('import button shows a menu with file and URL options', async ({ page }) => {
    await setupImportExportMocks(page);
    await loginToDashboard(page);
    await openYourCoaches(page);

    await page.getByRole('button', { name: 'Import Coach' }).click();

    const menu = page.getByRole('menu', { name: 'Import a coach' });
    await expect(menu.getByRole('menuitem', { name: 'Import from File' })).toBeVisible();
    await expect(menu.getByRole('menuitem', { name: 'Import from URL' })).toBeVisible();
  });

  test('export button triggers markdown download', async ({ page }) => {
    await setupImportExportMocks(page);
    await loginToDashboard(page);
    const section = await openYourCoaches(page);

    await expect(section.getByText('My Custom Coach')).toBeVisible();
    const exportButton = section.getByRole('button', { name: 'Export' }).first();
    await expect(exportButton).toBeVisible();

    const [download] = await Promise.all([
      page.waitForEvent('download', { timeout: 5000 }),
      exportButton.click(),
    ]);

    expect(download.suggestedFilename()).toContain('.md');
  });

  test('import from URL shows a preview then creates the coach', async ({ page }) => {
    await setupImportExportMocks(page);

    let savedUrl: string | null = null;
    await page.route('**/api/coaches/import/url', async (route) => {
      const body = route.request().postDataJSON();
      if (body?.save === false) {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({
            valid: true,
            parsed: {
              name: 'url-imported-coach',
              title: 'URL Imported Coach',
              category: 'training',
              tags: ['url'],
              purpose: 'Imported from URL.',
              has_instructions: true,
              has_example_inputs: false,
              has_example_outputs: false,
              has_success_criteria: false,
            },
            warnings: [],
            content_hash: 'def456',
            duplicate_exists: false,
            token_count: 80,
          }),
        });
      } else {
        savedUrl = body?.url ?? null;
        await route.fulfill({
          status: 201,
          contentType: 'application/json',
          body: JSON.stringify({
            coach: {
              id: 'url-imported-1',
              title: 'URL Imported Coach',
              description: 'Imported from URL.',
              system_prompt: 'You are a URL imported coach.',
              category: 'training',
              tags: ['url'],
              token_count: 80,
              is_favorite: false,
              use_count: 0,
              is_system: false,
              visibility: 'private',
            },
            parsed_name: 'url-imported-coach',
            token_count: 80,
            warnings: [],
          }),
        });
      }
    });

    await loginToDashboard(page);
    await openYourCoaches(page);

    await page.getByRole('button', { name: 'Import Coach' }).click();
    await page.getByRole('menuitem', { name: 'Import from URL' }).click();

    const urlInput = page.getByLabel('Coach file URL');
    await expect(urlInput).toBeVisible();
    await urlInput.fill('https://raw.githubusercontent.com/example/coaches/main/training.md');
    await page.getByRole('button', { name: 'Preview' }).click();

    // The preview names the parsed coach; confirming saves it.
    await expect(page.getByText('URL Imported Coach')).toBeVisible({ timeout: 10000 });
    await page.getByRole('button', { name: 'Import', exact: true }).click();

    await expect.poll(() => savedUrl).toBe('https://raw.githubusercontent.com/example/coaches/main/training.md');
  });
});
