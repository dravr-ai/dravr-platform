# Web surface inventory

Every tab the SPA can render, its owning component, and what a **seeded** stack must show.
Source of truth: `frontend/src/components/Dashboard.tsx` (tab registry ~L257-429, render
switch ~L743-915). If this list drifts from that file, the file wins — update this one.

Navigation is hash-based: `#<tab>` and `#<tab>/<sub>`. Role default is `#users` for admins,
`#chat` for users. Prefer clicking the sidebar (it tests the nav); `navigate_page` to the
hash only to recover from a stuck nav — and file the stuck nav.

## What the seed guarantees

Anything below that renders empty **despite** these seeds is a finding, not an empty state:

| Seeded by | Data |
|---|---|
| `user create --super-admin` | `admin@example.com` |
| `seed coaches` | full coach catalogue from `../dravr-contremaitre/prompts/coaches` |
| `seed demo-data --days 30` | demo users incl. `webtest@pierre.dev`, `mobiletest@pierre.dev`, `alice@acme.com`, `bob@startup.io` |
| `seed social` | friendships / social graph |
| `seed mobility` | stretches, yoga content |
| `seed synthetic-activities` | **30 activities over 30 days** for webtest+phil (Strava) and mobiletest+jf (Garmin); alice (Strava), bob (Garmin) — served through the real provider code path by the fixture API on 9555 |
| `seed llm-usage --days 30` | 30 days of LLM usage for the admin |

Cross-check any suspicious emptiness against the API before filing. Take the URL from the
request the page actually made (`list_network_requests` → `get_network_request`) rather than
guessing a path — routes are composed across many router modules and there is no published
path table:

```bash
TOKEN=$(cat logs/admin-token.txt)
curl -s -H "Authorization: Bearer $TOKEN" "http://localhost:8081<captured-path>" | jq '.[0:2]'
```

If the API returns rows and the panel shows none, the bug is in the UI. If the API returns
nothing either, the bug is in the backend (or the seed) — say which in the finding.

---

## Admin surfaces (22)

Logged in as `admin@example.com`. Because that account is `--super-admin`, all 21 admin tabs
**plus** `admin-tokens` render.

### Platform

| # | Tab | id | Component | Must verify |
|---|---|---|---|---|
| 1 | Users | `users` | `UserManagement` | Seeded users listed (webtest, mobiletest, alice, bob, phil_test, jf_test). Pending badge count matches the pending rows. Approve / suspend / role change each round-trip and persist across reload. Search + pagination bounded. |
| 2 | Activity | `activity` | `ActivityTab` | Recent platform activity is non-empty (30 days of seeded activities exist). Timestamps render as dates, never raw epochs or `Invalid Date`. |
| 3 | Engagement | `engagement` | `EngagementTab` | Metrics render numerically, no `NaN`. Its `onNavigate` links land on the right tab. |
| 4 | Notifications | `notifications` | `NotificationsPanel` | Unread badge matches the list. Mark-read persists across reload. Deep links (e.g. a coach reply → `chat/<conversationId>`) open that thread, not a blank chat. |
| — | Billing | `billing` | `BillingTab` | **Gated off** (`BILLING_ENABLED` = `VITE_BILLING_ENABLED === 'true'`). To sweep it, restart Vite with `VITE_BILLING_ENABLED=true`; otherwise record it as *not visited, feature-flagged off*. |

### Coaching

| # | Tab | id | Component | Must verify |
|---|---|---|---|---|
| 5 | Coaches | `coaches` | `SystemCoachesTab` | Coach catalogue non-empty and matching the contremaitre checkout. Each coach opens a detail view with a real persona/prompt, not a placeholder. |
| 6 | Coach Store | `coach-store` | `CoachStoreManagement` | Listings render; pending-moderation badge matches the queue. Approve/reject changes state and survives reload. |
| 7 | Groups | `groups` | `GroupManagement` → `GroupDetail` | Create a group, open it (`#groups/<id>`), reload — the detail view restores. Invite link generates. Member list correct. Back returns to the list, not to login. |

### Configuration

| # | Tab | id | Component | Must verify |
|---|---|---|---|---|
| 8 | Tool Management | `configuration` | `AdminConfiguration` | Tool list non-empty. Enable/disable persists. Count is consistent with the server's registry. |
| 9 | User Tools | `user-tools` | `UserToolOverrides` | Per-user overrides load for a picked user and save. |
| 10 | Prompts | `prompts` | `SystemPromptsTab` | Prompts render with real content; edits save and reload correctly. |
| 11 | Platform Settings | `platform-settings` | `AdminSettings` | Settings load current values (not blank defaults masquerading as saved state). Save round-trips. **No secret is echoed into the DOM** — check the snapshot for token/key values. |
| 12 | Claim Verdicts | `claim-verdicts` | `ClaimVerdictsTab` | Loads without error; empty state is explicit ("no verdicts"), never a silent blank. |
| 13 | Harness Config | `harness-config` | `HarnessConfigTab` | Config loads and saves. |
| 14 | Memory Worker | `memory-worker` | `MemoryExtractionMonitorTab` | Worker status renders a real state, not a hardcoded "healthy". |
| 15 | Coach Followups | `coach-followups` | `CoachFollowupsTab` | Loads; list or explicit empty state. |
| 16 | Coach Notes Audit | `coach-notes-audit` | `CoachNotesAuditTab` | Loads; audit rows or explicit empty state. |
| 17 | Myth Busting | `myth-busting` | `MythBustingTab` | Loads; content renders. |
| 18 | Coach Grades | `coach-grading` | `CoachGradingTab` | Loads; grades render numerically. |
| 19 | Eval Harness | `eval-harness` | `EvalHarnessTab` | Loads. Do **not** launch a long eval run during a sweep unless ChefFamille asks — note it as load-only coverage. |

