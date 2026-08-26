// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Playwright E2E tests for Coach Store functionality.
// ABOUTME: Tests browsing, searching, filtering, viewing details, and install/uninstall actions.

import { test, expect, type Page } from '@playwright/test';
import { setupDashboardMocks, loginToDashboard } from './test-helpers';

// Mock store coach data
const mockStoreCoaches = [
  {
    id: 'store-coach-1',
    title: 'Marathon Training Coach',
    description: 'A comprehensive marathon training program with weekly schedules',
    category: 'training',
    tags: ['marathon', 'running', 'endurance'],
    sample_prompts: ['What should my weekly mileage be?', 'How do I taper for race day?'],
    token_count: 1200,
    install_count: 75,
    icon_url: null,
    published_at: '2024-01-15T00:00:00Z',
    author_id: 'author-123',
    handle: 'marathon-training-coach',
  },
  {
    id: 'store-coach-2',
    title: 'Nutrition Expert',
    description: 'Personalized nutrition advice and meal planning',
    category: 'nutrition',
    tags: ['diet', 'macros', 'meal-planning'],
    sample_prompts: ['How many calories should I eat?', 'What should I eat before a race?'],
    token_count: 800,
    install_count: 120,
    icon_url: null,
    published_at: '2024-01-20T00:00:00Z',
    author_id: 'author-456',
  },
  {
    id: 'store-coach-3',
    title: 'Recovery Coach',
    description: 'Optimize your recovery and prevent injuries',
    category: 'recovery',
    tags: ['sleep', 'stretching', 'rest'],
    sample_prompts: ['How long should I sleep?', 'What stretches help after running?'],
    token_count: 600,
    install_count: 45,
    icon_url: null,
    published_at: '2024-01-25T00:00:00Z',
    author_id: 'author-789',
  },
];

// Mock coach detail with system_prompt
const mockCoachDetail = {
  ...mockStoreCoaches[0],
  system_prompt:
    'You are an expert marathon training coach. Help athletes prepare for marathon races with personalized training plans based on their current fitness level and goals.',
  created_at: '2024-01-10T00:00:00Z',
  publish_status: 'published',
};

// Installing a store coach mints a personal copy: a fresh id with `forked_from`
// pointing back at the store listing. Both GET /api/coaches and
// GET /api/store/installations return those copies, never the listing itself.
function personalCopyOf(storeCoach: (typeof mockStoreCoaches)[number]) {
  return {
    id: `installed-copy-of-${storeCoach.id}`,
    title: storeCoach.title,
    description: storeCoach.description,
    system_prompt: mockCoachDetail.system_prompt,
    category: storeCoach.category,
    tags: storeCoach.tags,
    sample_prompts: storeCoach.sample_prompts,
    token_count: storeCoach.token_count,
    is_favorite: false,
    use_count: 0,
    last_used_at: null,
    created_at: '2024-02-01T00:00:00Z',
    updated_at: '2024-02-01T00:00:00Z',
    is_system: false,
    visibility: 'private',
    is_assigned: true,
    forked_from: storeCoach.id,
    // The copy answers to its origin's catalogue handle.
    handle: storeCoach.handle,
  };
}

