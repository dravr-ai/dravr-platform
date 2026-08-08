---
name: test-mobile-app
description: Drive the real Expo app on a simulator as both a regular user and an operator, collect every defect, fix them, and land a Maestro flow or jest integration spec per issue
user-invocable: true
---

# Test Mobile App

Exploratory-then-regression testing of the **React Native / Expo app against the real stack** —
real Pierre server on 8081, real database, real seeded data. Not mocks. You drive a booted
simulator with `mcp__mobile-mcp__*` / `mcp__ios-simulator__*`, record everything that is broken,
fix it, and every confirmed defect leaves behind a test that would have caught it.

The web counterpart is a separate skill (`test-web-app`); this one is mobile only. The e2e tool
here is **Maestro** (`.maestro/`), not Detox — some comments in the repo still name Detox as the
thing Maestro replaced.

## The contract

1. **Real stack, no mocks during exploration.** The jest suite (`bun run test`) globally mocks
   AsyncStorage, SecureStore, expo-router, reanimated, FlashList and the native modules
   (`jest.setup.js`) — it cannot find what it stubs out. Exploration runs a real Expo bundle on a
   real simulator against a live 8081.
2. **Every surface gets visited, in both roles.** Regular user *and* operator (admin). Mobile has
   no admin console; the role difference is a **gate** — `SettingsScreen.tsx:79` computes
   `isAdminUser` and hides the personal Data Providers section for admins. A screen you did not
   open is a screen you did not test — say so in the report rather than implying coverage.
3. **Collect first, fix second.** Do not stop the sweep at the first bug. A half-swept app
   produces a fix list that shifts under you.
4. **Every confirmed defect ends with a failing-then-passing test.** No exceptions, no "covered
   by an existing flow" unless you name the flow and it genuinely fails without the fix.
5. **No skipping.** Never `test.skip`/`.only`/`xit`, never `continue-on-error`, never delete a
   flow from `config.yaml` to make the suite green, never weaken an assertion to accommodate a
   bug. If something cannot be fixed now, STOP and tell ChefFamille.
6. **Report faithfully.** If 5 of 30 screens failed, the report says 5 failed. Never round a
   partial sweep up to "everything works".

## Phase 0 — Preflight (before touching anything)

```bash
git status                                   # uncommitted work in the shared worktree?
git log --oneline -5
gh run list --branch main --limit 5 --json workflowName,conclusion
```

**Gate — ask ChefFamille before proceeding if any of these are true:**

- Uncommitted work exists in the worktree (the setup script does not touch tracked files, but a
  running stack does get killed).
- CI on `main` has been red for 2+ runs → ask "Should I investigate CI before the sweep?"
- Anything is already listening on 8081 / 8082 / 5173 → someone may be mid-session. The setup
  script **kills all services and resets the dev database**. That is destructive; confirm.

Confirm the coach source exists — the setup script hard-exits without it:

```bash
ls ../dravr-contremaitre/prompts/coaches >/dev/null && echo "coaches ok"
```

**Then resolve which backend the app will actually talk to.** This is the single most expensive
mistake available on mobile. `src/services/apiUrl.ts` gives `EXPO_PUBLIC_API_URL` top precedence
over every fallback, and `frontend-mobile/.env` is untracked and per-machine — it commonly holds
a Cloud Run URL left behind by a tunnel run:

```bash
cat frontend-mobile/.env          # EXPO_PUBLIC_API_URL=...
```

If it is not your local server, the sweep is testing *deployed dev*, and every "stale data" or
"missing seed" finding is an artefact. Point it at the local stack before starting:

```
EXPO_PUBLIC_API_URL="http://localhost:8081"      # iOS Simulator
EXPO_PUBLIC_API_URL="http://10.0.2.2:8081"       # Android emulator (host loopback)
```

Metro reads `.env` at bundler start — restart Expo after editing it, or the old value stays
baked into the bundle.

## Phase 1 — Boot the stack

