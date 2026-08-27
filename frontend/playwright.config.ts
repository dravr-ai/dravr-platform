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
  // CI uses 2; locally `undefined` means one worker per core, which on an
  // 8-core machine stampedes the Vite **dev** server the suite starts. That
  // server compiles routes on demand, so every worker's first navigation waits
  // on the same cold compile and the 10s selector waits in the a11y setup lose
  // the race — reproducibly on `main`, not just on a branch. Four keeps the
  // suite parallel without queueing eight cold compiles behind each other.
  workers: process.env.CI ? 2 : 4,
  reporter: process.env.CI ? 'github' : 'html',
  // Headroom for a cold start plus the test body. Playwright boots a fresh Vite
  // per run (reuseExistingServer: false) and several workers hit it while it is
  // still transforming the app, so the first navigation can take tens of
  // seconds. At the old 30s this budget was fully consumable by that one wait,
  // leaving nothing for the test itself — which surfaced as a click timing out
  // on a nav button that was simply not painted yet. Retries stay at 0: a real
  // failure should still fail.
  timeout: 60000,
  expect: {
    timeout: 5000,
  },
  use: {
    // This suite runs on its OWN port, never the dev server's 5173.
    //
    // The specs mock every endpoint they rely on, which is only safe while
    // unmocked requests go nowhere. A dev Vite (`bun run dev`, as the setup
    // script starts it) proxies /api and /oauth to the real backend, so any
    // endpoint a spec does not stub reaches a live 8081, which rejects the
    // fake `test-jwt-token` with a genuine 401. That fires
    // `pierre:auth:failure`, the app logs itself out, and every spec dies in
    // navigateToTab waiting for a sidebar that never renders — looking for all
    // the world like a broken suite rather than a wrong server.
    //
    // Owning a separate port means the dev stack and this suite coexist, so
    // that failure mode cannot happen. PLAYWRIGHT_BASE_URL still overrides for
    // a worktree that deliberately targets an already-running E2E-mode server.
    baseURL: process.env.PLAYWRIGHT_BASE_URL ?? 'http://localhost:5174',
    // The product default is French. This suite asserts English copy in well
    // over a thousand places, so it states the language it is testing rather
    // than depending on the chrome being untranslated.
    //
    // It lives here rather than in a helper because a helper is opt-in: the pin
    // started inside `applyTestStubs`, and the eight specs that never call it —
    // registration, pending-approval, login, the visual sweeps — kept rendering
    // French and failed the moment the copy was translated. Seeding the same
    // `pierre_app_language` key the switcher itself writes means no spec can
    // miss it, and the pin still travels the path a real preference travels.
    //
    // `locale-default.spec.ts` overrides it with an init script to cover the
    // unpinned French path that real users get.
    storageState: 'e2e/storage-state.json',
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
          // --strictPort so Vite fails loudly instead of silently sliding to
          // the next free port, which would leave `url` below pointing at
          // nothing and time out for a reason that reads as unrelated.
          command: 'bun run dev -- --port 5174 --strictPort',
          url: 'http://localhost:5174',
          // Never reuse. Reuse is what let a proxy-mode dev server masquerade
          // as this one; on a dedicated port there is nothing legitimate to
          // reuse anyway, and always launching guarantees E2E_TEST=true.
          reuseExistingServer: false,
          timeout: 120000,
          env: {
            E2E_TEST: 'true',
            // Billing ships disabled (BILLING_ENABLED defaults false), but its UI
            // must stay E2E-covered — expose the billing surface for the test run.
            VITE_BILLING_ENABLED: 'true',
          },
        },
      }),
});
