---
name: test-web-app
description: Drive the real web app end-to-end with chrome-devtools as both admin and user, collect every defect, fix them, and land a Playwright regression test per issue
user-invocable: true
---

# Test Web App

Exploratory-then-regression testing of the **web SPA against the real stack** — real Pierre
server, real database, real seeded data. Not mocks. You drive the browser with
`chrome-devtools`, record everything that is broken, fix it, and every confirmed defect
leaves behind a Playwright test that would have caught it.

The mobile counterpart is a separate skill (`test-mobile-app`); this one is web only.

## The contract

1. **Real stack, no mocks during exploration.** The mocked `frontend/e2e/` suite already
   passes in CI — it cannot find what it stubs out. Exploration runs against port 5173
   talking to a live 8081.
2. **Every surface gets visited, in both roles.** Admin mode and user mode. A tab you did
   not open is a tab you did not test — say so in the report rather than implying coverage.
3. **Collect first, fix second.** Do not stop the sweep at the first bug. A half-swept app
   produces a fix list that shifts under you.
4. **Every confirmed defect ends with a failing-then-passing Playwright test.** No
   exceptions, no "covered by an existing test" unless you name the test and it genuinely
   fails without the fix.
5. **No skipping.** Never `test.skip`/`.only`/`xit`, never `continue-on-error`, never
   weaken an assertion to accommodate a bug. If something cannot be fixed now, STOP and
   tell ChefFamille — do not leave it silently unlisted.
6. **Report faithfully.** If 4 of 22 surfaces failed, the report says 4 failed. Never round
   a partial sweep up to "everything works".

## Phase 0 — Preflight (before touching anything)

```bash
git status                                   # uncommitted work in the shared worktree?
git log --oneline -5
gh run list --branch main --limit 5 --json workflowName,conclusion
```

**Gate — ask ChefFamille before proceeding if any of these are true:**

- Uncommitted work exists in the worktree (the setup script does not touch tracked files,
  but a running stack does get killed).
- CI on `main` has been red for 2+ runs → ask "Should I investigate CI before the sweep?"
- Anything is already listening on 8081 / 5173 → someone may be mid-session. The setup
  script **kills all services and resets the dev database**. That is destructive; confirm.

Confirm the coach source exists — the setup script hard-exits without it:

```bash
ls ../dravr-contremaitre/prompts/coaches >/dev/null && echo "coaches ok"
```

## Phase 1 — Boot the stack

```bash
./bin/setup-db-with-seeds-and-oauth-and-start-servers.sh
```

Debug build by default (`--release` for an optimized run; slower to build, faster to drive).
This resets the DB, runs migrations, creates the admin, seeds coaches/demo/social/mobility,
seeds fixture-backed Strava+Garmin activities, and starts:

| Service | Port | Log |
|---|---|---|
| Pierre server | 8081 | `logs/pierre-server.log` |
| Vite frontend | 5173 | `logs/frontend.log` |
| Expo (mobile, ignore here) | 8082 | `logs/expo.log` |
| Dev fixture API (Strava/Garmin) | 9555 | `logs/fixture.log` |

Verify before driving anything — a sweep against a half-booted stack invents bugs:

```bash
curl -sf http://localhost:8081/health && echo " server ok"
curl -sf -o /dev/null http://localhost:5173 && echo " vite ok"
curl -sf http://127.0.0.1:9555/health && echo " fixture ok"
```

Credentials come in two kinds — do not conflate them:

**The admin is environment-dependent.** The setup script resolves
`${ADMIN_EMAIL:-admin@example.com}` / `${ADMIN_PASSWORD:-AdminPassword123}`, so `.envrc`
wins. Resolve it before logging in, never assume the default:

```bash
set -a; source .envrc; set +a; echo "$ADMIN_EMAIL"
```

On a machine whose `.envrc` sets `ADMIN_EMAIL="admin@pierre.mcp"`, `admin@example.com` does
not exist and returns `invalid_grant` — which reads exactly like a broken login and will
waste a sweep. CI has no override and gets the default.

**The seeded users are constants** from `crates/pierre-seeders/src/demo_data.rs`:

| Role | Email | Password |
|---|---|---|
| **User** (web, Strava-backed, 30 activities) | `webtest@pierre.dev` | `WebTest123!` |
| Mobile test (Garmin-backed) | `mobiletest@pierre.dev` | `MobileTest1234` |
| Demo user (Strava) | `alice@acme.com` | `DemoUser123!` |
| Demo user (Garmin) | `bob@startup.io` | `DemoUser123!` |

Verify the whole table before sweeping — it is 6 seconds and rules out a misseeded stack:

```bash
set -a; source .envrc; set +a
cd frontend && bunx playwright test --config=playwright.real.config.ts seeded-credentials
```

Admin token for API-side cross-checks: `logs/admin-token.txt`.