Same script as web; it starts Expo too.

```bash
./bin/setup-db-with-seeds-and-oauth-and-start-servers.sh
```

It resets the DB, runs migrations, creates the admin, seeds coaches/demo/social/mobility, seeds
fixture-backed Strava + Garmin activities, and starts:

| Service | Port | Log |
|---|---|---|
| Pierre server | 8081 | `logs/pierre-server.log` |
| Vite frontend (web, ignore here) | 5173 | `logs/frontend.log` |
| Expo / Metro | 8082 | `logs/expo.log` |
| Dev fixture API (Strava/Garmin) | 9555 | `logs/fixture.log` |

**8081 is reserved for Pierre. Expo is 8082, always** — `bun start` is `expo start --go --port
8082`. A bare `expo start` defaults to 8081 and collides with the server.

Verify before driving anything — a sweep against a half-booted stack invents bugs:

```bash
curl -sf http://localhost:8081/health && echo " server ok"
curl -sf -o /dev/null http://127.0.0.1:8082 && echo " metro ok"
curl -sf http://127.0.0.1:9555/health && echo " fixture ok"
```

The script launches Expo Go on the booted simulator (`expo start --ios --go`). If Expo Go is
missing: `./bin/install-expo-go.sh`. For the native/dev-client build (needed only for speech
recognition and native MMKV), the script takes `--native`.

Two bundle ids are in play and confusing them wastes a run:

| Runtime | Bundle id | How it loads |
|---|---|---|
| Expo Go (default) | `host.exp.Exponent` | deep link `exp://127.0.0.1:8082` |
| Native / dev-client build | `com.pierre.fitness` | installed app, Metro on 8082 |

`.maestro/config.yaml` declares `appId: com.pierre.fitness`, but every flow enters through
`helpers/launch-app.yaml`, which declares `host.exp.Exponent` and opens the `exp://` link. That
is deliberate, not drift.

**Credentials.** The seeded users are constants in `crates/pierre-seeders/src/demo_data.rs`:

| Role | Email | Password | Seeded with |
|---|---|---|---|
| **Mobile test user** | `mobiletest@pierre.dev` | `MobileTest1234` | Garmin, 30 activities / 30 days |
| Web test user | `webtest@pierre.dev` | `WebTest123!` | Strava, 30 activities |
| Demo user | `alice@acme.com` | `DemoUser123!` | Strava |
| Demo user | `bob@startup.io` | `DemoUser123!` | Garmin |

`mobiletest` is the account `.maestro/config.yaml` pins as `TEST_EMAIL`/`TEST_PASSWORD` and the
account both e2e workflows seed. Sweep as that user.

**The admin is environment-dependent.** The setup script resolves
`${ADMIN_EMAIL:-admin@example.com}` / `${ADMIN_PASSWORD:-AdminPassword123}`, so `.envrc` wins.
Resolve it, never assume the default — a wrong admin returns `invalid_grant` and reads exactly
like broken login:

```bash
set -a; source .envrc; set +a; echo "$ADMIN_EMAIL"
```

Admin token for API-side cross-checks: `logs/admin-token.txt`. A user token for cross-checking
what a screen *should* be showing:

```bash
curl -s -X POST http://localhost:8081/oauth/token \
  -H 'Content-Type: application/x-www-form-urlencoded' \
  -d 'grant_type=password&username=mobiletest@pierre.dev&password=MobileTest1234' | jq -r .access_token
```

Open a working ledger in the scratchpad and append to it as you go — never hold findings only in
your head across a 30-screen sweep:

```
<scratchpad>/mobile-sweep-<date>.md
```

## Phase 2 — Attach the simulator driver

Two toolsets are available; they see the same simulator.

