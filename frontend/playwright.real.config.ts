// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Playwright config for opt-in real-backend E2E specs (frontend/e2e-real/).
// ABOUTME: Run via `bun run test:e2e:real` after `./bin/setup-db-with-seeds-and-oauth-and-start-servers.sh`.

import { defineConfig, devices } from '@playwright/test';

const CHROMIUM = {
  ...devices['Desktop Chrome'],
  launchOptions: {
    args: process.env.CI ? ['--no-sandbox', '--disable-setuid-sandbox', '--disable-dev-shm-usage'] : [],
  },
};

/** The specs that start the scraper double (`e2e-real/sciotte-double.ts`). */
const SCRAPER_DOUBLE_SPECS = ['**/home-sync.real.spec.ts', '**/coros-code-step.real.spec.ts'];

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
    // The product default is French (`DEFAULT_LANGUAGE` in @pierre/i18n), so the
    // login page renders "Se connecter". `coaching-persona.real.spec.ts` drives the
    // real UI and matches English copy in six places — /sign in|log in/,
    // /open settings/, /coaching style/ — so it states the language it is testing
    // rather than depending on the chrome being untranslated.
    //
    // Pinned on `FRONTEND_URL` (http://localhost:5173), NOT on the `baseURL` above.
    // `baseURL` here is the API the request-context tests talk to; the one spec that
    // drives a browser loads the SPA from Vite via its own `FRONTEND_URL`, and
    // `localhost` and `127.0.0.1` are distinct localStorage origins — a state pinned
    // on the wrong one is silently inert, which is how the first attempt at this
    // failed CI identically.
    //
    // Same mechanism as the other two suites (e2e pins 5174, integration pins 5173):
    // a per-spec selector workaround would fix the login click and then fail on the
    // next English label down, of which this spec has five more.
    storageState: 'e2e-real/storage-state.json',
    trace: 'on-first-retry',
  },

  projects: [
    {
      name: 'chromium',
      testIgnore: SCRAPER_DOUBLE_SPECS,
      use: CHROMIUM,
    },
    {
      // The specs that start the scraper double share its one port and the
      // one DRAVR_SCIOTTE_REMOTE_URL the server under test names, so they run
      // one at a time, beside the rest of the suite.
      name: 'scraper-double',
      testMatch: SCRAPER_DOUBLE_SPECS,
      workers: 1,
      use: CHROMIUM,
    },
  ],

  // No webServer — these specs require a real Pierre server already running
  // on port 8081 with seeded admin/coaches/demo data. The runner fails loudly
  // if the server isn't up, by design. home-sync.real.spec.ts and
  // coros-code-step.real.spec.ts need a server pointed at the scraper double
  // they start: CI's integration lane starts one, and locally
  // scripts/e2e-home-sync-local.sh boots one for home-sync.
});