> The admin is created with `--super-admin`, so "admin mode" renders the **super-admin** tab
> set (all admin tabs **plus** Admin Tokens). A plain `admin` role is deliberately narrower;
> if you need to test that, mint a second user.

Open a working ledger in the scratchpad and append to it as you go — never hold findings
only in your head across a 20-surface sweep:

```
<scratchpad>/web-sweep-<date>.md
```

## Phase 2 — Attach chrome-devtools

Tools are `mcp__chrome-devtools__*`; short names used below.

1. `new_page` → `http://localhost:5173`
2. `list_pages` to confirm the target, `select_page` if more than one.
3. `resize_page` → 1440×900 (desktop baseline; the mobile breakpoint is <768px and gets its
   own pass in Phase 5).

**Interaction loop for every screen — do all four, every time:**

| Step | Tool | Why |
|---|---|---|
| Snapshot the a11y tree | `take_snapshot` | gives the `uid`s that `click`/`fill`/`hover` need, and is itself the accessibility evidence |
| Act | `click` / `fill` / `fill_form` / `press_key` / `hover` | drive the real interaction, not a URL jump |
| Settle | `wait_for` on expected text | never a blind sleep |
| Harvest | `list_console_messages` + `list_network_requests` | the defects that do not render |

Harvest rules — record as a finding when you see:

- **Console:** any `error`; any `warning` naming React (`key`, `act`, hydration, state update
  on unmounted), any uncaught promise rejection.
- **Network:** any XHR/fetch with status **≥ 400** that the UI does not visibly and
  correctly surface. A 401 on a logged-out probe is expected; a 403/500 on a tab you just
  opened is a finding. Pull the body with `get_network_request` before judging.
- **Render:** blank panel, infinite spinner (>10s), skeleton that never resolves, `NaN`,
  `undefined`, `[object Object]`, an empty state where seeded data exists.
- **Silent-stub smell:** a panel that renders all-zero / empty while the API returned rows.
  Cross-check with `curl` + the token in `logs/admin-token.txt`, using the URL you captured
  from `get_network_request` — never a guessed path. This class hides for months.

`take_screenshot` (fullPage) each surface into the run folder; screenshots are the diff you
show ChefFamille, and the only durable evidence of a visual defect.

## Phase 3 — Admin-mode sweep

Log in as `admin@example.com` / `AdminPassword123`. Landing tab is `users`.

Navigate **by clicking the sidebar** — that exercises the nav itself. Hash routes
(`#users`, `#coaches`, `#groups/<id>`, `#chat/<conversationId>`) exist and `navigate_page`
to them works via the `hashchange` listener; use that only to recover from a stuck nav, and
file the stuck nav as a finding.

Full tab-by-tab checklist with per-surface expectations:
**[reference/surfaces.md](reference/surfaces.md) → Admin surfaces** (22 tabs).

## Phase 4 — User-mode sweep

Log out (user menu → sign out), then log in as `webtest@pierre.dev` / `WebTest123!`.

A fresh browser profile means **onboarding runs first** — that is a test surface, not an
obstacle. Walk it: `profile_type` → `connect_provider` → `coach_proposal` →
`messaging_channel` → `messaging_configure`. `webtest` already has a seeded Strava
connection so `connect_provider` should self-satisfy; if it hard-gates anyway, that is a P0.

Step completion is partly localStorage (`dravr.profile_type_chosen.<userId>`,
`dravr.coach_proposal_done.<userId>`). To re-test onboarding from scratch, clear those keys
via `evaluate_script` rather than reseeding the DB. To skip straight to the dashboard on a
re-run, set them.

Landing tab is `chat`. Full checklist:
**[reference/surfaces.md](reference/surfaces.md) → User surfaces** (7 tabs + Settings).

## Phase 5 — Cross-cutting passes

Run these in both roles unless noted.

1. **Theme.** Toggle dark ⇄ light. Every surface stays legible; no unstyled flash, no
   hardcoded light-mode color surviving into dark. localStorage key `dravr.theme`.
2. **Mobile breakpoint.** `resize_page` 393×851 (<768px). Bottom tab bar shows the primary
   4 (`users/coaches/coach-store/groups` admin; `chat/my-coaches/insights/groups` user);
   everything else is in the off-canvas drawer, and the drawer badge aggregates.
   No horizontal scroll, no clipped content, touch targets ≥44×44.
3. **Back/forward.** `press_key` browser-back across visited tabs — the app pushes history
   per tab *and* sub-view. Back must walk tabs, not exit to login.
4. **Deep link + reload.** Reload on a deep route (`#groups/<id>`, `#chat/<id>`); the same
   view must restore, not reset.
5. **Auth boundaries.** As `webtest`, hit an admin-only surface (`#users`, `#admin-tokens`)
   via `navigate_page`. It must not render admin data. A rendered admin panel for a
   non-admin is **P0 security**, filed and fixed first.