| Toolset | Use it for |
|---|---|
| `mcp__ios-simulator__*` | the accessibility tree (`ui_describe_all`, `ui_find_element`, `ui_describe_point`), precise `ui_tap` / `ui_type` / `ui_swipe`, `screenshot`, `record_video` / `stop_recording` |
| `mcp__mobile-mcp__*` | device selection (`mobile_list_available_devices`), `mobile_launch_app` / `mobile_terminate_app`, `mobile_list_elements_on_screen`, `mobile_open_url` (deep links), `mobile_press_button`, `mobile_set_orientation`, `mobile_list_crashes` / `mobile_get_crash` |

Bring-up:

1. `mcp__ios-simulator__get_booted_sim_id` (or `open_simulator`, then
   `mcp__mobile-mcp__mobile_list_available_devices`).
2. `mcp__mobile-mcp__mobile_launch_app` on `host.exp.Exponent`, then
   `mcp__mobile-mcp__mobile_open_url` with `exp://127.0.0.1:8082`.
3. Expo Go v55 puts a native project-info sheet ("Dravr / Close / Reload / Go Home") over the RN
   tree on first load. Dismiss it (tap **Close**) before trusting an empty accessibility tree —
   `.maestro/helpers/launch-app.yaml` retries this three times for exactly this reason. An
   "everything is blank" finding that turns out to be that sheet is not a finding.

**Interaction loop for every screen — do all four, every time:**

| Step | Tool | Why |
|---|---|---|
| Describe | `ui_describe_all` / `mobile_list_elements_on_screen` | the element list you act against, and the accessibility evidence itself |
| Act | `ui_tap` / `ui_type` / `ui_swipe` / `mobile_press_button` | drive the real interaction, not a deep-link jump |
| Settle | re-describe until the expected element appears | never a blind sleep |
| Harvest | `logs/expo.log`, `logs/pierre-server.log`, `mobile_list_crashes` | the defects that do not render |

Harvest rules — record as a finding when you see:

- **Metro / JS:** any red-box error, any `console.error`, any unhandled promise rejection, any
  React key/`act`/state-on-unmounted warning in `logs/expo.log`.
- **Native:** any entry from `mobile_list_crashes`; pull the detail with `mobile_get_crash`.
- **Server:** a 4xx/5xx in `logs/pierre-server.log` correlated in time with the screen you just
  opened, that the UI does not visibly and correctly surface. A 401 on a logged-out probe is
  expected; a 403/500 on a tab you just opened is a finding.
- **Render:** blank screen, spinner that never resolves (>10s), `NaN`, `undefined`,
  `[object Object]`, an empty state where seeded data exists, content trapped under the floating
  tab bar or the keyboard.
- **Silent-stub smell:** a panel that renders all-zero / empty while the API returned rows.
  Cross-check with `curl` and the user token above, using the path from the screen's own API
  call site (`packages/api-client/src/domains/`) — never a guessed path. This class hides for
  months.

`screenshot` (or `mobile_save_screenshot`) each surface into the run folder; screenshots are the
diff you show ChefFamille and the only durable evidence of a visual defect. `record_video` /
`stop_recording` around anything animated (tab-bar expansion, chat streaming).

## Phase 3 — User-mode sweep

Log in as `mobiletest@pierre.dev` / `MobileTest1234` on the login screen (`login-screen` →
`email-input`, `password-input`, `login-button`).

A cleared keychain means **onboarding runs first** — that is a test surface, not an obstacle.
`app/_layout.tsx` (RootLayoutNav) resolves `needs_provider_connection` from
`/api/me/onboarding-status` and routes through the shared step registry:
`profile-type` → `connect` → `coach-proposal` → `messaging-channel` → `messaging-configure`.

Two steps are AsyncStorage-backed, not server state (`useProfileTypeChosen.ts`,
`useCoachProposalSeen.ts`):

```
dravr.profile_type_chosen.<userId>      # '1' = done
dravr.coach_proposal_done.<userId>      # '1' = done
```

Same key names as web, AsyncStorage instead of localStorage. Both hooks **fail open** (storage
error ⇒ treated as done), so a hung onboarding step is a real defect, never a storage hiccup.

