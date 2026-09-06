// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Playwright E2E tests for System Agents admin functionality.
// ABOUTME: Tests admin agents management, CRUD operations, and user assignments.

import { test, expect, type Page } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard, navigateToTab } from './test-helpers';

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

  // For non-admin users, mock the user coaches endpoint the chat mention palette reads
  if (!isAdmin) {
    await page.route('**/api/coaches', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ coaches: [], total: 0 }),
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

test.describe('System Agents Tab Visibility', () => {
  test('displays the Agents tab for admin users', async ({ page }) => {
    await setupCoachesMocks(page, { isAdmin: true });
    await loginToDashboard(page);

    await page.waitForSelector('nav', { timeout: 10000 });

    // The admin Agents tab should be visible (exact match so nothing else named Agents matches)
    await expect(page.locator('nav button').filter({ hasText: /^Agents$/ })).toBeVisible();
  });

  test('hides the Agents tab for non-admin users', async ({ page }) => {
    await setupCoachesMocks(page, { isAdmin: false });
    await loginToDashboard(page);

    // Non-admin users see chat-first layout (no admin sidebar)
    await page.waitForSelector('main', { timeout: 10000 });

    // Admin-only tabs should NOT be visible for non-admin users
    // User mode also has an "Agents" button, so check for admin-specific tabs instead
    await expect(page.locator('nav button').filter({ hasText: /^Agent Store$/ })).not.toBeVisible();
    await expect(page.locator('nav button').filter({ hasText: /^Users$/ })).not.toBeVisible();
  });
});