6. **Accessibility.** `take_snapshot` already gives the tree; for a scored pass run
   `lighthouse_audit` on the 3–4 heaviest surfaces. Axe-based specs live in
   `frontend/e2e/accessibility/`.

## Phase 6 — Triage

Consolidate the ledger. One row per defect, deduplicated (one root cause = one row, list
the surfaces it appears on).

| ID | Severity | Surface | Role | Symptom | Evidence | Root cause | Fix | Test |
|---|---|---|---|---|---|---|---|---|

Severity:

- **P0** — security/tenant boundary, data loss, crash, blank surface, login broken.
- **P1** — feature does not work or shows wrong data.
- **P2** — console error, failed request the UI swallows, visual break.
- **P3** — polish, copy, minor a11y.

Before filing, establish the root cause from **primary evidence** — server log, network
body, source. A guessed cause produces a guessed fix and a test that proves nothing.

Show ChefFamille the triage table **before** starting fixes. Scope decisions are theirs.

## Phase 7 — Fix

Work P0 → P3. Per project rules:

- Smallest reasonable change. **Never rewrite an existing implementation from scratch to fix
  a bug — stop and get explicit permission.**
- Rust: no `anyhow!` anywhere, structured errors only; no `unwrap`/`expect` on
  runtime-possible failures; no `#[allow(clippy::...)]`.
- No stubs, no confession comments ("for now", "in a real implementation"). If the honest
  fix is bigger than the sweep, surface it — do not paper it.
- Every DB query keeps `tenant_id` in the WHERE clause.
- If the fix touches a notify event, a messaging string, or an `McpTool`, mirror it into
  `dravr-contremaitre` in the same change (5 locales; `notify-events.yaml`;
  `EXPECTED_TOOLS` + SDK type regen) — those tests are full-suite-only and will red `main`
  after the squash, not on your branch.

## Phase 8 — Regression test per defect

**Non-negotiable: one Playwright test per confirmed defect.** Write it *before* the fix (or
stash the fix), watch it fail for the right reason, then apply the fix and watch it pass. A
test that never failed proves nothing.

Routing — pick by where the bug actually lives:

| Bug lives in | Destination | Config |
|---|---|---|
| UI render/state given a known API response | `frontend/e2e/<area>.spec.ts` | `playwright.config.ts` (mocked, runs in `frontend-tests.yml`) |
| API contract, auth, real data shape, anything only reproducible live | `frontend/e2e-real/<area>.real.spec.ts` | `playwright.real.config.ts` (real 8081, runs in `integration-tests.yml`) |
| Backend handler logic | **also** `crates/pierre-server/tests/<area>_test.rs` | `cargo test --test <file>` |
| Accessibility violation | `frontend/e2e/accessibility/<area>.a11y.spec.ts` | mocked config |

Templates, helper inventory, and assertion rules (assert **content**, never `.is_ok()`-style
existence checks): **[reference/regression-tests.md](reference/regression-tests.md)**.

## Phase 9 — Validate and land

Leave the stack from Phase 1 running. The mocked suite launches its own Vite on **5174**, so
it does not collide with the dev server on 5173, and `test:e2e:real` needs the live stack
anyway.

```bash
cd frontend
bunx playwright test e2e/<new-spec>.spec.ts        # the new tests, targeted
bun run test:e2e                                   # full mocked suite, no regressions
bun run test:e2e:real                              # real-backend suite (stack must be up)
bun run lint
bun run test -- --run
bun run build          # the REAL type gate — `type-check` is near-noop here
```

Backend, if touched — always target the file, never bare `cargo test`:

```bash
cargo test --test <file> -- --nocapture
cargo clippy -p <crate> --all-targets --all-features -- -D warnings
```

**Confirm tests actually ran.** `running N tests` with N > 0 and `N passed`. `running 0
tests` / `filtered out` means a wrong `--test` target — exit code 0 is not evidence.

Then:

```bash
./scripts/ci/pre-push-validate.sh     # the only local gate; never --no-verify
git add -A && git commit && git push origin main    # fixes are bug fixes → straight to main
```

**Push starts validation, it does not end it.** Watch CI on the pushed commit until every
relevant workflow reaches terminal status (WebFetch the Actions page first; `gh run list`
sparingly; never `gh run watch` or a <60s poll loop). Cancelled ≠ green. Red → fix and
re-push in the same session.

## Phase 10 — Report

Write the run report to `claude_docs/` (the gitignored symlink into the vault's
`Claude Outputs/`; create the symlink if missing) via the `obsidian-writer` skill.

Report contains:

- Stack state: build mode, commit SHA, date.
- Coverage: surfaces visited / total, per role. **Name anything not visited and why.**
- The triage table, with final status per row.
- Every fix: file, one-line rationale, the test that now guards it.
- Anything found but **not** fixed, with the reason — never let a known defect vanish.
- CI status on the head commit.

## Teardown

```bash
./bin/stop-all.sh
```

Leave the stack up if ChefFamille is going to look at it — ask.
