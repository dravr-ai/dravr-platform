# Regression tests — one per confirmed defect

Every defect found in the sweep leaves a test behind. This file says where it goes, what it
must look like, and how to prove it actually guards the fix.

## The must-fail-first protocol

A test written after the fix, that has never been red, proves nothing. Do this, in order:

1. Write the test against the **unfixed** code.
2. Run it. **It must fail — and fail for the reason you filed**, not on a selector typo or a
   timeout. Read the failure output; a wrong-reason failure is a broken test.
3. Apply the fix.
4. Run it again. Green.
5. Record the red output in the run report — that is the evidence the test has teeth.

If the fix is already applied, `git stash` it, run red, `git stash pop`, run green.

## Where the test goes

| The bug lives in | File | Config | CI workflow |
|---|---|---|---|
| UI render / state, given a known API response | `frontend/e2e/<area>.spec.ts` | `playwright.config.ts` | `frontend-tests.yml` → `e2e-tests` |
| API contract, auth, real data shape, anything only reproducible live | `frontend/e2e-real/<area>.real.spec.ts` | `playwright.real.config.ts` | `integration-tests.yml` |
| Backend handler logic | **also** `crates/pierre-server/tests/<area>_test.rs` | — | `ci-backend.yml` / `ci-postgres.yml` |
| WCAG violation | `frontend/e2e/accessibility/<area>.a11y.spec.ts` | `playwright.config.ts` | `frontend-tests.yml` |

Deciding rule: **could this bug have been caught with a stubbed API?** If yes → mocked
(`e2e/`), it is fast and always runs. If the bug *is* the API's behaviour, or only appears
against real seeded data → real (`e2e-real/`). A backend bug gets a Rust test **in addition
to** the browser test — the browser test proves the user-visible symptom is gone, the Rust
test pins the handler.

Prefer adding to an existing spec in the right area over creating a new file. There are ~40
specs already; `login.spec.ts`, `chat.spec.ts`, `settings.spec.ts`, `admin-*.spec.ts`,
`groups.spec.ts`, `connections.spec.ts`, `user-management.spec.ts` and friends cover most
surfaces.

## File header (required by project rules)

```ts
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: <what this spec covers, one line>
// ABOUTME: <the second line — the regression it guards>
```

Both `ABOUTME:` lines are mandatory on every new file and are greppable by design.

## Template — mocked spec (`frontend/e2e/`)

The mocked suite runs with **no backend** (`E2E_TEST=true` disables the Vite proxy). Every
endpoint the surface touches must be routed or the test hangs.

```ts
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: E2E for the analytics LLM consumption panel rendering seeded usage rows.
// ABOUTME: Guards the regression where the panel rendered all-zero despite a non-empty API response.

import { test, expect } from '@playwright/test';
import { setupAndLoginAsAdmin, navigateToTab } from './test-helpers';

test('LLM consumption panel renders the totals returned by the API', async ({ page }) => {
  // Route BEFORE navigating — page.route only intercepts subsequent requests.
  // The URL glob and the body shape both come from the real request you captured
  // during the sweep (see "Getting the URL and body right" below) — never guessed.
  await page.route('**<captured-path>**', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify(/* the real response shape, with values you can assert on */),
    }),
  );

  await setupAndLoginAsAdmin(page);
  await navigateToTab(page, 'Analytics');

  // Assert CONTENT, not existence. A stub that renders zeros must fail this.
  await expect(page.getByText('128,400')).toBeVisible();
  await expect(page.getByText('$4.21')).toBeVisible();
});
```

### Getting the URL and body right

Routes are composed across many axum router modules, so there is no single route table to
read, and `/api-docs/openapi.json` publishes **schemas only, not paths**. Do not guess a
path — a mock on a URL the app never calls silently never fires, and the test passes for the
wrong reason. Resolve it one of these ways, in order of reliability:

1. **From the sweep itself** — `list_network_requests` while the surface loads, then
   `get_network_request` for the exact URL, status and body. This is the request the app
   actually makes; mock that, assert that.
2. **From the app's own API layer** — `packages/api-client/src/domains/` (shared web+mobile)
   or `frontend/src/services/api/` (web-only: admin, a2a, dashboard, keys, usage).
