// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Playwright config for the production-build suite (frontend/e2e-prod/): dist/ behind the nginx image's security headers
// ABOUTME: The dev-server suite never runs the built bundle nor its CSP; this one runs nothing else

import { defineConfig, devices } from '@playwright/test';

const PORT = Number(process.env.E2E_PROD_PORT ?? '4180');

export default defineConfig({
  testDir: './e2e-prod',
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: 0,
  workers: process.env.CI ? 2 : 4,
  reporter: process.env.CI ? 'github' : 'list',
  timeout: 60000,
  expect: { timeout: 5000 },
  use: {
    baseURL: `http://127.0.0.1:${PORT}`,
    // The language pin the dev suite uses, on this suite's origin.
    storageState: 'e2e-prod/storage-state.json',
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
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
  webServer: {
    // The build is the thing under test, so it is made here rather than
    // trusted from whatever dist/ a previous run left behind.
    command: `bun run build && node e2e-prod/serve-prod.ts`,
    url: `http://127.0.0.1:${PORT}`,
    reuseExistingServer: false,
    timeout: 240000,
    env: { E2E_PROD_PORT: String(PORT) },
  },
});