Then walk all six tabs and their stacks. Navigate **by tapping the tab bar** (`tab-chat`,
`tab-coaches`, `tab-discover`, `tab-groups`, `tab-insights`, `tab-settings`) — that exercises the
nav itself. Deep-linking via `mobile_open_url` is for recovering from a stuck nav; file the stuck
nav as a finding.

Full screen-by-screen checklist with routes, anchors and per-surface expectations:
**[reference/surfaces.md](reference/surfaces.md)**.

## Phase 4 — Operator-mode pass

Log out (Settings → `settings-logout-button`), then log in as the resolved `$ADMIN_EMAIL`.

There is no admin console on mobile. What you are testing is the **pure-operator gate**: an
`admin` / `super_admin` must **not** see the personal Data Providers section
(`settings-data-section`, `SettingsScreen.tsx:312`), and a regular user must. Confirm both
directions — the gate rendering for an admin is a finding; the gate hiding it from a plain user
is a P0-adjacent regression of a shipped behaviour (`__tests__/SettingsScreenAdminGate.test.tsx`
is its unit-level pin).

Sweep the rest of the tabs as the admin too. An operator account has no seeded provider data, so
"empty" is the *expected* state on activity-backed surfaces — an empty chat coach list or empty
insights feed for the admin is not automatically a defect. Verify against the API before filing.

## Phase 5 — Cross-cutting passes

Run these as the user unless noted.

1. **Appearance.** Settings → Appearance: `appearance-option-system` / `-light` / `-dark`.
   Persisted in AsyncStorage under `pierre.appearance_pref`; `system` resolves through
   NativeWind. Every screen stays legible in both schemes; no hardcoded light-mode colour
   surviving into dark.
2. **Back navigation.** `mobile_press_button` back (Android) / swipe-from-edge (iOS) across each
   stack. Back must walk the stack, not drop to login or to a blank Slot.
3. **Deep link + relaunch.** The app scheme is `pierre` (`app.config.js`). Relaunch on a deep
   route (`/(app)/(tabs)/(groups)/<id>`, `/(app)/memory`); the same view must restore, not reset.
4. **Server-unreachable banner.** `ServerStatusBanner` renders from `useServerStatus` above the
   tabs. Stop the server (`./bin/stop-server.sh`), confirm the banner appears and Retry works,
   restart. A screen that silently renders empty instead of surfacing the outage is a finding.
5. **Keyboard and safe area.** Every text-entry screen: keyboard must not cover the input or the
   submit control. The floating tab bar is `COLLAPSED_HEIGHT + 40` tall
   (`TAB_BAR_BOTTOM_OFFSET`); content clipped beneath it is a finding.
6. **Orientation.** `mobile_set_orientation` landscape on the heaviest screens (chat, coach
   library, store). The app declares `orientation: 'portrait'` — a screen that reflows badly
   under a forced rotation is lower severity than one that crashes.
7. **Accessibility.** `ui_describe_all` is the tree. Interactive elements need an
   `accessibilityRole` and a label (the tab bar sets both). Touch targets ≥44×44.
8. **Android.** If an emulator is available, repeat the login + one tab per area with
   `EXPO_PUBLIC_API_URL=http://10.0.2.2:8081`. Android-only defects are real and the nightly
   Android suite already carries a reduced critical list, so it will not catch them for you.

## Phase 6 — Triage

Consolidate the ledger. One row per defect, deduplicated (one root cause = one row, list the
screens it appears on).

| ID | Severity | Screen | Role | Platform | Symptom | Evidence | Root cause | Fix | Test |
|---|---|---|---|---|---|---|---|---|---|

Severity:

- **P0** — security/tenant boundary, data loss, native crash, blank screen, login broken.
- **P1** — feature does not work or shows wrong data.
- **P2** — JS error, failed request the UI swallows, layout break, content under the tab bar.
- **P3** — polish, copy, minor a11y.

Before filing, establish the root cause from **primary evidence** — server log, Metro log, crash
report, source line. A guessed cause produces a guessed fix and a test that proves nothing.