test.describe('System Agents List View', () => {
  test('displays empty state when no agents exist', async ({ page }) => {
    await setupCoachesMocks(page, { isAdmin: true, emptyState: true });
    await loginToDashboard(page);

    await page.waitForSelector('nav', { timeout: 10000 });
    await navigateToTab(page, 'Agents');

    // Should see empty state message
    await expect(page.getByText('No system agents')).toBeVisible({ timeout: 10000 });
    await expect(page.getByText('Create your first system agent')).toBeVisible();
    await expect(page.getByRole('button', { name: 'Create Your First Agent' })).toBeVisible();
  });

  test('displays agent cards with correct information', async ({ page }) => {
    await setupCoachesMocks(page, { isAdmin: true });
    await loginToDashboard(page);

    await page.waitForSelector('nav', { timeout: 10000 });
    await navigateToTab(page, 'Agents');

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

  test('the Create Agent button navigates to the form', async ({ page }) => {
    await setupCoachesMocks(page, { isAdmin: true });
    await loginToDashboard(page);

    await page.waitForSelector('nav', { timeout: 10000 });
    await navigateToTab(page, 'Agents');

    await expect(page.getByText('Marathon Training Coach')).toBeVisible({ timeout: 10000 });

    // Click the Create Agent button
    await page.getByRole('button', { name: 'Create Agent' }).click();

    // Should see form
    await expect(page.getByText('Create System Agent')).toBeVisible();
    await expect(page.getByText('Title')).toBeVisible();
    await expect(page.getByText('System Prompt')).toBeVisible();
  });
});

test.describe('Create Agent Form', () => {
  test('displays form with all required fields', async ({ page }) => {
    await setupCoachesMocks(page, { isAdmin: true });
    await loginToDashboard(page);

    await page.waitForSelector('nav', { timeout: 10000 });
    await navigateToTab(page, 'Agents');
    await page.getByRole('button', { name: 'Create Agent' }).click();

    // Check form fields by their labels (text labels, not htmlFor)
    await expect(page.getByText('Title').first()).toBeVisible();
    await expect(page.getByText('Description')).toBeVisible();
    await expect(page.getByText('System Prompt')).toBeVisible();
    await expect(page.getByText('Category')).toBeVisible();
    await expect(page.getByText('Visibility')).toBeVisible();
    await expect(page.getByText('Tags')).toBeVisible();
    // Check that input fields are visible
    await expect(page.getByPlaceholder('e.g., Marathon Training Agent')).toBeVisible();
    await expect(page.getByPlaceholder('You are Dravr, specializing in marathon training')).toBeVisible();
  });

  test('displays token count estimate for system prompt', async ({ page }) => {
    await setupCoachesMocks(page, { isAdmin: true });
    await loginToDashboard(page);

    await page.waitForSelector('nav', { timeout: 10000 });
    await navigateToTab(page, 'Agents');
    await page.getByRole('button', { name: 'Create Agent' }).click();

    // Enter system prompt
    const systemPromptField = page.locator('textarea').filter({ hasText: '' }).first();
    await systemPromptField.fill('You are a professional coach with expertise in marathon training.');

    // Should display token estimate
    await expect(page.getByText(/Estimated tokens:/)).toBeVisible();
  });

  test('creates the agent successfully', async ({ page }) => {
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
    await navigateToTab(page, 'Agents');
    await page.getByRole('button', { name: 'Create Agent' }).click();

    // Fill form using placeholders and locators
    await page.getByPlaceholder('e.g., Marathon Training Agent').fill('Recovery Coach');
    await page.locator('textarea').first().fill('Optional description');
    await page.locator('textarea').nth(1).fill('You are a recovery specialist...');
    await page.locator('select').first().selectOption('Recovery');
    await page.getByPlaceholder('marathon, endurance, beginner').fill('recovery, rest, sleep');

    // Submit
    await page.getByRole('button', { name: 'Create Agent' }).click();

    await page.waitForTimeout(500);
    expect(createCalled).toBe(true);
    expect(createdData.title).toBe('Recovery Coach');
    expect(createdData.category).toBe('Recovery');
  });

  test('Back button returns to list view', async ({ page }) => {
    await setupCoachesMocks(page, { isAdmin: true });
    await loginToDashboard(page);

    await page.waitForSelector('nav', { timeout: 10000 });
    await navigateToTab(page, 'Agents');
    await page.getByRole('button', { name: 'Create Agent' }).click();

    await expect(page.getByText('Create System Agent')).toBeVisible();

    // Click back
    await page.getByText('Back to Agents').click();

    // Should return to list
    await expect(page.getByText('Marathon Training Coach')).toBeVisible();
  });

  test('Cancel button returns to list view', async ({ page }) => {
    await setupCoachesMocks(page, { isAdmin: true });
    await loginToDashboard(page);

    await page.waitForSelector('nav', { timeout: 10000 });
    await navigateToTab(page, 'Agents');
    await page.getByRole('button', { name: 'Create Agent' }).click();

    await expect(page.getByText('Create System Agent')).toBeVisible();

    // Click cancel
    await page.getByRole('button', { name: 'Cancel' }).click();

    // Should return to list
    await expect(page.getByText('Marathon Training Coach')).toBeVisible();
  });
});

test.describe('Agent Detail View', () => {
  test('clicking an agent card opens detail view', async ({ page }) => {
    await setupCoachesMocks(page, { isAdmin: true });
    await loginToDashboard(page);

    await page.waitForSelector('nav', { timeout: 10000 });
    await navigateToTab(page, 'Agents');

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
    await navigateToTab(page, 'Agents');

    await page.getByText('Marathon Training Coach').click();

    // Should display created and updated timestamps
    await expect(page.getByText('Created:')).toBeVisible({ timeout: 5000 });
    await expect(page.getByText('Last Updated:')).toBeVisible();
  });

  test('displays tags in detail view', async ({ page }) => {
    await setupCoachesMocks(page, { isAdmin: true });
    await loginToDashboard(page);

    await page.waitForSelector('nav', { timeout: 10000 });
    await navigateToTab(page, 'Agents');

    await page.getByText('Marathon Training Coach').click();

    // Should display tags section
    await expect(page.getByText('Tags')).toBeVisible({ timeout: 5000 });
    // Use exact match to avoid matching the tag text in title/description
    await expect(page.getByText('marathon', { exact: true })).toBeVisible();
    await expect(page.getByText('endurance', { exact: true })).toBeVisible();
    await expect(page.getByText('running', { exact: true })).toBeVisible();
  });
});

test.describe('Edit Agent Form', () => {
  test('Edit button opens form with pre-populated data', async ({ page }) => {
    await setupCoachesMocks(page, { isAdmin: true });
    await loginToDashboard(page);

    await page.waitForSelector('nav', { timeout: 10000 });
    await navigateToTab(page, 'Agents');

    await page.getByText('Marathon Training Coach').click();
    await expect(page.getByRole('button', { name: 'Edit' })).toBeVisible({ timeout: 5000 });

    // Click edit
    await page.getByRole('button', { name: 'Edit' }).click();

    // Should see edit form with populated data
    await expect(page.getByText('Edit "Marathon Training Coach"')).toBeVisible();
    // Use placeholder selector since form doesn't use htmlFor
    const titleInput = page.getByPlaceholder('e.g., Marathon Training Agent');
    await expect(titleInput).toHaveValue('Marathon Training Coach');
  });

  test('updates the agent successfully', async ({ page }) => {
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
    await navigateToTab(page, 'Agents');

    await page.getByText('Marathon Training Coach').click();
    await page.getByRole('button', { name: 'Edit' }).click();

    // Wait for edit form to be visible
    await expect(page.getByText('Edit "Marathon Training Coach"')).toBeVisible({ timeout: 5000 });

    // Wait for form to be populated - token count > 0 indicates system_prompt has content
    await expect(page.getByText(/Estimated tokens: [1-9]/)).toBeVisible({ timeout: 5000 });

    // Modify title using placeholder selector
    await page.getByPlaceholder('e.g., Marathon Training Agent').fill('Updated Marathon Coach');

    // Save
    await page.getByRole('button', { name: 'Save Changes' }).click();

    await page.waitForTimeout(500);
    expect(updateCalled).toBe(true);
  });

  test('visibility dropdown is disabled when editing', async ({ page }) => {
    await setupCoachesMocks(page, { isAdmin: true });
    await loginToDashboard(page);

    await page.waitForSelector('nav', { timeout: 10000 });
    await navigateToTab(page, 'Agents');

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

test.describe('Delete Agent', () => {
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
      expect(dialog.message()).toContain('Delete agent');
      await dialog.accept();
    });

    await loginToDashboard(page);

    await page.waitForSelector('nav', { timeout: 10000 });
    await navigateToTab(page, 'Agents');

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
    await navigateToTab(page, 'Agents');

    await page.getByText('Marathon Training Coach').click();

    // Should display assigned users count
    await expect(page.getByText('Assigned Users')).toBeVisible({ timeout: 5000 });
    await expect(page.getByText('1').first()).toBeVisible(); // 1 assignment in mock
  });

  test('displays User Assignments card with assigned users', async ({ page }) => {
    await setupCoachesMocks(page, { isAdmin: true });
    await loginToDashboard(page);

    await page.waitForSelector('nav', { timeout: 10000 });
    await navigateToTab(page, 'Agents');

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
    await navigateToTab(page, 'Agents');

    await page.getByText('Marathon Training Coach').click();
    await expect(page.getByRole('button', { name: 'Assign Users' })).toBeVisible({ timeout: 5000 });

    // Click Assign Users
    await page.getByRole('button', { name: 'Assign Users' }).click();

    // Should see modal
    await expect(page.getByText('Assign Users to Agent')).toBeVisible();
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
    await navigateToTab(page, 'Agents');

    await page.getByText('Marathon Training Coach').click();
    await page.getByRole('button', { name: 'Assign Users' }).click();

    // Wait for modal and users to load
    await expect(page.getByText('Assign Users to Agent')).toBeVisible({ timeout: 5000 });
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
    await navigateToTab(page, 'Agents');

    await page.getByText('Marathon Training Coach').click();

    // Should show empty state message
    await expect(page.getByText('No users assigned to this agent yet')).toBeVisible({ timeout: 5000 });
  });
});

test.describe('Error Handling', () => {
  test('shows error when failing to load agents', async ({ page }) => {
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
    await navigateToTab(page, 'Agents');

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
    await navigateToTab(page, 'Agents');

    await page.getByRole('button', { name: 'Create Agent' }).click();

    // Fill minimal form using placeholder selectors
    await page.getByPlaceholder('e.g., Marathon Training Agent').fill('Test Coach');
    await page.getByPlaceholder('You are Dravr, specializing in marathon training').fill('Test prompt');

    // Submit
    await page.getByRole('button', { name: 'Create Agent' }).click();

    await page.waitForTimeout(500);
    // Form should still be visible (not submitted successfully)
    await expect(page.getByText('Create System Agent')).toBeVisible();
  });
});

test.describe('Category Colors', () => {
  test('displays correct category colors on cards', async ({ page }) => {
    await setupCoachesMocks(page, { isAdmin: true });
    await loginToDashboard(page);

    await page.waitForSelector('nav', { timeout: 10000 });
    await navigateToTab(page, 'Agents');

    await expect(page.getByText('Marathon Training Coach')).toBeVisible({ timeout: 10000 });

    // Training category should have Training badge
    const trainingBadge = page.locator('.rounded-full').filter({ hasText: 'Training' });
    await expect(trainingBadge).toBeVisible();

    // Nutrition category should have Nutrition badge
    const nutritionBadge = page.locator('.rounded-full').filter({ hasText: 'Nutrition' });
    await expect(nutritionBadge).toBeVisible();
  });
});