async function setupStoreMocks(
  page: Page,
  options: { emptyStore?: boolean; installed?: string[]; failStore?: boolean } = {}
) {
  const { emptyStore = false, installed = [], failStore = false } = options;
  const installedCopies = mockStoreCoaches
    .filter((c) => installed.includes(c.id))
    .map(personalCopyOf);

  // Set up base dashboard mocks for regular user
  await setupDashboardMocks(page, { role: 'user' });

  // Mock user coaches endpoint (required for sidebar, and the source of the
  // `forked_from` mapping the store uses to recognize an installed coach).
  // The regex also matches the `?include_hidden=true&personalize=true` variant
  // while leaving sub-paths like /api/coaches/hidden to the dashboard mocks.
  await page.route(/\/api\/coaches(\?.*)?$/, async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        coaches: installedCopies,
        total: installedCopies.length,
        metadata: { timestamp: new Date().toISOString(), api_version: '1.0' },
      }),
    });
  });

  // Mock store installations endpoint (must come before /api/store/coaches/*)
  await page.route('**/api/store/installations', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        coaches: installedCopies,
        metadata: { timestamp: new Date().toISOString(), api_version: '1.0' },
      }),
    });
  });

  // Mock store categories endpoint
  await page.route('**/api/store/categories', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        categories: ['training', 'nutrition', 'recovery', 'recipes', 'mobility', 'custom'],
        metadata: { timestamp: new Date().toISOString(), api_version: '1.0' },
      }),
    });
  });

  // Mock store search endpoint
  await page.route('**/api/store/search**', async (route) => {
    const url = new URL(route.request().url());
    const query = url.searchParams.get('q') || '';

    const coaches = emptyStore
      ? []
      : mockStoreCoaches.filter(
          (c) =>
            c.title.toLowerCase().includes(query.toLowerCase()) ||
            c.description.toLowerCase().includes(query.toLowerCase()) ||
            c.tags.some((t) => t.toLowerCase().includes(query.toLowerCase()))
        );

    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        coaches,
        query,
        metadata: { timestamp: new Date().toISOString(), api_version: '1.0' },
      }),
    });
  });

  // Mock individual coach detail and install/uninstall endpoints
  // This pattern matches /api/store/coaches/{id} and /api/store/coaches/{id}/install
  await page.route('**/api/store/coaches/*/**', async (route) => {
    const url = route.request().url();
    const pathId = url.split('/api/store/coaches/')[1]?.split('/')[0] ?? '';

    // Install takes the store listing id and rejects a second install
    if (url.includes('/install') && route.request().method() === 'POST') {
      if (installed.includes(pathId)) {
        await route.fulfill({
          status: 400,
          contentType: 'application/json',
          body: JSON.stringify({
            code: 'InvalidInput',
            message: `Coach ${pathId} is already installed`,
          }),
        });
        return;
      }
      const source = mockStoreCoaches.find((c) => c.id === pathId);
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          message: 'Coach installed successfully',
          coach: source ? personalCopyOf(source) : null,
          metadata: { timestamp: new Date().toISOString(), api_version: '1.0' },
        }),
      });
      return;
    }

    // Uninstall takes the personal copy id — the server 404s on a listing id
    if (url.includes('/install') && route.request().method() === 'DELETE') {
      const copy = installedCopies.find((c) => c.id === pathId);
      if (!copy) {
        await route.fulfill({
          status: 404,
          contentType: 'application/json',
          body: JSON.stringify({
            code: 'ResourceNotFound',
            message: 'The requested resource was not found',
          }),
        });
        return;
      }
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          message: 'Coach uninstalled successfully',
          source_coach_id: copy.forked_from,
          metadata: { timestamp: new Date().toISOString(), api_version: '1.0' },
        }),
      });
      return;
    }

    await route.continue();
  });

  // Mock individual coach GET endpoint (must be separate for single segment match)
  await page.route(/\/api\/store\/coaches\/[^/]+$/, async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(mockCoachDetail),
    });
  });

  // Mock store browse endpoint (handles query params)
  await page.route(/\/api\/store\/coaches(\?.*)?$/, async (route) => {
    if (failStore) {
      await route.fulfill({
        status: 500,
        contentType: 'application/json',
        body: JSON.stringify({
          code: 'InternalError',
          message: 'Store listing query failed',
        }),
      });
      return;
    }

    const url = new URL(route.request().url());
    const category = url.searchParams.get('category');
    const sortBy = url.searchParams.get('sort_by');

    let coaches = emptyStore ? [] : [...mockStoreCoaches];

    // Apply category filter
    if (category && category !== 'all') {
      coaches = coaches.filter((c) => c.category === category);
    }

    // Apply sort
    if (sortBy === 'newest') {
      coaches.sort((a, b) => new Date(b.published_at).getTime() - new Date(a.published_at).getTime());
    } else if (sortBy === 'title') {
      coaches.sort((a, b) => a.title.localeCompare(b.title));
    } else {
      // Default: popular (by install_count)
      coaches.sort((a, b) => b.install_count - a.install_count);
    }

    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        coaches,
        next_cursor: null,
        has_more: false,
        metadata: { timestamp: new Date().toISOString(), api_version: '1.0' },
      }),
    });
  });
}

test.describe('Coach Store Access', () => {
  test('displays Discover button in sidebar', async ({ page }) => {
    await setupStoreMocks(page);
    await loginToDashboard(page);

    await page.waitForSelector('main', { timeout: 10000 });

    // Should see Discover button in sidebar
    await expect(page.getByRole('button', { name: 'Discover', exact: true })).toBeVisible({ timeout: 5000 });
  });

  test('opens Coach Store when button is clicked', async ({ page }) => {
    await setupStoreMocks(page);
    await loginToDashboard(page);

    await page.waitForSelector('main', { timeout: 10000 });

    // Click Discover button
    await page.getByRole('button', { name: 'Discover', exact: true }).click();

    // Should see store header (use heading role to avoid ambiguity with button)
    await expect(page.getByText('Find AI coaching assistants')).toBeVisible({ timeout: 5000 });
    await expect(page.getByText('Find AI coaching assistants')).toBeVisible();
  });
});