**Known-intentional states — do not file these as defects:**

| Observation | Why it is correct |
|---|---|
| No Garmin card on Connections, even for the Garmin-seeded `mobiletest` | `/api/providers` deliberately skips `garmin` (`crates/pierre-routes-auth/src/oauth.rs`, `if provider_name == oauth_providers::GARMIN { continue; }`) — Garmin's OAuth API is uncredentialed, so the supported Garmin path is the `sciotte_garmin` scrape card |
| Billing screen unreachable / redirects to Settings | `BILLING_ENABLED = false` in `src/constants/features.ts`; `app/(app)/billing.tsx` redirects by design |
| Memory screen has no in-app entry point | `app/(app)/memory.tsx` is deep-link-only today |
| `.maestro/onboarding/` never runs in the suite | Not registered in `config.yaml`; the flow header documents why (`mobiletest` has providers connected, so the forced-onboarding assertion needs a zero-provider user) |
| Expo Go project-info sheet / dev menu over the app | Expo Go v55 startup, not the app |

Show ChefFamille the triage table **before** starting fixes. Scope decisions are theirs.

## Phase 7 — Fix

Work P0 → P3. Per project rules:

- Smallest reasonable change. **Never rewrite an existing implementation from scratch to fix a
  bug — stop and get explicit permission.**
- Mobile: NativeWind `className`, no inline styles in new code; explicit prop types, `unknown` +
  type guards over `any`; React Query for server state, Context for app state.
- Shared web/mobile API methods belong in `packages/api-client/src/domains/`, shared types in
  `@pierre/shared-types` — never a mobile-local duplicate of an endpoint the web already calls.
- Rust, if the fix reaches the backend: no `anyhow!` anywhere, structured errors only; no
  `unwrap`/`expect` on runtime-possible failures; no `#[allow(clippy::...)]`; `tenant_id` stays
  in every WHERE clause.
- No stubs, no confession comments ("for now", "in a real implementation"). If the honest fix is
  bigger than the sweep, surface it — do not paper it.
- If the fix touches a notify event, a messaging string, or an `McpTool`, mirror it into
  `dravr-contremaitre` in the same change (5 locales; `notify-events.yaml`; `EXPECTED_TOOLS` +
  SDK type regen) — those tests are full-suite-only and will red `main` after the squash.
- **Adding a `testID` to make a screen testable is a legitimate part of the fix.** Several
  screens have no stable anchor (see surfaces.md); add one rather than writing a flow that
  matches on brittle body text.

## Phase 8 — Regression test per defect

**Non-negotiable: one test per confirmed defect.** Write it *before* the fix (or stash the fix),
watch it fail for the reason you filed, then apply the fix and watch it pass. A test that never
failed proves nothing. If the fix is already applied: `git stash`, run red, `git stash pop`, run
green. Record the red output in the run report.

Routing — pick by where the bug actually lives:

| Bug lives in | Destination | Run it with | CI workflow |
|---|---|---|---|
| A UI journey: navigation, gating, a control that does not work on device | `.maestro/<area>/NN-name.yaml` | `maestro test .maestro/<area>/NN-name.yaml` | `mobile-e2e-ios.yml` + `mobile-e2e-android.yml` — **nightly cron + `workflow_dispatch` only** |
| API contract, auth, real data shape — anything reproducible without a simulator | `integration/specs/<area>.integration.test.js` | `bun run e2e:integration` | **none — no workflow runs this suite** |
| Component / hook render logic given known props or a mocked API | `__tests__/<Name>.test.tsx` or `src/<path>/__tests__/` | `bun run test` | `mobile-unit-tests.yml`, every push touching `frontend-mobile/**` or `packages/**` |
| Backend handler logic | **also** `crates/pierre-server/tests/<area>_test.rs` | `cargo test --test <file>` | `ci-backend.yml` / `ci-postgres.yml` |

