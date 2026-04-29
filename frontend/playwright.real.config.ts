// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Playwright config for opt-in real-backend E2E specs (frontend/e2e-real/).
// ABOUTME: Run via `bun run test:e2e:real` after `./bin/setup-db-with-seeds-and-oauth-and-start-servers.sh`.

import { defineConfig, devices } from '@playwright/test';

export default defineConfig({
  testDir: './e2e-real',
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: 0,
  workers: process.env.CI ? 2 : undefined,
  reporter: process.env.CI ? 'github' : 'list',
  timeout: 30000,
  expect: { timeout: 5000 },
  use: {
    baseURL: 'http://127.0.0.1:8081',
    trace: 'on-first-retry',
  },

  projects: [
    {
      name: 'chromium',
      use: {
        ...devices['Desktop Chrome'],
        launchOptions: {
          args: process.env.CI
            ? ['--no-sandbox', '--disable-setuid-sandbox', '--disable-dev-shm-usage']
            : [],
        },
      },
    },
  ],

  // No webServer — these specs require a real Pierre server already running
  // on port 8081 with seeded admin/coaches/demo data. The runner fails loudly
  // if the server isn't up, by design.
});