test.describe('Coach Store Browse', () => {
  test('displays coaches in store', async ({ page }) => {
    await setupStoreMocks(page);
    await loginToDashboard(page);

    await page.waitForSelector('main', { timeout: 10000 });
    await page.getByRole('button', { name: 'Discover', exact: true }).click();

    // Wait for store to load
    await expect(page.getByText('Find AI coaching assistants')).toBeVisible({ timeout: 5000 });

    // Should display coach cards
    await expect(page.getByText('Marathon Training Coach')).toBeVisible({ timeout: 10000 });
    await expect(page.getByText('Nutrition Expert')).toBeVisible();
    await expect(page.getByText('Recovery Coach')).toBeVisible();
  });

  test('displays coach user counts', async ({ page }) => {
    await setupStoreMocks(page);
    await loginToDashboard(page);

    await page.waitForSelector('main', { timeout: 10000 });
    await page.getByRole('button', { name: 'Discover', exact: true }).click();

    await expect(page.getByText('Find AI coaching assistants')).toBeVisible({ timeout: 5000 });
    await expect(page.getByText('Marathon Training Coach')).toBeVisible({ timeout: 10000 });

    // Should display user counts
    await expect(page.getByText('75 users')).toBeVisible();
    await expect(page.getByText('120 users')).toBeVisible();
    await expect(page.getByText('45 users')).toBeVisible();
  });

  test('displays coach category badges', async ({ page }) => {
    await setupStoreMocks(page);
    await loginToDashboard(page);

    await page.waitForSelector('main', { timeout: 10000 });
    await page.getByRole('button', { name: 'Discover', exact: true }).click();

    await expect(page.getByText('Find AI coaching assistants')).toBeVisible({ timeout: 5000 });
    await expect(page.getByText('Marathon Training Coach')).toBeVisible({ timeout: 10000 });

    // Should display category badges
    await expect(page.getByText('training').first()).toBeVisible();
    await expect(page.getByText('nutrition').first()).toBeVisible();
    await expect(page.getByText('recovery').first()).toBeVisible();
  });

  test('displays coach tags', async ({ page }) => {
    await setupStoreMocks(page);
    await loginToDashboard(page);

    await page.waitForSelector('main', { timeout: 10000 });
    await page.getByRole('button', { name: 'Discover', exact: true }).click();

    await expect(page.getByText('Find AI coaching assistants')).toBeVisible({ timeout: 5000 });
    await expect(page.getByText('Marathon Training Coach')).toBeVisible({ timeout: 10000 });

    // Should display tags
    await expect(page.getByText('marathon', { exact: true })).toBeVisible();
    await expect(page.getByText('running', { exact: true })).toBeVisible();
    await expect(page.getByText('diet', { exact: true })).toBeVisible();
  });

  test('displays end-of-list indicator when all coaches are shown', async ({ page }) => {
    await setupStoreMocks(page);
    await loginToDashboard(page);

    await page.waitForSelector('main', { timeout: 10000 });
    await page.getByRole('button', { name: 'Discover', exact: true }).click();

    await expect(page.getByText('Find AI coaching assistants')).toBeVisible({ timeout: 5000 });
    await expect(page.getByText('Marathon Training Coach')).toBeVisible({ timeout: 10000 });

    // Mock returns has_more: false, so end-of-list indicator should appear
    await expect(page.getByText("You've seen all coaches")).toBeVisible({ timeout: 5000 });
  });

  test('shows empty state when store is empty', async ({ page }) => {
    await setupStoreMocks(page, { emptyStore: true });
    await loginToDashboard(page);

    await page.waitForSelector('main', { timeout: 10000 });
    await page.getByRole('button', { name: 'Discover', exact: true }).click();

    // Should show empty state
    await expect(page.getByText('Store is empty')).toBeVisible({ timeout: 10000 });
  });
});