### Developer

| # | Tab | id | Component | Must verify |
|---|---|---|---|---|
| 20 | Service Tokens | `connections` | `UnifiedConnections` | Token list loads. Create → the secret is shown **once**; revoke removes it. Ownership is `tenant_id`-scoped. |
| 21 | Analytics | `analytics` | `UsageAnalytics` + `LlmConsumptionPanel` + `ToolUsagePanel` | **All three panels non-empty** — `seed llm-usage` provisioned 30 days. An all-zero LLM panel here is the classic silent-stub signature; cross-check the API before dismissing it. Charts render, no `NaN` axes. |

### Super admin

| # | Tab | id | Component | Must verify |
|---|---|---|---|---|
| 22 | Admin Tokens | `admin-tokens` | list → `ApiKeyDetails` | Tokens list. Detail view opens and backs out. Create requires super-admin; the raw token is shown once and never re-fetchable. Revoke works. |

---

## User surfaces (7 + Settings)

Logged in as `webtest@pierre.dev`. Onboarding runs first on a fresh profile — see SKILL.md
Phase 4.

| # | Tab | id | Component | Must verify |
|---|---|---|---|---|
| 1 | Chat | `chat` | `ChatTab` | Conversation list loads. Send a message → a coach replies. **The coach must answer in its own persona** — a reply identifying as the underlying LLM/CLI provider is a P0 identity leak. Deep link `#chat/<id>` restores the thread on reload. Streaming does not duplicate or truncate messages. |
| 2 | Coaches | `my-coaches` | `CoachLibraryTab` | The user's coaches render. `ConnectProviderBanner` shows only when no provider is connected — webtest **has** Strava, so a banner here is a finding. |
| 3 | Discover | `discover` | `StoreScreen` | Store listings non-empty. Install/add a coach → it appears under Coaches. Same banner rule as above. |
| 4 | Data Providers | `data-providers` | `UserSettings initialTab="connections" hideTabNav` | Strava shows **connected** for webtest. Disconnect must clear *both* `provider_connections` and `oauth_tokens` — a "connected but disconnected" drift is a known recurring class. Reconnect works. Role-gated: `!isAdminUser`, so an admin hand-typing `#data-providers` must get nothing. |
| 5 | Groups | `groups` | `GroupManagement` → `GroupDetail` | Join via invite code. `#groups/<id>` deep link + reload restores. Leave works. |
| 6 | Insights | `insights` | `SocialFeedTab` ⇄ `FriendsTab` | Feed non-empty (`seed social` + 30 activities). `#insights/friends` renders the friends sub-view and back returns to the feed. Friend request → accept round-trips. |
| 7 | Notifications | `notifications` | `NotificationsPanel` | Same checks as admin. |
| — | Usage | `usage` | `BillingPage` | **Gated off** with billing. Record as not visited / flagged off unless run with `VITE_BILLING_ENABLED=true`. |
| S | Settings | `settings` | `UserSettings` | Reached via the gear icon, **not** the sidebar. Every sub-tab opens. Profile edit saves and survives reload. Theme toggle persists (`dravr.theme`). Privacy & Data → analytics consent toggle actually boots/shuts PostHog (watch the network). Password change round-trips. |

---

## Auth & shell surfaces (both roles)

| Surface | Reached by | Must verify |
|---|---|---|
| Login | `/` logged out | Wrong password → a clear error, no stack trace, no internal detail. Correct → dashboard. |
| Register | Login → "register" | New account created. `@example.com` is **not** auto-approved (only `dravr.ai` is) → lands on Pending Approval. |
| Forgot / Reset password | Login → "forgot password" | Code-send + reset path renders and errors legibly. |
| Pending Approval | register a fresh `@example.com` user | Gated screen renders; admin approval flips it to active. |
| Suspended | admin suspends a user, that user logs in | "Account Suspended" screen, no dashboard access. |
| OAuth callback | `/?provider=strava&success=true` | Result screen renders and closes cleanly back to the app. |
| Impersonation banner | admin impersonates a user | Banner visible for the whole session; exit restores the admin. |
| Group invite link | `/groups/join/<code>` | Consumes the code, cleans the URL, lands in the group. |
| Error boundary | force a component throw via `evaluate_script` | A boundary screen, not a white page. |

## Known-fragile areas — look harder here

Recurring defect classes in this codebase; give them extra scrutiny:

- **Coach identity leak** — a coach replying as the provider CLI instead of its persona.
  Surfaces at larger prompt sizes / longer conversations, not on turn 1.
- **Provider connection drift** — connected in one table, disconnected in the other.
- **All-zero analytics panels** — data exists server-side, UI renders zeros.
- **Tenant scoping** — any list that could show another tenant's rows.
- **Onboarding hard-gates** — a user with a seeded provider being forced back to connect.