Deciding rule: **could this bug have been caught without a device?** If yes → jest (unit for
render logic, `integration/` for API contract): fast, and the unit tier is the only one that runs
on every push. If the bug *is* the on-device behaviour — a tap that does nothing, a guard that
does not fire, a keyboard covering a button — it needs a Maestro flow.

**Be honest about what CI will actually run.** Three facts, all verified in the workflow files,
that decide whether your new test guards anything:

- A new Maestro flow in a **new** directory does not run at all until the directory is added to
  `flows:` in `.maestro/config.yaml`.
- Even inside a registered directory, the nightly workflows do not run the whole suite. On a
  branch push or manual dispatch they run a hardcoded ~11-flow *critical* list
  (`BATCH1_FILES` / `BATCH2_FILES` in `mobile-e2e-ios.yml`, `CRITICAL_TESTS` in
  `mobile-e2e-android.yml`); on a push to `main` they run smoke only. **A new flow that is not
  added to those lists never runs in CI.** Add it, or say plainly in the report that the flow is
  local-only.
- `bun run e2e:integration` is referenced by no workflow. A spec you add there is a local gate
  only — run it yourself and say so.

### Template — Maestro flow

```yaml
# ABOUTME: <what this flow covers, one line>
# ABOUTME: <the regression it guards>

appId: com.pierre.fitness

---
- runFlow:
    file: ../helpers/launch-app.yaml
- runFlow:
    file: ../helpers/login.yaml

# Reach the surface the way a user does.
- tapOn:
    id: "tab-settings"
- extendedWaitUntil:
    visible:
      id: "settings-screen"
    timeout: 5000

# Assert CONTENT, not mere presence of the screen.
- assertVisible:
    id: "settings-data-section"
- assertVisible: "Data Providers"
```

Flow rules learned the hard way in this repo — the existing flows encode them, follow them:

- `extendedWaitUntil` with an explicit `timeout`, never `assertVisible` immediately after an
  action and never a `swipe` used as a sleep in new flows (the launch helper does that only
  because Maestro has no sleep and Expo Go needs settling).
- `runFlow: when: visible:` evaluates **once**, instantly — it is not a wait. Pair it with a
  preceding `extendedWaitUntil` if the element can appear late.
- `hideKeyboard` fails in Expo Go v55. Dismiss the keyboard by tapping a non-interactive area
  (`point: "50%,20%"` above the form, `"50%,15%"` in chat) before tapping a button.
- iOS shows a native **Save Password?** sheet after any `secureTextEntry` submit; it blocks the
  whole RN tree. `helpers/dismiss-save-password.yaml` swipes it away.
- Prefer `id:` anchors over text. If the screen has no stable anchor, add the `testID` as part of
  the fix.
- New area directory → register it in `.maestro/config.yaml` `flows:` **and** in the CI critical
  list if it must run in CI.
- Anything you create (a coach, a group) gets a cleanup flow — `coaches/09-cleanup-delete-coach`
  and `coach-wizard/10-cleanup-wizard-coach` are the pattern. The dev DB persists across runs.

### Template — integration spec (`integration/specs/`)

Requires a live 8081 (`globalSetup` hard-fails otherwise, by design). `BACKEND_URL` overrides the
`http://localhost:8081` default. Sequential (`maxWorkers: 1`), plain JS, no RN runtime.

```js
// ABOUTME: <what this spec covers, one line>
// ABOUTME: <the regression it guards>

const { createAndLoginAsAdmin, authenticatedRequest } = require('../helpers');
const { endpoints, timeouts } = require('../fixtures');

describe('<area> integration', () => {
  let accessToken;

  beforeAll(async () => {
    const login = await createAndLoginAsAdmin();
    expect(login.success).toBe(true);
    accessToken = login.accessToken;
  }, timeouts.serverStart);

  it('returns the seeded rows, not an empty list', async () => {
    const result = await authenticatedRequest(endpoints.<endpoint>, accessToken);
    expect(result.status).toBe(200);
    // Assert CONTENT. `success === true` alone passes against a returns-empty stub.
    expect(result.data.<field>).toHaveLength(<N>);
  });
});
```