test.describe('Coach Store Pagination', () => {
  test('loads more coaches on scroll with cursor pagination', async ({ page }) => {
    // Mock coaches for pagination - first page and second page
    const page1Coaches = mockStoreCoaches.slice(0, 2);
    const page2Coach = mockStoreCoaches[2];

    // Set up base dashboard mocks
    await setupDashboardMocks(page, { role: 'user' });

    // Mock user coaches endpoint
    await page.route('**/api/coaches', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ coaches: [], total: 0 }),
      });
    });

    // Mock store installations endpoint
    await page.route('**/api/store/installations', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          coaches: [],
          metadata: { timestamp: new Date().toISOString(), api_version: '1.0' },
        }),
      });
    });

    // Mock store categories endpoint
    await page.route('**/api/store/categories', async (route) => {
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          categories: ['training', 'nutrition', 'recovery'],
          metadata: { timestamp: new Date().toISOString(), api_version: '1.0' },
        }),
      });
    });

    // Track requests to verify cursor is sent
    let requestCount = 0;

    // Mock store browse endpoint with cursor pagination
    await page.route(/\/api\/store\/coaches(\?.*)?$/, async (route) => {
      const url = new URL(route.request().url());
      const cursor = url.searchParams.get('cursor');
      requestCount++;

      if (!cursor) {
        // First page - return first 2 coaches with cursor
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({
            coaches: page1Coaches,
            next_cursor: 'test-cursor-page-2',
            has_more: true,
            metadata: { timestamp: new Date().toISOString(), api_version: '1.0' },
          }),
        });
      } else {
        // Second page - return last coach with no more
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({
            coaches: [page2Coach],
            next_cursor: null,
            has_more: false,
            metadata: { timestamp: new Date().toISOString(), api_version: '1.0' },
          }),
        });
      }
    });

    await loginToDashboard(page);

    await page.waitForSelector('main', { timeout: 10000 });
    await page.getByRole('button', { name: 'Discover', exact: true }).click();

    await expect(page.getByText('Find AI coaching assistants')).toBeVisible({ timeout: 5000 });

    // Should display first page coaches
    await expect(page.getByText('Marathon Training Coach')).toBeVisible({ timeout: 10000 });
    await expect(page.getByText('Nutrition Expert')).toBeVisible();

    // Bring the sentinel into view to trigger infinite scroll. Discover scrolls
    // inside its own pane — the pinned coaches, filters and grid share one
    // scroll container — so the window itself never moves.
    await page.getByText('Scroll for more').scrollIntoViewIfNeeded();

    // Wait for second page to load
    await page.waitForTimeout(1000);

    // Should now also display the third coach from page 2
    await expect(page.getByText('Recovery Coach')).toBeVisible({ timeout: 5000 });

    // Verify multiple requests were made (initial + pagination)
    expect(requestCount).toBeGreaterThanOrEqual(2);
  });
});

test.describe('Coach Store Filtering', () => {
  test('displays category filter buttons', async ({ page }) => {
    await setupStoreMocks(page);
    await loginToDashboard(page);

    await page.waitForSelector('main', { timeout: 10000 });
    await page.getByRole('button', { name: 'Discover', exact: true }).click();

    await expect(page.getByText('Find AI coaching assistants')).toBeVisible({ timeout: 5000 });

    // Should display category filters (use exact: true to avoid matching coach cards)
    await expect(page.getByRole('button', { name: 'All', exact: true })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Training', exact: true })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Nutrition', exact: true })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Recovery', exact: true })).toBeVisible();
  });

  test('filters by category when clicked', async ({ page }) => {
    await setupStoreMocks(page);
    await loginToDashboard(page);

    await page.waitForSelector('main', { timeout: 10000 });
    await page.getByRole('button', { name: 'Discover', exact: true }).click();

    await expect(page.getByText('Find AI coaching assistants')).toBeVisible({ timeout: 5000 });
    await expect(page.getByText('Marathon Training Coach')).toBeVisible({ timeout: 10000 });

    // Click Training filter (use exact: true to avoid matching coach cards)
    await page.getByRole('button', { name: 'Training', exact: true }).click();

    await page.waitForTimeout(500);

    // Should show only training coach
    await expect(page.getByText('Marathon Training Coach')).toBeVisible();
  });

  test('displays sort options', async ({ page }) => {
    await setupStoreMocks(page);
    await loginToDashboard(page);

    await page.waitForSelector('main', { timeout: 10000 });
    await page.getByRole('button', { name: 'Discover', exact: true }).click();

    await expect(page.getByText('Find AI coaching assistants')).toBeVisible({ timeout: 5000 });

    // Should display sort options
    await expect(page.getByRole('button', { name: 'Popular' })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Newest' })).toBeVisible();
    await expect(page.getByRole('button', { name: 'A-Z' })).toBeVisible();
  });

  test('changes sort when option is clicked', async ({ page }) => {
    await setupStoreMocks(page);
    await loginToDashboard(page);

    await page.waitForSelector('main', { timeout: 10000 });
    await page.getByRole('button', { name: 'Discover', exact: true }).click();

    await expect(page.getByText('Find AI coaching assistants')).toBeVisible({ timeout: 5000 });
    await expect(page.getByText('Marathon Training Coach')).toBeVisible({ timeout: 10000 });

    // Click Newest sort
    await page.getByRole('button', { name: 'Newest' }).click();

    await page.waitForTimeout(500);

    // Coaches should be displayed (sorted by newest, but we just verify they're still visible)
    await expect(page.getByText('Recovery Coach')).toBeVisible();
  });
});

