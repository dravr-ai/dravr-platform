// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Playwright E2E test configuration for the Pierre frontend.
// ABOUTME: Configures browser settings, base URL, and test directory structure.

import { defineConfig, devices } from '@playwright/test';

export default defineConfig({
  testDir: './e2e',
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: 0, // Disable retries to prevent CI timeout - failing tests should be fixed not retried
  workers: process.env.CI ? 2 : undefined, // Use 2 workers in CI for reasonable speed
  reporter: process.env.CI ? 'github' : 'html',
  timeout: 30000,
  expect: {
    timeout: 5000,
  },
  use: {
    // Allow PLAYWRIGHT_BASE_URL to override so a developer can target an
    // already-running Vite from a feature-branch worktree (e.g. when the
    // parent worktree's :5173 Vite is also running). Default stays 5173
    // for the CI webServer launch below.
    baseURL: process.env.PLAYWRIGHT_BASE_URL ?? 'http://localhost:5173',
    trace: 'on-first-retry',
    screenshot: 'only-on-failure',
  },

  projects: [
    {
      name: 'chromium',
      // Mobile-only specs belong to the mobile-chrome project; running them
      // at desktop viewport breaks the breakpoint contract.
      testIgnore: ['**/*.mobile.spec.ts'],
      use: {
        ...devices['Desktop Chrome'],
        launchOptions: {
          args: process.env.CI
            ? ['--no-sandbox', '--disable-setuid-sandbox', '--disable-dev-shm-usage']
            : [],
        },
      },
    },
    {
      name: 'mobile-chrome',
      // Pixel-class viewport (393x851 in playwright 1.57). We run a small
      // subset of specs against this project — the full suite is too heavy
      // to mirror twice.
      testMatch: ['**/*.mobile.spec.ts'],
      use: {
        ...devices['Pixel 7'],
        launchOptions: {
          args: process.env.CI
            ? ['--no-sandbox', '--disable-setuid-sandbox', '--disable-dev-shm-usage']
            : [],
        },
      },
    },
  ],

  // Run the Vite dev server before starting tests, unless the user pointed
  // PLAYWRIGHT_BASE_URL at an existing server (worktree dev server).
  // E2E_TEST=true disables backend proxy since all APIs are mocked by Playwright
  ...(process.env.PLAYWRIGHT_BASE_URL
    ? {}
    : {
        webServer: {
          command: 'bun run dev',
          url: 'http://localhost:5173',
          reuseExistingServer: !process.env.CI,
          timeout: 120000,
          env: {
            E2E_TEST: 'true',
          },
        },
      }),
});