Existing specs (`auth`, `chat`, `coaches`, `connections`, `social`) lean on shape assertions like
`Array.isArray(...)`. **Do not copy that for a spec written to guard a fix** — an
`Array.isArray` check passes against the empty array that was the bug. Assert the value.

Uniquify anything you create (`` `mobile-e2e-${Date.now()}@test.local` ``) and restore what you
mutate; the dev DB persists across runs.

### Assertion rules (project-enforced)

- **Assert content, never existence.** A real field value, a concrete count, the actual string.
  `expect(res.success).toBe(true)` alone passes against a returns-empty stub — exactly the class
  of bug this sweep exists to catch.
- **Never weaken an assertion** to make a test pass. That is itself a violation.
- **Never** `test.skip` / `.only` / `xit` / commented-out tests / `continue-on-error`, and never
  remove a flow from `config.yaml` to go green.
- Both `ABOUTME:` lines are mandatory on every new file — jest specs use `//`, Maestro flows use
  `#`, above the `appId`.

## Phase 9 — Validate and land

Leave the stack from Phase 1 running; the integration suite and Maestro both need it.

```bash
cd frontend-mobile

bun run typecheck                       # REAL gate on mobile (unlike web's near-noop type-check)
bun run lint                            # NOT run by mobile-unit-tests.yml — local only
bun run test                            # jest unit suite
../scripts/ci/pre-push-mobile-tests.sh  # the mobile tier of pre-push

maestro test .maestro/<area>/NN-name.yaml   # the new flow, targeted
bun run e2e:integration                     # if you added an integration spec (stack must be up)
```

`mobile-unit-tests.yml` runs `typecheck` and `test --coverage` and **does not run lint**, despite
the job name — so `bun run lint` locally is the only lint gate for mobile.

Backend, if touched — always target the file, never bare `cargo test`:

```bash
cargo test --test <file> -- --nocapture
cargo clippy -p <crate> --all-targets --all-features -- -D warnings
```

**Confirm tests actually ran.** jest: a non-zero test count in the summary. `cargo test`:
`running N tests` with N > 0 and `N passed` — exit code 0 is not evidence. Maestro: read the
per-flow result lines, not just the exit code.

Then:

```bash
./scripts/ci/pre-push-validate.sh     # the only local gate; never --no-verify
git add -A && git commit && git push origin main    # fixes are bug fixes → straight to main
```

**Push starts validation, it does not end it.** Watch CI on the pushed commit until every
relevant workflow reaches terminal status (WebFetch the Actions page first; `gh run list`
sparingly; never `gh run watch` or a <60s poll loop). Cancelled ≠ green. Red → fix and re-push in
the same session.

Note what push-time CI will *not* tell you: the e2e workflows are nightly. If your fix is guarded
only by a Maestro flow, the earliest CI signal is the next 06:00 UTC run, or a manual
`workflow_dispatch`. Trigger the dispatch or say in the report that the flow is unverified in CI.

## Phase 10 — Report

Write the run report to `claude_docs/` (the gitignored symlink into the vault's `Claude Outputs/`;
create the symlink if missing) via the `obsidian-writer` skill.

Report contains:

- Stack state: build mode, `EXPO_PUBLIC_API_URL` actually in effect, Expo Go vs dev-client,
  simulator/emulator and OS version, commit SHA, date.
- Coverage: screens visited / total, per role, per platform. **Name anything not visited and
  why.**
- The triage table, with final status per row.
- Every fix: file, one-line rationale, the test that now guards it, and **which CI workflow (if
  any) runs that test**.
- Anything found but **not** fixed, with the reason — never let a known defect vanish.
- CI status on the head commit.

## Teardown

```bash
./bin/stop-all.sh
```

Leave the stack up if ChefFamille is going to look at it — ask.