test.describe('Coach Store Search', () => {
  test('displays search input', async ({ page }) => {
    await setupStoreMocks(page);
    await loginToDashboard(page);

    await page.waitForSelector('main', { timeout: 10000 });
    await page.getByRole('button', { name: 'Discover', exact: true }).click();

    await expect(page.getByPlaceholder('Search coaches...')).toBeVisible({ timeout: 10000 });
  });

  test('searches coaches when text is entered', async ({ page }) => {
    await setupStoreMocks(page);
    await loginToDashboard(page);

    await page.waitForSelector('main', { timeout: 10000 });
    await page.getByRole('button', { name: 'Discover', exact: true }).click();

    await expect(page.getByText('Find AI coaching assistants')).toBeVisible({ timeout: 5000 });
    await expect(page.getByText('Marathon Training Coach')).toBeVisible({ timeout: 10000 });

    // Type in search
    await page.getByPlaceholder('Search coaches...').fill('marathon');

    // Wait for debounced search
    await page.waitForTimeout(500);

    // Should show marathon coach
    await expect(page.getByText('Marathon Training Coach')).toBeVisible();
  });

  test('shows no results message for non-matching search', async ({ page }) => {
    await setupStoreMocks(page);
    await loginToDashboard(page);

    await page.waitForSelector('main', { timeout: 10000 });
    await page.getByRole('button', { name: 'Discover', exact: true }).click();

    await expect(page.getByText('Find AI coaching assistants')).toBeVisible({ timeout: 5000 });
    await expect(page.getByText('Marathon Training Coach')).toBeVisible({ timeout: 10000 });

    // Type in search that won't match
    await page.getByPlaceholder('Search coaches...').fill('nonexistent');

    // Wait for debounced search
    await page.waitForTimeout(500);

    // Should show no results message
    await expect(page.getByText('No coaches found')).toBeVisible({ timeout: 5000 });
  });
});