3. **From the handler** — `rg '<fragment>' crates/pierre-server/src/routes/`.

Verified endpoints usable without lookup: `POST /oauth/token`, `POST /api/auth/register`,
`GET /api/auth/session`, `GET /api/oauth/status`, `POST /api/oauth/disconnect/{provider}`,
`GET /api/me/onboarding-status`, `GET /health`.

### Helpers available in `frontend/e2e/test-helpers.ts`

| Helper | Use |
|---|---|
| `applyTestStubs(page)` | theme pin + feature-flag + onboarding-status stubs. Call from any spec-local login helper. |
| `setupDashboardMocks(page, { role, email, displayName, status })` | the full auth/dashboard mock set. `role`: `'user' \| 'admin' \| 'super_admin'`; `status`: `'active' \| 'pending' \| 'suspended'`. |
| `loginToDashboard(page, { email, password })` | fills the login form and waits for `main`. |
| `navigateToTab(page, 'Analytics')` | clicks the sidebar tab by visible name (handles badge suffixes like "2 Users"). |
| `setupAndLoginAsAdmin(page)` / `setupAndLoginAsSuperAdmin(page)` / `setupAndLoginAsUser(page)` | shorthand: mocks + login. |

Gotchas that cost time:

- **The mocked suite runs on port 5174, the dev stack on 5173 — leave both up.** You do not
  need to stop anything the setup script started. This used to be a trap: the config reused an
  existing server, so the sweep's own proxy-mode dev Vite got picked up instead of an
  E2E-mode one, every unmocked request reached the live 8081, the real 401 on a fake token
  logged the app out, and the whole suite died at login looking broken. The config now owns a
  dedicated port with `reuseExistingServer: false`, so that cannot happen. If you ever see the
  suite fail at login again, confirm which port the run bound to before suspecting the specs.
- **Mock every endpoint the surface touches.** The corollary of the above: these specs are
  only safe because unmocked requests go nowhere. A surface that fetches something you did not
  stub will hang rather than fall back to a real server.
- **Route before navigate.** `page.route` does not replay past requests.
- **Onboarding gets in the way.** `setupDashboardMocks` defaults
  `needs_provider_connection: false`; the localStorage step flags
  (`dravr.profile_type_chosen.<userId>`, `dravr.coach_proposal_done.<userId>`) still need
  setting via `page.addInitScript` if your surface sits behind the flow. The mocked user id
  is `user-123`.
- **Billing surfaces** only render because the mocked config launches Vite with
  `VITE_BILLING_ENABLED=true`. Do not assume that outside the e2e webServer.
- **`*.mobile.spec.ts`** is a separate Playwright project (Pixel 7 viewport). Name a mobile-
  breakpoint regression that way or it runs at desktop width and silently proves nothing.

## Template — real-backend spec (`frontend/e2e-real/`)

Requires a live stack (`./bin/setup-db-with-seeds-and-oauth-and-start-servers.sh`). No
`webServer` — the runner fails loudly if 8081 is down, by design.

```ts
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Real-backend E2E for provider disconnect clearing both connection and token rows.
// ABOUTME: Guards the "connected but disconnected" drift between provider_connections and oauth_tokens.

import { test, expect, request as apiRequest } from '@playwright/test';

const PIERRE_URL = process.env.PIERRE_URL ?? 'http://127.0.0.1:8081';
const USER_EMAIL = process.env.WEB_TEST_EMAIL ?? 'webtest@pierre.dev';
const USER_PASSWORD = process.env.WEB_TEST_PASSWORD ?? 'WebTest123!';

test('disconnecting a provider clears the connection for real', async () => {
  const ctx = await apiRequest.newContext({ baseURL: PIERRE_URL });

  const login = await ctx.post('/oauth/token', {
    headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
    form: { grant_type: 'password', username: USER_EMAIL, password: USER_PASSWORD },
  });
  expect(login.ok(), `login failed: ${login.status()}`).toBe(true);
  const { access_token } = await login.json();

  const auth = { Authorization: `Bearer ${access_token}` };

  // GET /api/oauth/status and POST /api/oauth/disconnect/{provider} are the real
  // routes; read the response shape from the handler before asserting on fields.
  const before = await ctx.get('/api/oauth/status', { headers: auth });
  expect(before.ok()).toBe(true);
  expect(JSON.stringify(await before.json())).toContain('strava');

  const disconnect = await ctx.post('/api/oauth/disconnect/strava', { headers: auth });
  expect(disconnect.ok(), `disconnect failed: ${disconnect.status()}`).toBe(true);

  const after = await ctx.get('/api/oauth/status', { headers: auth });
  // Assert the concrete post-state, not merely that the call succeeded.
  expect(/* the provider's connected flag in the real shape */).toBe(false);

  await ctx.dispose();
});
```