test.describe('Coach Store Detail View', () => {
  test('opens coach detail when card is clicked', async ({ page }) => {
    await setupStoreMocks(page);
    await loginToDashboard(page);
    await page.waitForSelector('main', { timeout: 10000 });
    await page.getByRole('button', { name: 'Discover', exact: true }).click();
    await expect(page.getByText('Find AI coaching assistants')).toBeVisible({ timeout: 5000 });
    await expect(page.getByText('Marathon Training Coach')).toBeVisible({ timeout: 10000 });

    // Click on coach card to open detail view
    await page.getByText('Marathon Training Coach').click();

    // Should show Add Coach button in detail view
    await expect(page.getByRole('button', { name: 'Add Coach' })).toBeVisible({ timeout: 5000 });
  });

  test('displays coach details', async ({ page }) => {
    await setupStoreMocks(page);
    await loginToDashboard(page);
    await page.waitForSelector('main', { timeout: 10000 });
    await page.getByRole('button', { name: 'Discover', exact: true }).click();
    await expect(page.getByText('Marathon Training Coach')).toBeVisible({ timeout: 10000 });

    // Click on coach to see detail
    await page.getByText('Marathon Training Coach').click();
    await expect(page.getByRole('button', { name: 'Add Coach' })).toBeVisible({ timeout: 5000 });

    // Should display coach details
    await expect(page.getByText('Sample Prompts')).toBeVisible();
    await expect(page.getByText('System Prompt')).toBeVisible();
    await expect(page.getByText('Details')).toBeVisible();
    await expect(page.getByText('Token Count')).toBeVisible();
  });

  test('displays system prompt preview text', async ({ page }) => {
    await setupStoreMocks(page);
    await loginToDashboard(page);
    await page.waitForSelector('main', { timeout: 10000 });
    await page.getByRole('button', { name: 'Discover', exact: true }).click();
    await expect(page.getByText('Marathon Training Coach')).toBeVisible({ timeout: 10000 });

    // Click on coach to see detail
    await page.getByText('Marathon Training Coach').click();
    await expect(page.getByRole('button', { name: 'Add Coach' })).toBeVisible({ timeout: 5000 });

    // Should display the system prompt text preview content
    await expect(page.getByText('System Prompt')).toBeVisible();
    await expect(page.getByText(/expert marathon training coach/i)).toBeVisible();
  });

  test('displays token count value in details section', async ({ page }) => {
    await setupStoreMocks(page);
    await loginToDashboard(page);
    await page.waitForSelector('main', { timeout: 10000 });
    await page.getByRole('button', { name: 'Discover', exact: true }).click();
    await expect(page.getByText('Marathon Training Coach')).toBeVisible({ timeout: 10000 });

    // Click on coach to see detail
    await page.getByText('Marathon Training Coach').click();
    await expect(page.getByRole('button', { name: 'Add Coach' })).toBeVisible({ timeout: 5000 });

    // Should display token count label and value
    await expect(page.getByText('Token Count')).toBeVisible();
    await expect(page.getByText('1,200')).toBeVisible();
  });

  test('back button returns to store browse', async ({ page }) => {
    await setupStoreMocks(page);
    await loginToDashboard(page);
    await page.waitForSelector('main', { timeout: 10000 });
    await page.getByRole('button', { name: 'Discover', exact: true }).click();
    // An installed coach is pinned above the store too; open the listing.
    const listing = page.getByTestId('store-coach-grid').getByText('Marathon Training Coach');
    await expect(listing).toBeVisible({ timeout: 10000 });
    await listing.click();
    await expect(page.getByRole('button', { name: 'Add Coach' })).toBeVisible({ timeout: 5000 });

    // Click back button
    await page.getByRole('button', { name: 'Back to Store' }).click();

    // Should return to store browse view - use h2 heading specifically to avoid matching both h1 and h2
    await expect(page.getByText('Find AI coaching assistants')).toBeVisible({ timeout: 5000 });
    await expect(page.getByText('Marathon Training Coach')).toBeVisible({ timeout: 5000 });
  });
});

test.describe('Coach Store Add/Remove', () => {
  test('shows Add button for coach not in library', async ({ page }) => {
    await setupStoreMocks(page, { installed: [] });
    await loginToDashboard(page);
    await page.waitForSelector('main', { timeout: 10000 });
    await page.getByRole('button', { name: 'Discover', exact: true }).click();
    // An installed coach is pinned above the store too; open the listing.
    const listing = page.getByTestId('store-coach-grid').getByText('Marathon Training Coach');
    await expect(listing).toBeVisible({ timeout: 10000 });
    await listing.click();

    // Should show Add Coach button
    await expect(page.getByRole('button', { name: 'Add Coach' })).toBeVisible({ timeout: 5000 });
  });

  test('adds coach when Add button is clicked', async ({ page }) => {
    await setupStoreMocks(page, { installed: [] });
    await loginToDashboard(page);
    await page.waitForSelector('main', { timeout: 10000 });
    await page.getByRole('button', { name: 'Discover', exact: true }).click();
    await expect(page.getByText('Marathon Training Coach')).toBeVisible({ timeout: 10000 });

    // Navigate to detail view and click Add
    await page.getByText('Marathon Training Coach').click();
    await expect(page.getByRole('button', { name: 'Add Coach' })).toBeVisible({ timeout: 5000 });
    await page.getByRole('button', { name: 'Add Coach' }).click();

    // Should show success message
    await expect(page.getByText(/has been added to your coaches/)).toBeVisible({ timeout: 5000 });
  });

  test('shows success message after install', async ({ page }) => {
    await setupStoreMocks(page, { installed: [] });
    await loginToDashboard(page);
    await page.waitForSelector('main', { timeout: 10000 });
    await page.getByRole('button', { name: 'Discover', exact: true }).click();
    await expect(page.getByText('Marathon Training Coach')).toBeVisible({ timeout: 10000 });

    // Navigate to detail view and install
    await page.getByText('Marathon Training Coach').click();
    await page.getByRole('button', { name: 'Add Coach' }).click();

    // Should show success message
    await expect(page.getByText(/has been added to your coaches/)).toBeVisible({ timeout: 5000 });
  });

  test('shows Remove button for coach in library', async ({ page }) => {
    // Mock with store-coach-1 already installed
    await setupStoreMocks(page, { installed: ['store-coach-1'] });
    await loginToDashboard(page);
    await page.waitForSelector('main', { timeout: 10000 });
    await page.getByRole('button', { name: 'Discover', exact: true }).click();
    // An installed coach is pinned above the store too; open the listing.
    const listing = page.getByTestId('store-coach-grid').getByText('Marathon Training Coach');
    await expect(listing).toBeVisible({ timeout: 10000 });
    await listing.click();

    // Wait for detail view to fully load (indicated by System Prompt section)
    await expect(page.getByText('System Prompt')).toBeVisible({ timeout: 5000 });

    // Should show Remove button since coach is installed (longer timeout for queries to complete)
    await expect(page.getByRole('button', { name: 'Remove' })).toBeVisible({ timeout: 10000 });
  });

  test('shows confirmation dialog when Remove is clicked', async ({ page }) => {
    await setupStoreMocks(page, { installed: ['store-coach-1'] });
    await loginToDashboard(page);
    await page.waitForSelector('main', { timeout: 10000 });
    await page.getByRole('button', { name: 'Discover', exact: true }).click();
    // An installed coach is pinned above the store too; open the listing.
    const listing = page.getByTestId('store-coach-grid').getByText('Marathon Training Coach');
    await expect(listing).toBeVisible({ timeout: 10000 });
    await listing.click();

    // Wait for detail view to fully load
    await expect(page.getByText('System Prompt')).toBeVisible({ timeout: 5000 });
    await expect(page.getByRole('button', { name: 'Remove' })).toBeVisible({ timeout: 10000 });

    // Set up dialog listener before clicking
    let dialogMessage = '';
    page.on('dialog', async (dialog) => {
      dialogMessage = dialog.message();
      await dialog.dismiss(); // Dismiss for this test
    });

    // Click Remove
    await page.getByRole('button', { name: 'Remove' }).click();

    // Should have shown confirmation dialog
    await page.waitForTimeout(500);
    expect(dialogMessage).toContain('Remove Coach');
  });

  test('removes coach when confirmed', async ({ page }) => {
    await setupStoreMocks(page, { installed: ['store-coach-1'] });
    await loginToDashboard(page);
    await page.waitForSelector('main', { timeout: 10000 });
    await page.getByRole('button', { name: 'Discover', exact: true }).click();
    // An installed coach is pinned above the store too; open the listing.
    const listing = page.getByTestId('store-coach-grid').getByText('Marathon Training Coach');
    await expect(listing).toBeVisible({ timeout: 10000 });
    await listing.click();

    // Wait for detail view to fully load
    await expect(page.getByText('System Prompt')).toBeVisible({ timeout: 5000 });
    await expect(page.getByRole('button', { name: 'Remove' })).toBeVisible({ timeout: 10000 });

    // Accept dialog when shown
    page.on('dialog', async (dialog) => {
      await dialog.accept();
    });

    // Click Remove
    await page.getByRole('button', { name: 'Remove' }).click();

    // Should show success message
    await expect(page.getByText(/has been removed from your library/)).toBeVisible({ timeout: 5000 });
  });

  test('uninstall addresses the personal copy id, not the store listing id', async ({ page }) => {
    await setupStoreMocks(page, { installed: ['store-coach-1'] });
    await loginToDashboard(page);
    await page.waitForSelector('main', { timeout: 10000 });
    await page.getByRole('button', { name: 'Discover', exact: true }).click();
    const listing = page.getByTestId('store-coach-grid').getByText('Marathon Training Coach');
    await expect(listing).toBeVisible({ timeout: 10000 });

    await listing.click();
    await expect(page.getByText('System Prompt')).toBeVisible({ timeout: 5000 });
    await expect(page.getByRole('button', { name: 'Remove' })).toBeVisible({ timeout: 10000 });

    const deleteUrls: string[] = [];
    page.on('request', (request) => {
      if (request.method() === 'DELETE' && request.url().includes('/install')) {
        deleteUrls.push(request.url());
      }
    });
    page.on('dialog', async (dialog) => {
      await dialog.accept();
    });

    await page.getByRole('button', { name: 'Remove' }).click();
    await expect(page.getByText(/has been removed from your library/)).toBeVisible({ timeout: 5000 });

    expect(deleteUrls).toHaveLength(1);
    expect(deleteUrls[0]).toContain('/api/store/coaches/installed-copy-of-store-coach-1/install');
  });
});