Real-spec rules:

- **Uniquify anything you create** (`` `e2e-${Date.now()}@example.com` ``) — the dev DB
  persists across runs and CI re-runs the suite on a shared seeded database.
- **Do not depend on ordering** — `fullyParallel: true`.
- **Restore what you mutate**, or use a throwaway account. A spec that disconnects
  `webtest`'s Strava and leaves it disconnected breaks every later run.
- Read the endpoint's real shape from the handler before asserting on it. Guessing a field
  name produces a test that fails for the wrong reason.

## Template — accessibility spec

```ts
import { test, expect } from '@playwright/test';
import AxeBuilder from '@axe-core/playwright';
import { setupAndLoginAsUser } from '../test-helpers';

test('settings has no WCAG 2.1 AA violations', async ({ page }) => {
  await setupAndLoginAsUser(page);
  // …navigate to the surface…
  const results = await new AxeBuilder({ page })
    .withTags(['wcag2a', 'wcag2aa', 'wcag21aa'])
    .analyze();
  expect(results.violations).toEqual([]);
});
```

Existing a11y specs `.disableRules(['color-contrast'])` and use `expect.soft`. **Do not copy
that into a new spec written to guard a fix you just made** — a soft assertion on a disabled
rule cannot fail, which makes it a stub. Use a hard `expect` on the rule your fix addressed.

## Assertion rules (project-enforced)

- **Assert content, never existence.** `expect(rows).toHaveCount(6)`, `toHaveText('128,400')`,
  a real field value. `expect(res.ok()).toBe(true)` alone passes against a returns-empty
  stub — that is exactly the class of bug this sweep exists to catch.
- **Never weaken an assertion** to make a test pass. That is itself a violation.
- **Never** `test.skip` / `test.only` / `xit` / commented-out tests / `continue-on-error`.
- Prefer role/text locators (`getByRole`, `getByText`) over CSS chains — they survive
  refactors and double as accessibility assertions.
- No arbitrary `waitForTimeout` as a synchronisation primitive; wait on the condition.

## Running them

```bash
cd frontend

bunx playwright test e2e/chat.spec.ts                       # one mocked spec
bunx playwright test e2e/chat.spec.ts -g "renders totals"   # one test
bunx playwright test --project=mobile-chrome                # mobile-viewport project
bunx playwright test e2e/accessibility/                     # a11y only
bun run test:e2e                                            # full mocked suite
bun run test:e2e:real                                       # real-backend suite (stack up)
bunx playwright test --headed --debug e2e/chat.spec.ts      # watch it drive
```

Reports: `frontend/playwright-report/` (mocked), failure screenshots + traces under
`test-results/`.

Backend counterpart — always target the file:

```bash
rg "<test_name>" crates/pierre-server/tests/ --files-with-matches
cargo test --test <file> <test_name> -- --nocapture
```

Never `cargo test <name>` without `--test` (compiles all 325 binaries), never
`cargo test --lib` (runs ~0 tests). Confirm `running N tests` with N > 0 and `N passed` —
`cargo test` exits 0 when zero tests run.

## Before you call it done

- [ ] Test failed on unfixed code, for the filed reason, and the output is in the report.
- [ ] Test passes on fixed code.
- [ ] Full mocked suite still green (no collateral breakage).
- [ ] `bun run lint`, `bun run test -- --run`, `bun run build` green.
- [ ] Backend touched → targeted `cargo test` + `cargo clippy -p <crate>` green.
- [ ] Every P0–P2 in the triage table has a test row filled in — or an explicit, surfaced
      reason it does not.