test.describe('Coach Store Failures', () => {
  test('shows a retry error state when the store request fails', async ({ page }) => {
    await setupStoreMocks(page, { failStore: true });
    await loginToDashboard(page);
    await page.waitForSelector('main', { timeout: 10000 });
    await page.getByRole('button', { name: 'Discover', exact: true }).click();

    // The app's QueryClient keeps React Query's default 3 retries with
    // exponential backoff, so the terminal error state lands ~7s after the
    // first failure
    await expect(page.getByText("Couldn't load the store")).toBeVisible({ timeout: 30000 });

    // A failed request must not claim the store has no published coaches
    await expect(page.getByText('Store is empty')).toHaveCount(0);
    await expect(page.getByText('No published coaches available yet')).toHaveCount(0);
    await expect(page.getByRole('button', { name: 'Try Again' })).toBeVisible();
  });

  test('surfaces a failed install instead of showing nothing', async ({ page }) => {
    await setupStoreMocks(page, { installed: [] });
    // Registered after setupStoreMocks so it wins over the success handler
    await page.route('**/api/store/coaches/store-coach-1/install', async (route) => {
      if (route.request().method() === 'POST') {
        await route.fulfill({
          status: 400,
          contentType: 'application/json',
          body: JSON.stringify({
            code: 'InvalidInput',
            message: 'Coach Marathon Training Coach is already installed',
          }),
        });
        return;
      }
      await route.continue();
    });

    await loginToDashboard(page);
    await page.waitForSelector('main', { timeout: 10000 });
    await page.getByRole('button', { name: 'Discover', exact: true }).click();
    await expect(page.getByText('Marathon Training Coach')).toBeVisible({ timeout: 10000 });

    await page.getByText('Marathon Training Coach').click();
    await expect(page.getByRole('button', { name: 'Add Coach' })).toBeVisible({ timeout: 5000 });
    await page.getByRole('button', { name: 'Add Coach' }).click();

    // The banner carries the rejected request's message (axios reports the
    // status), so the failure is visible rather than a spinner that stops
    await expect(page.getByText(/status code 400/)).toBeVisible({ timeout: 5000 });
    await expect(page.getByText(/has been added to your coaches/)).toHaveCount(0);
    await expect(page.getByRole('button', { name: 'Add Coach' })).toBeVisible();
  });
});

test.describe('Coach Store Navigation', () => {
  test('success message appears after adding coach', async ({ page }) => {
    await setupStoreMocks(page, { installed: [] });
    await loginToDashboard(page);
    await page.waitForSelector('main', { timeout: 10000 });
    await page.getByRole('button', { name: 'Discover', exact: true }).click();
    await expect(page.getByText('Marathon Training Coach')).toBeVisible({ timeout: 10000 });

    // Navigate to detail view and install
    await page.getByText('Marathon Training Coach').click();
    await expect(page.getByRole('button', { name: 'Add Coach' })).toBeVisible({ timeout: 5000 });
    await page.getByRole('button', { name: 'Add Coach' }).click();

    // Should show success message
    await expect(page.getByText(/has been added to your coaches/)).toBeVisible({ timeout: 5000 });
  });

  test('an installed coach is pinned above the store with its handle, and Remove works from the listing', async ({ page }) => {
    // Start with coach already installed
    await setupStoreMocks(page, { installed: ['store-coach-1'] });
    await loginToDashboard(page);
    await page.waitForSelector('main', { timeout: 10000 });
    await page.getByRole('button', { name: 'Discover', exact: true }).click();

    // The copy the install minted sits in "Your coaches" above the store,
    // carrying the handle the athlete types to bring it into a chat. There
    // is no "View My Coaches" link any more: this is where the coach lives.
    const pinned = page.getByRole('region', { name: /Your coaches/ });
    await expect(pinned.getByText('Marathon Training Coach')).toBeVisible({ timeout: 10000 });
    await expect(pinned.getByTestId('coach-handle')).toHaveText('@marathon-training-coach');
    await expect(page.getByText('View My Coaches')).toHaveCount(0);

    // Open the store listing (not the pinned card) and remove from there.
    await page.getByTestId('store-coach-grid').getByText('Marathon Training Coach').click();
    await expect(page.getByText('System Prompt')).toBeVisible({ timeout: 5000 });
    await expect(page.getByRole('button', { name: 'Remove' })).toBeVisible({ timeout: 10000 });

    page.on('dialog', async (dialog) => {
      await dialog.accept();
    });
    await page.getByRole('button', { name: 'Remove' }).click();

    await expect(page.getByText(/has been removed from your library/)).toBeVisible({ timeout: 5000 });
  });
});
